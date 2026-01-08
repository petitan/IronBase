// Search operations for CollectionCore
// Fuzzy text search and full-text search with TF-IDF

use serde_json::Value;
use std::collections::HashMap;

use crate::error::{IronBaseError, Result};
use crate::execution::ExecutionContext;
use crate::fulltext::{build_phrase_regex, parse_query, FulltextIndexMetadata};
use crate::index::FuzzyAlgorithm;
use crate::log_error;
use crate::storage::{RawStorage, Storage};
use crate::value_utils::get_nested_value;

use super::index_persistence::build_fulltext_index_file_path;
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

        // Step 1: Create the fuzzy index in IndexManager
        {
            let mut indexes = self.indexes.write();
            indexes.create_fuzzy_index(index_name.clone(), field.clone(), algorithm, threshold)?;
        }

        // Step 2 & 3: Scan documents IN BATCHES and populate index
        // This is memory-efficient for large collections
        const INDEX_BUILD_BATCH_SIZE: usize = 100;

        let field_clone = field.clone();
        let index_name_clone = index_name.clone();
        let indexes_ref = self.indexes.clone();

        self.scan_documents_in_batches(INDEX_BUILD_BATCH_SIZE, |_batch_num, batch_docs| {
            let mut indexes = indexes_ref.write();
            if let Some(index) = indexes.get_fuzzy_index_mut(&index_name_clone) {
                for (doc_id, doc) in &batch_docs {
                    if let Some(value) = get_nested_value(doc, &field_clone) {
                        if let Some(s) = value.as_str() {
                            index.insert(s, doc_id.clone());
                        }
                    }
                }
            }
            Ok(())
        })?;

        // Mark index as ready after population is complete
        {
            let mut indexes = self.indexes.write();
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
            storage.flush()?;
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
        const INDEX_BUILD_BATCH_SIZE: usize = 100;

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
                                // Log error but continue indexing other documents
                                if let Err(e) = index.insert(doc_id, s) {
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
        if let Some(ref meta) = metadata {
            if meta.num_documents == 0 {
                // Empty index - likely a failed creation, clean up
                cleanup(&index_name, &storage_path);
                return Err(crate::error::IronBaseError::IndexError(format!(
                    "Fulltext index '{}' has no documents - field '{}' may not exist or contain text",
                    index_name, field
                )));
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
            storage.flush()?;
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

    /// Fulltext search with execution context for cancellation support.
    ///
    /// This is the cancellation-aware version of `fulltext_search`.
    /// Pass an ExecutionContext to enable timeout/cancellation checking.
    ///
    /// # MongoDB-compatible phrase search
    ///
    /// Supports quoted phrases for exact matching (MongoDB $text syntax):
    /// - `"exact phrase"` - matches documents containing the exact phrase
    /// - `word1 word2` - matches documents containing either word (OR)
    /// - `"phrase one" other words` - mixed: phrase AND/OR other words
    ///
    /// # Example
    /// ```rust,ignore
    /// // OR search (default)
    /// coll.fulltext_search_with_ctx("content", "hello world", ...)?;
    ///
    /// // Phrase search (exact match)
    /// coll.fulltext_search_with_ctx("content", "\"hello world\"", ...)?;
    /// ```
    pub fn fulltext_search_with_ctx(
        &self,
        field: &str,
        query: &str,
        limit: Option<usize>,
        skip: Option<usize>,
        min_score: Option<f64>,
        projection: Option<HashMap<String, i32>>,
        ctx: Option<&ExecutionContext>,
    ) -> Result<Vec<(Value, f64, Vec<String>)>> {
        self.check_not_closed()?;

        // Parse query for phrases (MongoDB-compatible syntax)
        let parsed = parse_query(query);
        let effective_limit = limit.unwrap_or(10);
        let effective_skip = skip.unwrap_or(0);

        let indexes = self.indexes.read();

        // Find fulltext index for this field
        let fulltext_index = indexes.get_fulltext_index_for_field(field).ok_or_else(|| {
            IronBaseError::IndexError(format!("No fulltext index found for field '{}'", field))
        })?;

        // If no phrases, use fast path (existing behavior)
        if !parsed.has_phrases() {
            let search_results = fulltext_index.search_with_ctx(
                query,
                effective_limit,
                effective_skip,
                min_score,
                ctx,
            )?;

            drop(indexes);

            // Fetch documents for matched IDs with cancellation checks
            let mut results = Vec::with_capacity(search_results.len());
            for (iteration, result) in search_results.into_iter().enumerate() {
                if let Some(exec_ctx) = ctx {
                    exec_ctx.maybe_check(iteration)?;
                }

                if let Some(doc) = self.read_document_by_id(&result.doc_id)? {
                    let projected_doc = if let Some(ref proj) = projection {
                        crate::find_options::apply_projection(&doc, proj)?
                    } else {
                        doc
                    };
                    results.push((projected_doc, result.score, result.matched_tokens));
                }
            }

            return Ok(results);
        }

        // Phrase search: get more candidates, then filter with regex
        // Request limit * 10 candidates to account for regex filtering
        let candidate_limit = effective_limit.saturating_mul(10).max(100);

        // Build search query from all terms (phrases tokenized + regular terms)
        let all_terms = parsed.all_search_terms(&fulltext_index.options);

        // Search for candidates (skip=0, we handle skip manually after filtering)
        let search_results = fulltext_index.search_with_ctx(
            &all_terms.join(" "),
            candidate_limit,
            0, // No skip - we apply it after phrase filtering
            min_score,
            ctx,
        )?;

        drop(indexes);

        // Build regex patterns for all phrases
        let phrase_regexes: Vec<_> = parsed
            .phrases
            .iter()
            .map(|p| build_phrase_regex(p))
            .collect::<Result<Vec<_>>>()?;

        // Streaming doc loading with phrase filtering
        let mut results = Vec::new();
        let mut skipped = 0;

        for (iteration, result) in search_results.into_iter().enumerate() {
            // Cancellation check
            if let Some(exec_ctx) = ctx {
                exec_ctx.maybe_check(iteration)?;
            }

            if let Some(doc) = self.read_document_by_id(&result.doc_id)? {
                // Get field text for phrase matching
                let text = doc.get(field).and_then(|v| v.as_str()).unwrap_or("");

                // Check all phrases match
                let all_phrases_match = phrase_regexes.iter().all(|re| re.is_match(text));

                if all_phrases_match {
                    // Handle skip
                    if skipped < effective_skip {
                        skipped += 1;
                        continue;
                    }

                    // Apply projection
                    let projected_doc = if let Some(ref proj) = projection {
                        crate::find_options::apply_projection(&doc, proj)?
                    } else {
                        doc
                    };

                    results.push((projected_doc, result.score, result.matched_tokens));

                    // Early exit when limit reached
                    if results.len() >= effective_limit {
                        break;
                    }
                }
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
}
