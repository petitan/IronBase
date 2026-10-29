// Search operations for CollectionCore
// Fuzzy text search and full-text search with TF-IDF

use serde_json::Value;
use std::collections::{HashMap, HashSet};

use crate::document::Document;
use crate::error::{IronBaseError, Result};
use crate::execution::ExecutionContext;
use crate::fulltext::{
    build_phrase_regex_cached, generate_highlights, parse_query, tokenize_unique,
    FulltextIndexMetadata, FulltextSearchOptions, FulltextSearchResultExt,
};
use crate::index::{FuzzyAlgorithm, FuzzySearchOptions, FuzzySearchResult};
use crate::log_error;
use crate::query::Query;
use crate::storage::{RawStorage, Storage};
use crate::value_utils::get_nested_value;

use super::index_persistence::{build_fulltext_index_file_path, build_fuzzy_index_file_path};
use super::topk::topk_select_with_skip;
use super::CollectionCore;

/// Search operations for CollectionCore
impl<S: Storage + RawStorage> CollectionCore<S> {
    /// Create a fuzzy text index on a field
    ///
    /// Fuzzy indexes enable similarity-based text search using algorithms like
    /// Jaro-Winkler, Levenshtein, or Damerau-Levenshtein.
    ///
    /// # Arguments
    /// * `field` - Field to index
    /// * `algorithm` - Similarity algorithm (default: JaroWinkler)
    /// * `threshold` - Minimum similarity threshold 0.0-1.0 (default: 0.8)
    ///
    /// # Example
    /// ```rust,ignore
    /// collection.create_fuzzy_index("name", FuzzyAlgorithm::JaroWinkler, 0.8)?;
    /// ```
    pub fn create_fuzzy_index(
        &self,
        field: String,
        algorithm: FuzzyAlgorithm,
        threshold: f64,
    ) -> Result<String> {
        self.check_not_closed()?;
        let index_name = format!("{}_{}_fuzzy", self.name, field);

        // Disk storage like create_fulltext_index: without a path the
        // checkpoint never wrote a .fzidx, so every reopen after a crash
        // rebuilt the index from the documents
        let storage_path = {
            let storage = self.storage.read();
            build_fuzzy_index_file_path(storage.get_file_path(), &index_name)
        };

        // Step 1: Create the fuzzy index in IndexManager
        {
            let mut indexes = self.indexes.write();
            indexes.create_fuzzy_index_with_storage(
                index_name.clone(),
                field.clone(),
                algorithm,
                threshold,
                storage_path,
            )?;
            // Writers maintain the index while the scan below fills it (T2)
            if let Some(index) = indexes.get_fuzzy_index_mut(&index_name) {
                index.begin_build();
            }
        }

        // Step 2 & 3: Scan documents IN BATCHES and populate index
        // This is memory-efficient for large collections
        use crate::limits::FULLTEXT_INDEX_BUILD_BATCH_SIZE as INDEX_BUILD_BATCH_SIZE;

        let field_clone = field.clone();
        let index_name_clone = index_name.clone();
        let indexes_ref = self.indexes.clone();

        let scan =
            self.scan_documents_in_batches(INDEX_BUILD_BATCH_SIZE, |_batch_num, batch_docs| {
                let mut indexes = indexes_ref.write();
                if let Some(index) = indexes.get_fuzzy_index_mut(&index_name_clone) {
                    for (doc_id, doc) in &batch_docs {
                        if let Some(value) = get_nested_value(doc, &field_clone) {
                            if let Some(s) = value.as_str() {
                                index.build_insert(s, doc_id.clone());
                            }
                        }
                    }
                }
                Ok(())
            });
        if let Err(e) = scan {
            // A failed build leaves no half-populated index behind
            let _ = self.indexes.write().drop_index(&index_name);
            return Err(e);
        }

        // Mark index as ready after population is complete
        {
            let mut indexes = self.indexes.write();
            if let Some(index) = indexes.get_fuzzy_index_mut(&index_name) {
                index.end_build();
            }
            if let Err(e) = indexes.set_index_ready(&index_name) {
                tracing::warn!(error = %e, "Failed to mark fuzzy index as ready");
            }
        }

        // Step 4: Get metadata for persistence
        let metadata = {
            let indexes = self.indexes.read();
            indexes
                .get_fuzzy_index(&index_name)
                .map(|idx| idx.metadata.clone())
        };

        // Step 4: Persist metadata to storage (if index was created successfully)
        if let Some(metadata) = metadata {
            let mut storage = self.storage.write();
            if let Some(collection_meta) = storage.get_collection_meta_mut(&self.name) {
                // Avoid duplicates
                if !collection_meta
                    .fuzzy_indexes
                    .iter()
                    .any(|m| m.name == metadata.name)
                {
                    collection_meta.fuzzy_indexes.push(metadata);
                }
            }
            storage.persist_metadata()?;
        }

        Ok(index_name)
    }

    /// Search using a fuzzy index
    ///
    /// Returns documents where the indexed field is similar to the query string.
    /// Results are sorted by similarity (highest first).
    ///
    /// # Arguments
    /// * `field` - Field to search (must have a fuzzy index)
    /// * `query` - Search string
    /// * `threshold` - Optional threshold override
    /// * `algorithm` - Optional algorithm override
    ///
    /// # Returns
    /// Vector of (document, similarity_score) pairs
    pub fn fuzzy_search(
        &self,
        field: &str,
        query: &str,
        threshold: Option<f64>,
        algorithm: Option<FuzzyAlgorithm>,
    ) -> Result<Vec<(Value, f64)>> {
        // Batch mode: see the inserts acknowledged before this call
        self.flush_pending_writes()?;
        self.check_not_closed()?;
        let indexes = self.indexes.read();

        // Find fuzzy index for this field
        let fuzzy_index = indexes.get_fuzzy_index_for_field(field).ok_or_else(|| {
            IronBaseError::IndexError(format!("No fuzzy index found for field '{}'", field))
        })?;

        // Perform search
        let matches = if let Some(algo) = algorithm {
            fuzzy_index.search_with_algorithm(query, algo, threshold.unwrap_or(0.8))
        } else {
            fuzzy_index.search(query, threshold)
        };

        drop(indexes);

        // Fetch full documents for matched IDs
        let mut results = Vec::with_capacity(matches.len());
        for (doc_id, similarity) in matches {
            if let Some(doc) = self.read_document_by_id(&doc_id)? {
                results.push((doc, similarity));
            }
        }

        Ok(results)
    }

