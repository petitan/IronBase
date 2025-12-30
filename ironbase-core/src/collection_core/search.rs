// Search operations for CollectionCore
// Fuzzy text search and full-text search with TF-IDF

use serde_json::Value;
use std::collections::HashMap;

use crate::error::{IronBaseError, Result};
use crate::fulltext::FulltextIndexMetadata;
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

        // Step 4: Flush fulltext index to disk and get metadata
        let metadata = {
            let mut indexes = self.indexes.write();
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
}