    /// Fuzzy search with execution context for cancellation support.
    ///
    /// This is the cancellation-aware version of `fuzzy_search`.
    /// Pass an ExecutionContext to enable timeout/cancellation checking.
    ///
    /// # Arguments
    /// * `field` - Field to search (must have a fuzzy index)
    /// * `query` - Search query string
    /// * `threshold` - Optional similarity threshold override (0.0-1.0)
    /// * `algorithm` - Optional algorithm override
    /// * `limit` - Optional limit for top N results
    /// * `ctx` - Optional execution context for cancellation
    pub fn fuzzy_search_with_ctx(
        &self,
        field: &str,
        query: &str,
        threshold: Option<f64>,
        algorithm: Option<FuzzyAlgorithm>,
        limit: Option<usize>,
        ctx: Option<&ExecutionContext>,
    ) -> Result<Vec<(Value, f64)>> {
        // Batch mode: see the inserts acknowledged before this call
        self.flush_pending_writes()?;
        self.check_not_closed()?;
        let indexes = self.indexes.read();

        // Find fuzzy index for this field
        let fuzzy_index = indexes.get_fuzzy_index_for_field(field).ok_or_else(|| {
            IronBaseError::IndexError(format!("No fuzzy index found for field '{}'", field))
        })?;

        // Perform search WITH cancellation support and limit (the expensive part!)
        let matches = fuzzy_index.search_with_ctx(query, threshold, algorithm, limit, ctx)?;

        drop(indexes);

        // Fetch full documents for matched IDs with cancellation checks
        let mut results = Vec::with_capacity(matches.len());
        for (iteration, (doc_id, similarity)) in matches.into_iter().enumerate() {
            // Check for cancellation every N iterations
            if let Some(exec_ctx) = ctx {
                exec_ctx.maybe_check(iteration)?;
            }

            if let Some(doc) = self.read_document_by_id(&doc_id)? {
                results.push((doc, similarity));
            }
        }

        Ok(results)
    }

    /// Extended fuzzy search with filter + projection support (CORE LEVEL)
    ///
    /// This is the recommended unified API for fuzzy search (consistent with fulltext_search_ext).
    /// All logic (similarity matching, filter, projection) is handled in core.
    ///
    /// # Arguments
    /// * `field` - Field with fuzzy index
    /// * `query` - Search query string (the value to match)
    /// * `options` - `FuzzySearchOptions` with all search parameters
    ///
    /// # Returns
    /// Vector of `FuzzySearchResult` with document, score, matched value, and optional highlight
    ///
    /// # Example
    /// ```rust,ignore
    /// use ironbase_core::FuzzySearchOptions;
    /// use serde_json::json;
    ///
    /// let options = FuzzySearchOptions::new()
    ///     .with_limit(10)
    ///     .with_threshold(0.75)
    ///     .with_filter(json!({"status": "active"}))
    ///     .with_projection(HashMap::from([("name".to_string(), 1)]));
    ///
    /// let results = collection.fuzzy_search_ext("name", "Jhon", options)?;
    /// for result in results {
    ///     println!("Score: {:.2}, Matched: {}, Doc: {:?}",
    ///              result.score, result.matched_value, result.document);
    /// }
    /// ```
    pub fn fuzzy_search_ext(
        &self,
        field: &str,
        query: &str,
        options: FuzzySearchOptions,
    ) -> Result<Vec<FuzzySearchResult>> {
        // Batch mode: see the inserts acknowledged before this call
        self.flush_pending_writes()?;
        self.check_not_closed()?;

        // Build execution context from options
        let ctx = if options.cancel_flag.is_some() || options.deadline.is_some() {
            Some(
                ExecutionContext::new()
                    .with_deadline_opt(options.deadline)
                    .with_cancel_flag_opt(options.cancel_flag.clone()),
            )
        } else {
            None
        };

        let effective_limit = options.limit.unwrap_or(10);
        let effective_skip = options.skip.unwrap_or(0);

        let indexes = self.indexes.read();

        // Find fuzzy index for this field
        let fuzzy_index = indexes.get_fuzzy_index_for_field(field).ok_or_else(|| {
            IronBaseError::IndexError(format!("No fuzzy index found for field '{}'", field))
        })?;

        // Parse filter if provided
        let parsed_filter = if let Some(ref filter) = options.filter {
            Some(Query::from_json(filter)?)
        } else {
            None
        };

        // Determine candidate limit: the skipped matches are candidates too.
        // With a filter, request more candidates
        let page_end = effective_skip.saturating_add(effective_limit);
        let candidate_limit = if parsed_filter.is_some() {
            page_end.saturating_mul(10).max(100)
        } else {
            page_end
        };

        // Perform fuzzy search with cancellation support
        let matches = fuzzy_index.search_with_ctx(
            query,
            options.threshold,
            options.algorithm,
            Some(candidate_limit),
            ctx.as_ref(),
        )?;

        drop(indexes);

        // Process results: load doc, filter, project, highlight
        let mut results = Vec::new();
        let mut skipped = 0;

        for (iteration, (doc_id, similarity)) in matches.into_iter().enumerate() {
            // Cancellation check
            if let Some(ref exec_ctx) = ctx {
                exec_ctx.maybe_check(iteration)?;
            }

            // Load document
            let doc = match self.read_document_by_id(&doc_id)? {
                Some(d) => d,
                None => continue, // Deleted doc, skip
            };

            // Apply MongoDB filter (if provided)
            if let Some(ref filter) = parsed_filter {
                match Document::from_value(&doc) {
                    Ok(document) => {
                        if !filter.matches(&document)? {
                            continue; // Doesn't match filter, skip
                        }
                    }
                    Err(_) => continue, // Invalid document, skip
                }
            }

            // Skip the matches (after the filter) before the page
            if skipped < effective_skip {
                skipped += 1;
                continue;
            }

            // Get matched value for the result
            let matched_value = get_nested_value(&doc, field)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            // Apply projection
            let projected_doc = if let Some(ref proj) = options.projection {
                crate::find_options::apply_projection(&doc, proj)?
            } else {
                doc
            };

            // Generate highlight if enabled
            let highlight = if options.highlight && !matched_value.is_empty() {
                // Simple highlight: wrap the matched portion in <mark> tags
                // For fuzzy matching, we highlight the entire matched value
                Some(format!("<mark>{}</mark>", matched_value))
            } else {
                None
            };

            // Build result
            results.push(FuzzySearchResult {
                document: projected_doc,
                score: similarity,
                matched_value,
                highlight,
            });

            // Early exit when limit reached
            if results.len() >= effective_limit {
                break;
            }
        }

        Ok(results)
    }

    /// Create a full-text search index with language support
    ///
    /// # Arguments
    /// * `field` - Field to index (supports dot notation for nested fields)
    /// * `language` - Language for stemming and stop words ("hungarian", "english", "german", "none")
    /// * `min_word_length` - Minimum word length to index (default: 2)
    /// * `accent_folding` - Whether to apply accent folding (default: true)
    ///
    /// # Returns
    /// The name of the created index (format: `{collection}_{field}_fts`)
    ///
    /// # Example
    /// ```rust,ignore
    /// collection.create_fulltext_index(
    ///     "content".to_string(),
    ///     "hungarian",
    ///     None,
    ///     None,
    /// )?;
    /// ```
    pub fn create_fulltext_index(
        &self,
        field: String,
        language: &str,
        min_word_length: Option<usize>,
        accent_folding: Option<bool>,
    ) -> Result<String> {
        self.check_not_closed()?;
        use crate::fulltext::FtsLanguage;

        let index_name = format!("{}_{}_fts", self.name, field);
        let lang = FtsLanguage::from_str(language);

        // Get db_path for .ftidx file storage
        let storage_path = {
            let storage = self.storage.read();
            let db_path = storage.get_file_path().to_string();
            build_fulltext_index_file_path(&db_path, &index_name)
        };

        // Step 1: Create the fulltext index with disk storage
        {
            let mut indexes = self.indexes.write();
            indexes.create_fulltext_index_with_storage(
                index_name.clone(),
                field.clone(),
                lang,
                min_word_length,
                accent_folding,
                storage_path.clone(),
            )?;
            // Writers maintain the index while the scan below fills it (T2)
            if let Some(index) = indexes.get_fulltext_index_mut(&index_name) {
                index.begin_build();
            }
        }

        // Helper closure for cleanup on failure
        let cleanup = |index_name: &str, storage_path: &Option<std::path::PathBuf>| {
            // Remove index from IndexManager
            let mut indexes = self.indexes.write();
            let _ = indexes.drop_fulltext_index(index_name);
            // Delete partial .ftidx file if it exists
            if let Some(path) = storage_path {
                let _ = std::fs::remove_file(path);
            }
        };

        // Step 2 & 3: Scan documents IN BATCHES and populate index
        // This is memory-efficient for large collections (e.g., 10GB+ email databases)
        // Each batch is processed and then dropped before loading the next batch
        use crate::limits::FULLTEXT_INDEX_BUILD_BATCH_SIZE as INDEX_BUILD_BATCH_SIZE;

        let field_clone = field.clone();
        let index_name_clone = index_name.clone();
        let indexes_ref = self.indexes.clone();

        if let Err(e) =
            self.scan_documents_in_batches(INDEX_BUILD_BATCH_SIZE, |_batch_num, batch_docs| {
                let mut indexes = indexes_ref.write();
                if let Some(index) = indexes.get_fulltext_index_mut(&index_name_clone) {
                    for (doc_id, doc) in &batch_docs {
                        if let Some(value) = get_nested_value(doc, &field_clone) {
                            if let Some(s) = value.as_str() {
                                let pdid_field = index.parent_doc_id_field().to_string();
                                let parent_doc_id = get_nested_value(doc, &pdid_field)
                                    .and_then(|v| v.as_str().map(|x| x.to_string()));
                                let insert_result =
                                    index.build_insert(doc_id, s, parent_doc_id.as_deref());
                                if let Err(e) = insert_result {
                                    log_error!(
                                        "Failed to index doc {:?} in fulltext index '{}': {:?}",
                                        doc_id,
                                        index_name_clone,
                                        e
                                    );
                                }
                            }
                        }
                    }
                }
                Ok(())
            })
        {
            cleanup(&index_name, &storage_path);
            return Err(e);
        }

        // Step 4: Mark index as ready and flush to disk
        let metadata = {
            let mut indexes = self.indexes.write();
            if let Some(index) = indexes.get_fulltext_index_mut(&index_name) {
                index.end_build();
            }
            // Mark index as ready BEFORE persisting
            if let Err(e) = indexes.set_index_ready(&index_name) {
                tracing::warn!(error = %e, "Failed to mark fulltext index as ready");
            }
            if let Some(index) = indexes.get_fulltext_index_mut(&index_name) {
                // Flush the fulltext index to persist inverted index data to .ftidx file
                if let Err(e) = index.save_to_file() {
                    drop(indexes); // Release lock before cleanup
                    cleanup(&index_name, &storage_path);
                    return Err(e);
                }
            }
            indexes
                .get_fulltext_index(&index_name)
                .map(|i| i.metadata())
        };

        // Step 5: Validate index has content before saving metadata
        // This prevents ghost indexes from being registered
        // Exception: if the collection itself is empty, an empty index is valid
        // (e.g., rag_collection_create on a new collection)
        if let Some(ref meta) = metadata {
            if meta.num_documents == 0 {
                let collection_is_empty = {
                    let storage = self.storage.read();
                    storage
                        .get_collection_meta(&self.name)
                        .map(|m| m.live_document_count == 0)
                        .unwrap_or(true)
                };
                if !collection_is_empty {
                    // Non-empty collection but empty index - field likely doesn't exist
                    cleanup(&index_name, &storage_path);
                    return Err(crate::error::IronBaseError::IndexError(format!(
                        "Fulltext index '{}' has no documents - field '{}' may not exist or contain text",
                        index_name, field
                    )));
                }
                // Empty collection → empty index is valid, proceed with registration
            }
        }

        // Step 6: Store fulltext index metadata in storage
        if let Some(meta) = metadata {
            let mut storage = self.storage.write();
            if let Some(coll_meta) = storage.get_collection_meta_mut(&self.name) {
                coll_meta.fulltext_indexes.push(meta);
            }
            // CRITICAL FIX: Flush metadata to persist index metadata to disk
            // Without this, crash before next checkpoint would lose the index (bug found 2024-12-26)
            storage.persist_metadata()?;
        }

        Ok(index_name)
    }

    /// Search documents using a full-text index with TF-IDF scoring
    ///
    /// # Arguments
    /// * `field` - Field with fulltext index
    /// * `query` - Search query text
    /// * `limit` - Maximum number of results (default: 10)
    /// * `skip` - Number of results to skip (default: 0)
    /// * `min_score` - Minimum TF-IDF score threshold (default: None)
    /// * `projection` - Optional projection to include/exclude fields (default: None = all fields)
    ///
    /// # Returns
    /// Vector of tuples (document, score, matched_tokens)
    ///
    /// # Example
    /// ```rust,ignore
    /// use std::collections::HashMap;
    ///
    /// // Search with projection (exclude full_text field)
    /// let mut projection = HashMap::new();
    /// projection.insert("full_text".to_string(), 0);
    ///
    /// let results = collection.fulltext_search(
    ///     "content",
    ///     "rust programming",
    ///     Some(10),
    ///     None,
    ///     None,
    ///     Some(projection),
    /// )?;
    /// for (doc, score, tokens) in results {
    ///     println!("Score: {:.2}, Tokens: {:?}", score, tokens);
    /// }
    /// ```
    pub fn fulltext_search(
        &self,
        field: &str,
        query: &str,
        limit: Option<usize>,
        skip: Option<usize>,
        min_score: Option<f64>,
        projection: Option<HashMap<String, i32>>,
    ) -> Result<Vec<(Value, f64, Vec<String>)>> {
        // Batch mode: see the inserts acknowledged before this call
        self.flush_pending_writes()?;
        self.check_not_closed()?;
        let indexes = self.indexes.read();

        // Find fulltext index for this field
        let fulltext_index = indexes.get_fulltext_index_for_field(field).ok_or_else(|| {
            IronBaseError::IndexError(format!("No fulltext index found for field '{}'", field))
        })?;

        // Perform search
        let search_results =
            fulltext_index.search(query, limit.unwrap_or(10), skip.unwrap_or(0), min_score);

        drop(indexes);

        // Fetch documents for matched IDs and apply projection if specified
        let mut results = Vec::with_capacity(search_results.len());
        for result in search_results {
            if let Some(doc) = self.read_document_by_id(&result.doc_id)? {
                let projected_doc = if let Some(ref proj) = projection {
                    crate::find_options::apply_projection(&doc, proj)?
                } else {
                    doc
                };
                results.push((projected_doc, result.score, result.matched_tokens));
            }
        }

        Ok(results)
    }

    /// Extended fulltext search with filter + highlight support (CORE LEVEL)
    ///
    /// This is the recommended unified API for fulltext search.
    /// All logic (TF-IDF, filter, highlight) is handled in core.
    ///
    /// # Arguments
    /// * `field` - Field with fulltext index
    /// * `query` - Search query text (supports phrase search with "quotes")
    /// * `options` - `FulltextSearchOptions` with all search parameters
    ///
    /// # Returns
    /// Vector of `FulltextSearchResultExt` with document, score, matched tokens, and optional highlights
    ///
    /// # Example
    /// ```rust,ignore
    /// use ironbase_core::fulltext::FulltextSearchOptions;
    /// use serde_json::json;
    ///
    /// let options = FulltextSearchOptions::new()
    ///     .with_limit(10)
    ///     .with_filter(json!({"from.email": {"$regex": "@scania.com$"}}))
    ///     .with_highlight(100, 3);
    ///
    /// let results = collection.fulltext_search_ext("subject", "mérőcella", options)?;
    /// for result in results {
    ///     println!("Score: {:.2}, Doc: {:?}", result.score, result.document);
    ///     if let Some(highlights) = result.highlights {
    ///         for h in highlights {
    ///             for snippet in h.snippets {
    ///                 println!("  Highlight: {}", snippet);
    ///             }
    ///         }
    ///     }
    /// }
    /// ```
    pub fn fulltext_search_ext(
        &self,
        field: &str,
        query: &str,
        options: FulltextSearchOptions,
    ) -> Result<Vec<FulltextSearchResultExt>> {
        // Batch mode: see the inserts acknowledged before this call
        self.flush_pending_writes()?;
        self.check_not_closed()?;

        // Build execution context from options
        let ctx = if options.cancel_flag.is_some() || options.deadline.is_some() {
            Some(
                ExecutionContext::new()
                    .with_deadline_opt(options.deadline)
                    .with_cancel_flag_opt(options.cancel_flag.clone()),
            )
        } else {
            None
        };

        // Parse query for phrases (MongoDB-compatible syntax)
        let parsed = parse_query(query);
        let effective_limit = options.limit.unwrap_or(10);
        let effective_skip = options.skip.unwrap_or(0);

        let indexes = self.indexes.read();

        // Find fulltext index for this field
        let fulltext_index = indexes.get_fulltext_index_for_field(field).ok_or_else(|| {
            IronBaseError::IndexError(format!("No fulltext index found for field '{}'", field))
        })?;

        // Store FTS options for tokenization and highlights
        let fts_options = fulltext_index.options.clone();

        // Parse filter if provided
        let parsed_filter = if let Some(ref filter) = options.filter {
            Some(Query::from_json(filter)?)
        } else {
            None
        };

        // Prepare highlight tokenization if enabled
        let query_tokens = if options.highlight {
            tokenize_unique(query, &fts_options)
        } else {
            Vec::new()
        };

        let highlight_opts = options.highlight_options.clone().unwrap_or_default();

        // AND mode: tokenize query to know required token count
        let and_mode = options.and_mode;
        let required_token_count = if and_mode {
            tokenize_unique(query, &fts_options).len()
        } else {
            0
        };

        // Determine candidate limit - if we have a filter, request more candidates
        let has_filter_or_phrase_or_and =
            parsed_filter.is_some() || parsed.has_phrases() || and_mode;
        let candidate_limit = FulltextSearchOptions::calculate_candidate_limit(
            effective_limit,
            has_filter_or_phrase_or_and,
        );

        // Perform TF-IDF search
        let search_results = if let Some(ref target_ids) = options.target_doc_ids {
            // Pre-filtered path: only score chunks belonging to target doc_ids
            fulltext_index.search_for_doc_ids_with_ctx(
                query,
                target_ids,
                candidate_limit,
                if parsed_filter.is_none() {
                    effective_skip
                } else {
                    0
                },
                options.min_score,
                ctx.as_ref(),
            )?
        } else if !parsed.has_phrases() && and_mode {
            // AND-optimized path: intersect posting lists before scoring
            fulltext_index.search_and_with_ctx(
                query,
                candidate_limit,
                if parsed_filter.is_none() {
                    effective_skip
                } else {
                    0
                },
                options.min_score,
                ctx.as_ref(),
            )?
        } else if !parsed.has_phrases() {
            // Fast path: no phrases, OR mode
            fulltext_index.search_with_ctx(
                query,
                candidate_limit,
                if parsed_filter.is_none() {
                    effective_skip
                } else {
                    0
                }, // Skip in TF-IDF only if no filter
                options.min_score,
                ctx.as_ref(),
            )?
        } else {
            // Phrase search: use all terms
            let all_terms = parsed.all_search_terms(&fts_options);
            fulltext_index.search_with_ctx(
                &all_terms.join(" "),
                candidate_limit,
                0, // Skip handled manually after phrase filtering
                options.min_score,
                ctx.as_ref(),
            )?
        };

        // Build phrase regexes if needed
        let phrase_regexes: Vec<_> = if parsed.has_phrases() {
            parsed
                .phrases
                .iter()
                .map(|p| build_phrase_regex_cached(p))
                .collect::<Result<Vec<_>>>()?
        } else {
            Vec::new()
        };

        drop(indexes);

        // Process results: load doc, filter, project, highlight
        let mut results = Vec::new();
        let mut skipped = 0;
        let parse_skip = projection_parse_skip(
            options.projection.as_ref(),
            &[field],
            options.filter.as_ref(),
        );

        for (iteration, fts_result) in search_results.into_iter().enumerate() {
            // Cancellation check
            if let Some(ref exec_ctx) = ctx {
                exec_ctx.maybe_check(iteration)?;
            }

            // Load document
            let doc = match self.read_document_by_id_skipping(&fts_result.doc_id, &parse_skip)? {
                Some(d) => d,
                None => continue, // Deleted doc, skip
            };

            // AND mode: all query tokens must match
            if and_mode
                && required_token_count > 0
                && fts_result.matched_tokens.len() < required_token_count
            {
                continue;
            }

            // Phrase check (if applicable)
            if !phrase_regexes.is_empty() {
                // Supports dot notation for nested fields (e.g. "address.city")
                let text = get_nested_value(&doc, field)
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let all_phrases_match = phrase_regexes.iter().all(|re| re.is_match(text));
                if !all_phrases_match {
                    continue;
                }
            }

            // Apply MongoDB filter (if provided)
            if let Some(ref filter) = parsed_filter {
                match Document::from_value(&doc) {
                    Ok(document) => {
                        if !filter.matches(&document)? {
                            continue; // Doesn't match filter, skip
                        }
                    }
                    Err(_) => continue, // Invalid document, skip
                }
            }

            // Handle skip (manual skip when filter/phrase/and was used)
            if has_filter_or_phrase_or_and && skipped < effective_skip {
                skipped += 1;
                continue;
            }

            // Apply projection
            let projected_doc = if let Some(ref proj) = options.projection {
                crate::find_options::apply_projection(&doc, proj)?
            } else {
                doc.clone()
            };

            // Generate highlights if enabled
            let highlights = if options.highlight {
                let field_value = get_nested_value(&doc, field)
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                if !field_value.is_empty() && !query_tokens.is_empty() {
                    let result = generate_highlights(
                        field_value,
                        &query_tokens,
                        field,
                        &fts_options,
                        &highlight_opts,
                    );

                    if !result.snippets.is_empty() {
                        Some(vec![result])
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            };

            // Build result
            results.push(FulltextSearchResultExt {
                document: projected_doc,
                score: fts_result.score,
                matched_tokens: fts_result.matched_tokens,
                highlights,
            });

            // Early exit when limit reached
            if results.len() >= effective_limit {
                break;
            }
        }

        Ok(results)
    }

    /// Multi-field fulltext search with score merging
    ///
    /// Searches across multiple fulltext-indexed fields and merges results by document.
    /// Each document's final score is the **maximum** score across all matching fields
    /// (similar to Elasticsearch `best_fields` strategy).
    ///
    /// # Arguments
    /// * `fields` - Fields to search (each must have a fulltext index)
    /// * `query` - Search query (supports phrases, AND mode, etc.)
    /// * `options` - Search options (limit, skip, filter, highlight, etc.)
    ///
    /// # Returns
    /// Merged results sorted by best score. `matched_tokens` is the union across all fields.
    /// Highlights include snippets from all matching fields.
    ///
    /// # Errors
    /// Returns error if any field has no fulltext index.
    pub fn fulltext_search_multi_ext(
        &self,
        fields: &[&str],
        query: &str,
        options: FulltextSearchOptions,
    ) -> Result<Vec<FulltextSearchResultExt>> {
        // Batch mode: see the inserts acknowledged before this call
        self.flush_pending_writes()?;
        self.check_not_closed()?;

        // Single field → delegate to existing method
        if fields.len() == 1 {
            return self.fulltext_search_ext(fields[0], query, options);
        }

        if fields.is_empty() {
            return Err(IronBaseError::IndexError(
                "No fields specified for multi-field fulltext search".to_string(),
            ));
        }

        // Build execution context from options
        let ctx = if options.cancel_flag.is_some() || options.deadline.is_some() {
            Some(
                ExecutionContext::new()
                    .with_deadline_opt(options.deadline)
                    .with_cancel_flag_opt(options.cancel_flag.clone()),
            )
        } else {
            None
        };

        let parsed = parse_query(query);
        let effective_limit = options.limit.unwrap_or(10);
        let effective_skip = options.skip.unwrap_or(0);
        let and_mode = options.and_mode;

        // Parse filter once
        let parsed_filter = if let Some(ref filter) = options.filter {
            Some(Query::from_json(filter)?)
        } else {
            None
        };

        let highlight_opts = options.highlight_options.clone().unwrap_or_default();

        // Determine candidate limit with overallocation
        let has_filter_or_phrase_or_and =
            parsed_filter.is_some() || parsed.has_phrases() || and_mode;
        let candidate_limit = FulltextSearchOptions::calculate_candidate_limit(
            effective_limit,
            has_filter_or_phrase_or_and,
        );

        // Build phrase regexes once (shared across fields)
        let phrase_regexes: Vec<_> = if parsed.has_phrases() {
            parsed
                .phrases
                .iter()
                .map(|p| build_phrase_regex_cached(p))
                .collect::<Result<Vec<_>>>()?
        } else {
            Vec::new()
        };

        // ---- Per-field TF-IDF search, collecting (doc_id → best score + tokens + field) ----
        // Key: doc_id, Value: (best_score, merged_matched_tokens, set of matching fields)
        let mut doc_scores: HashMap<crate::DocumentId, (f64, HashSet<String>, Vec<String>)> =
            HashMap::new();

        // Collect per-field FTS options for highlight/tokenize
        let mut field_fts_options: HashMap<String, crate::fulltext::FtsOptions> = HashMap::new();

        {
            let indexes = self.indexes.read();

            for &field in fields {
                let fulltext_index =
                    indexes.get_fulltext_index_for_field(field).ok_or_else(|| {
                        IronBaseError::IndexError(format!(
                            "No fulltext index found for field '{}'",
                            field
                        ))
                    })?;

                field_fts_options.insert(field.to_string(), fulltext_index.options.clone());

                let required_token_count = if and_mode {
                    tokenize_unique(query, &fulltext_index.options).len()
                } else {
                    0
                };

                // Search this field's index
                let search_results = if let Some(ref target_ids) = options.target_doc_ids {
                    fulltext_index.search_for_doc_ids_with_ctx(
                        query,
                        target_ids,
                        candidate_limit,
                        0,
                        options.min_score,
                        ctx.as_ref(),
                    )?
                } else if !parsed.has_phrases() && and_mode {
                    fulltext_index.search_and_with_ctx(
                        query,
                        candidate_limit,
                        0,
                        options.min_score,
                        ctx.as_ref(),
                    )?
                } else if !parsed.has_phrases() {
                    fulltext_index.search_with_ctx(
                        query,
                        candidate_limit,
                        0, // Skip handled manually after merge
                        options.min_score,
                        ctx.as_ref(),
                    )?
                } else {
                    let fts_opts = &fulltext_index.options;
                    let all_terms = parsed.all_search_terms(fts_opts);
                    fulltext_index.search_with_ctx(
                        &all_terms.join(" "),
                        candidate_limit,
                        0,
                        options.min_score,
                        ctx.as_ref(),
                    )?
                };

                for fts_result in search_results {
                    // AND mode: skip docs that don't match all tokens for this field
                    if and_mode
                        && required_token_count > 0
                        && fts_result.matched_tokens.len() < required_token_count
                    {
                        continue;
                    }

                    let entry = doc_scores
                        .entry(fts_result.doc_id.clone())
                        .or_insert_with(|| (0.0, HashSet::new(), Vec::new()));

                    // Best score across fields (max)
                    if fts_result.score > entry.0 {
                        entry.0 = fts_result.score;
                    }

                    // Union matched tokens
                    for token in &fts_result.matched_tokens {
                        entry.1.insert(token.clone());
                    }

                    // Track which fields matched
                    entry.2.push(field.to_string());
                }
            }
        } // drop indexes read lock

        // Select top-K by score descending.
        // When there is no post-filter (no parsed_filter, no phrase_regexes),
        // every candidate passes through, so Top-K + skip is exact and uses
        // O(skip + limit) memory instead of sorting the full O(candidate_limit) vec.
        // When a filter/phrase IS present, candidates may be rejected after
        // sorting, so we need the full sorted list (candidate_limit is already
        // capped at 100K by calculate_candidate_limit, so this is bounded).
        let has_post_filter = parsed_filter.is_some() || !phrase_regexes.is_empty();
        // Score descending, doc_id ascending on ties, as in single-field
        // search: `doc_scores` is a HashMap, so comparing only the score left
        // ties in hash order, different on every call (hybrid review M2)
        let score_cmp =
            |a: &(crate::DocumentId, (f64, HashSet<String>, Vec<String>)),
             b: &(crate::DocumentId, (f64, HashSet<String>, Vec<String>))| {
                b.1 .0
                    .partial_cmp(&a.1 .0)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| a.0.cmp(&b.0))
            };

        let sorted_docs: Vec<_> = if has_post_filter {
            // Full sort — post-filter may reject arbitrary elements
            let mut v: Vec<_> = doc_scores.into_iter().collect();
            v.sort_by(score_cmp);
            v
        } else {
            // Top-K selection — O(skip + limit) memory, O(n log(skip + limit)) time
            topk_select_with_skip(
                doc_scores.into_iter(),
                effective_skip,
                effective_limit,
                score_cmp,
            )
        };

        // Prepare query tokens for highlights (use first field's options)
        let query_tokens = if options.highlight {
            field_fts_options
                .values()
                .next()
                .map(|fts_opts| tokenize_unique(query, fts_opts))
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        // Process results: load doc, filter, project, highlight
        let mut results = Vec::new();
        let mut skipped = 0;
        let parse_skip =
            projection_parse_skip(options.projection.as_ref(), fields, options.filter.as_ref());

        for (iteration, (doc_id, (score, matched_token_set, matched_fields))) in
            sorted_docs.into_iter().enumerate()
        {
            // Cancellation check
            if let Some(ref exec_ctx) = ctx {
                exec_ctx.maybe_check(iteration)?;
            }

            // Load document
            let doc = match self.read_document_by_id_skipping(&doc_id, &parse_skip)? {
                Some(d) => d,
                None => continue,
            };

            // Phrase check on ANY matching field
            if !phrase_regexes.is_empty() {
                let any_field_matches = matched_fields.iter().any(|f| {
                    let text = doc.get(f.as_str()).and_then(|v| v.as_str()).unwrap_or("");
                    phrase_regexes.iter().all(|re| re.is_match(text))
                });
                if !any_field_matches {
                    continue;
                }
            }

            // Apply MongoDB filter
            if let Some(ref filter) = parsed_filter {
                match Document::from_value(&doc) {
                    Ok(document) => {
                        if !filter.matches(&document)? {
                            continue;
                        }
                    }
                    Err(_) => continue,
                }
            }

            // Handle skip (only needed for full-sort path; Top-K path
            // already applied skip inside topk_select_with_skip)
            if has_post_filter && skipped < effective_skip {
                skipped += 1;
                continue;
            }

            // Apply projection
            let projected_doc = if let Some(ref proj) = options.projection {
                crate::find_options::apply_projection(&doc, proj)?
            } else {
                doc.clone()
            };

            // Generate highlights from all matching fields
            let highlights = if options.highlight && !query_tokens.is_empty() {
                let mut all_highlights = Vec::new();
                for field in &matched_fields {
                    if let Some(fts_opts) = field_fts_options.get(field.as_str()) {
                        let field_value = get_nested_value(&doc, field)
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        if !field_value.is_empty() {
                            let tokens = tokenize_unique(query, fts_opts);
                            let result = generate_highlights(
                                field_value,
                                &tokens,
                                field,
                                fts_opts,
                                &highlight_opts,
                            );
                            if !result.snippets.is_empty() {
                                all_highlights.push(result);
                            }
                        }
                    }
                }
                if all_highlights.is_empty() {
                    None
                } else {
                    Some(all_highlights)
                }
            } else {
                None
            };

            let matched_tokens: Vec<String> = matched_token_set.into_iter().collect();

            results.push(FulltextSearchResultExt {
                document: projected_doc,
                score,
                matched_tokens,
                highlights,
            });

            if results.len() >= effective_limit {
                break;
            }
        }

        Ok(results)
    }

    /// List all fulltext indexes for this collection
    pub fn list_fulltext_indexes(&self) -> Result<Vec<FulltextIndexMetadata>> {
        self.check_not_closed()?;
        let indexes = self.indexes.read();
        Ok(indexes
            .list_fulltext_indexes()
            .into_iter()
            .filter(|idx| idx.name.starts_with(&format!("{}_", self.name)))
            .map(|idx| idx.metadata())
            .collect())
    }

    /// List all fuzzy indexes for this collection
    ///
    /// Mirrors [`list_fulltext_indexes`](Self::list_fulltext_indexes): filters by the
    /// `{collection}_` name prefix and clones the stored metadata (name, field,
    /// algorithm, threshold, num_entries).
    pub fn list_fuzzy_indexes(&self) -> Result<Vec<crate::index::fuzzy::FuzzyIndexMetadata>> {
        self.check_not_closed()?;
        let indexes = self.indexes.read();
        let prefix = format!("{}_", self.name);
        Ok(indexes
            .list_fuzzy_indexes()
            .into_iter()
            .filter(|idx| idx.metadata.name.starts_with(&prefix))
            .map(|idx| idx.metadata.clone())
            .collect())
    }

    /// Get field names that have fulltext indexes (lightweight)
    ///
    /// Unlike `list_fulltext_indexes()`, this does NOT call `metadata()` on each index,
    /// avoiding the expensive `unique_token_count()` computation in lazy mode.
    /// Only accesses `pub name` and `pub field` String fields — O(1) per index.
    pub fn fulltext_indexed_fields(&self) -> Result<Vec<String>> {
        self.check_not_closed()?;
        let indexes = self.indexes.read();
        let prefix = format!("{}_", self.name);
        Ok(indexes
            .list_fulltext_indexes()
            .into_iter()
            .filter(|idx| idx.name.starts_with(&prefix))
            .map(|idx| idx.field.clone())
            .collect())
    }

    /// Get fulltext index options for a field
    ///
    /// Returns the FtsOptions used by the fulltext index on the specified field.
    /// This is useful for generating highlights with the same tokenization settings.
    pub fn get_fulltext_index_options(&self, field: &str) -> Result<crate::fulltext::FtsOptions> {
        self.check_not_closed()?;
        let indexes = self.indexes.read();
        let index = indexes.get_fulltext_index_for_field(field).ok_or_else(|| {
            IronBaseError::IndexError(format!("No fulltext index found for field '{}'", field))
        })?;
        Ok(index.options.clone())
    }

    /// Restricts a fulltext search to document-level AND when asked for.
    ///
    /// With `and_mode` and `document_scope` (the caller's `match_scope` is not
    /// `"chunk"`), the documents holding every query token across their chunks
    /// are qualified first ([`Self::fulltext_qualify_documents`]); the search
    /// then runs in OR mode inside them, combined with `user_filter`. Without a
    /// fulltext index on the collection nothing is restricted. Shared by the
    /// hybrid and the plain fulltext search.
    pub fn fulltext_document_scope(
        &self,
        fields: &[&str],
        query: &str,
        and_mode: bool,
        document_scope: bool,
        user_filter: Option<Value>,
    ) -> Result<QualificationOutcome> {
        let is_doc_scope = and_mode && document_scope;
        if !is_doc_scope || fields.is_empty() {
            return Ok(QualificationOutcome {
                is_doc_scope,
                effective_and_mode: and_mode,
                effective_filter: user_filter,
                qualified_doc_count: None,
            });
        }
        // A collection without fulltext indexes (non-RAG) is not restricted
        if self
            .fulltext_indexed_fields()
            .unwrap_or_default()
            .is_empty()
        {
            return Ok(QualificationOutcome {
                is_doc_scope: false,
                effective_and_mode: and_mode,
                effective_filter: user_filter,
                qualified_doc_count: None,
            });
        }

        let qualify_start = std::time::Instant::now();
        let qualification = self.fulltext_qualify_documents(fields, query)?;
        let qualified_doc_count = match &qualification {
            DocumentQualification::NotRequired => None,
            DocumentQualification::Documents { ids, .. } => Some(ids.len()),
        };
        if let Some(count) = qualified_doc_count {
            tracing::info!(
                collection = %self.name,
                qualified = count,
                elapsed_ms = qualify_start.elapsed().as_millis() as u64,
                "document-level AND qualification"
            );
        }

        // The qualified documents hold every token, so the search runs in OR
        // mode inside them
        let (effective_and_mode, effective_filter) = match qualification.to_filter() {
            Some(qualified) => {
                let merged = match user_filter {
                    Some(user) => serde_json::json!({ "$and": [user, qualified] }),
                    None => qualified,
                };
                (false, Some(merged))
            }
            None => (and_mode, user_filter),
        };
        Ok(QualificationOutcome {
            is_doc_scope,
            effective_and_mode,
            effective_filter,
            qualified_doc_count,
        })
    }

    /// Document-level AND: the documents whose indexed text holds every query
    /// token, possibly spread over several chunks.
    ///
    /// A RAG index (its chunk → parent mapping is populated) intersects the
    /// tokens' posting lists per parent document and keys the result on the
    /// parent field (`doc_id`); any other index intersects per indexed
    /// document (`_id`), since each document is its own identity there.
    /// Tokens are taken rarest first, so an empty intersection stops early.
    ///
    /// With several `fields` a document qualifies if it holds every token in
    /// ANY one of them (union over the fields). A query of fewer than two
    /// tokens on any field restricts nothing: `NotRequired`.
    pub fn fulltext_qualify_documents(
        &self,
        fields: &[&str],
        query: &str,
    ) -> Result<DocumentQualification> {
        self.check_not_closed()?;
        let indexes = self.indexes.read();
        let mut field_key: Option<String> = None;
        let mut union: HashMap<String, Value> = HashMap::new();
        for field in fields {
            let index = indexes.get_fulltext_index_for_field(field).ok_or_else(|| {
                IronBaseError::IndexError(format!("No fulltext index for field '{}'", field))
            })?;
            let tokens = index.tokenize_query(query);
            if tokens.len() <= 1 {
                return Ok(DocumentQualification::NotRequired);
            }
            let mut tokens: Vec<(String, usize)> = tokens
                .into_iter()
                .map(|t| {
                    let count = index.token_posting_count(&t);
                    (t, count)
                })
                .collect();
            tokens.sort_by_key(|(_, count)| *count);

            let by_parent = index.has_chunk_doc_mapping();
            let mut qualified: Option<HashMap<String, Value>> = None;
            for (token, count) in &tokens {
                let ids: HashMap<String, Value> = if *count == 0 {
                    HashMap::new()
                } else if by_parent {
                    let chunk_ids = index.token_chunk_ids(token);
                    index
                        .resolve_chunk_doc_ids(&chunk_ids)
                        .into_values()
                        .map(|doc_id| (doc_id.to_string(), Value::String(doc_id.to_string())))
                        .collect()
                } else {
                    index
                        .token_chunk_ids(token)
                        .into_iter()
                        .filter_map(|id| {
                            let value = serde_json::to_value(&id).ok()?;
                            Some((super::doc_id_to_string(&id), value))
                        })
                        .collect()
                };
                let next = match qualified {
                    None => ids,
                    Some(prev) => prev
                        .into_iter()
                        .filter(|(k, _)| ids.contains_key(k))
                        .collect(),
                };
                let empty = next.is_empty();
                qualified = Some(next);
                if empty {
                    break;
                }
            }

            let qualified = qualified.unwrap_or_default();
            // An index without matches says nothing about the key; the fields
            // of one collection agree on it otherwise.
            if !qualified.is_empty() {
                field_key.get_or_insert_with(|| {
                    if by_parent {
                        index.parent_doc_id_field().to_string()
                    } else {
                        "_id".to_string()
                    }
                });
            }
            for (k, v) in qualified {
                union.entry(k).or_insert(v);
            }
        }
        Ok(DocumentQualification::Documents {
            field: field_key.unwrap_or_else(|| "_id".to_string()),
            ids: union.into_values().collect(),
        })
    }
}

/// Result of [`CollectionCore::fulltext_document_scope`]
#[derive(Debug, Clone, PartialEq)]
pub struct QualificationOutcome {
    /// Document-level AND is in effect (`and_mode` + document scope)
    pub is_doc_scope: bool,
    /// The fulltext mode to search with: OR once the documents are qualified
    pub effective_and_mode: bool,
    /// The caller's filter, combined with the qualified documents
    pub effective_filter: Option<Value>,
    /// Number of qualified documents, when they were qualified
    pub qualified_doc_count: Option<usize>,
}

/// Result of [`CollectionCore::fulltext_qualify_documents`].
#[derive(Debug, Clone, PartialEq)]
pub enum DocumentQualification {
    /// The query has fewer than two tokens: every document qualifies.
    NotRequired,
    /// The qualifying documents: `ids` are values of `field` (`doc_id` for a
    /// RAG index, `_id` otherwise). Empty when no document qualifies.
    Documents { field: String, ids: Vec<Value> },
}

impl DocumentQualification {
    /// The restriction as a query filter (`{field: {"$in": ids}}`), `None`
    /// when nothing is restricted.
    pub fn to_filter(&self) -> Option<Value> {
        match self {
            Self::NotRequired => None,
            Self::Documents { field, ids } => {
                let mut filter = serde_json::Map::new();
                filter.insert(field.clone(), serde_json::json!({ "$in": ids }));
                Some(Value::Object(filter))
            }
        }
    }
}

/// Top-level fields a search may leave unparsed when it loads its results:
/// the ones an exclusion `projection` drops anyway, unless the search reads
/// them before projecting (`fields`, the filter). Nothing for an inclusion
/// projection, a nested exclusion, or a filter that may read any field
/// (`$expr`, `$where`, `$**`) (hybrid review T2).
pub(crate) fn projection_parse_skip(
    projection: Option<&HashMap<String, i32>>,
    fields: &[&str],
    filter: Option<&Value>,
) -> HashSet<String> {
    let Some(projection) = projection else {
        return HashSet::new();
    };
    if projection.values().any(|&v| v != 0) {
        return HashSet::new();
    }
    let mut read = HashSet::new();
    if let Some(filter) = filter {
        if !collect_filter_roots(filter, &mut read) {
            return HashSet::new();
        }
    }
    read.extend(fields.iter().map(|f| field_root(f).to_string()));
    projection
        .keys()
        .filter(|k| !k.contains('.') && !read.contains(k.as_str()))
        .cloned()
        .collect()
}

fn field_root(path: &str) -> &str {
    path.split('.').next().unwrap_or(path)
}

/// Add the top-level fields `filter` reads to `roots`; false when it may read
/// any field
fn collect_filter_roots(filter: &Value, roots: &mut HashSet<String>) -> bool {
    match filter {
        Value::Object(map) => map.iter().all(|(key, value)| {
            if key == "$expr" || key == "$where" || key.starts_with("$**") {
                false
            } else if key.starts_with('$') {
                collect_filter_roots(value, roots)
            } else {
                roots.insert(field_root(key).to_string());
                true
            }
        }),
        Value::Array(items) => items.iter().all(|item| collect_filter_roots(item, roots)),
        _ => true,
    }
}
