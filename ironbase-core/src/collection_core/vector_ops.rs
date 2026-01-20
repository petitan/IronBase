//! Vector index operations for CollectionCore
//!
//! Provides HNSW-based vector similarity search capabilities.
//!
//! # Features
//!
//! - Create/drop vector indexes
//! - Vector similarity search with configurable metrics
//! - Hybrid search (vector + attribute filter)
//! - Automatic index persistence as .hnsw cache files

use std::path::PathBuf;

use serde_json::Value;

use crate::document::DocumentId;
use crate::error::{IronBaseError, Result};
use crate::storage::{RawStorage, Storage};
use crate::value_utils::get_nested_value;
use crate::vector::{HnswIndex, VectorIndexConfig, VectorIndexMetadata, VectorSearchResult};

use super::CollectionCore;

/// Convert DocumentId to String for HNSW storage
fn doc_id_to_string(id: &DocumentId) -> String {
    match id {
        DocumentId::Int(i) => i.to_string(),
        DocumentId::String(s) => s.clone(),
        DocumentId::ObjectId(oid) => oid.clone(),
    }
}

/// Convert String back to DocumentId
fn string_to_doc_id(s: &str) -> DocumentId {
    // Try to parse as int first
    if let Ok(i) = s.parse::<i64>() {
        DocumentId::Int(i)
    } else if s.len() == 24 && s.chars().all(|c| c.is_ascii_hexdigit()) {
        DocumentId::ObjectId(s.to_string())
    } else {
        DocumentId::String(s.to_string())
    }
}

/// Vector index operations for CollectionCore
impl<S: Storage + RawStorage> CollectionCore<S> {
    /// Create a vector index on a field
    ///
    /// # Arguments
    ///
    /// * `field` - Field containing the embedding vectors
    /// * `config` - Vector index configuration (dimension, metric, HNSW params)
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use ironbase_core::vector::{VectorIndexConfig, DistanceMetric};
    ///
    /// let config = VectorIndexConfig::new(300)
    ///     .with_metric(DistanceMetric::Cosine)
    ///     .with_max_vectors(50_000);
    ///
    /// let index_name = collection.create_vector_index("embedding", config)?;
    /// ```
    pub fn create_vector_index(&self, field: &str, config: VectorIndexConfig) -> Result<String> {
        self.check_not_closed()?;

        // Validate config
        config
            .validate()
            .map_err(|e| IronBaseError::IndexError(e.to_string()))?;

        let index_name = format!("{}_vec_{}", self.name, field);

        tracing::info!(
            collection = %self.name,
            field = %field,
            index_name = %index_name,
            dim = config.dim,
            metric = ?config.metric,
            "Starting vector index creation"
        );

        // Check if index already exists
        {
            let storage = self.storage.read();
            if let Some(meta) = storage.get_collection_meta(&self.name) {
                if meta.vector_indexes.iter().any(|idx| idx.name == index_name) {
                    return Err(IronBaseError::IndexError(format!(
                        "Vector index '{}' already exists",
                        index_name
                    )));
                }
            }
        }

        // Create the HNSW index with field info for auto-indexing
        let config_with_field = VectorIndexConfig {
            field: field.to_string(),
            ..config
        };
        let mut hnsw = HnswIndex::new(config_with_field.clone());

        // Scan documents and build the index
        use crate::limits::INDEX_BUILD_BATCH_SIZE;
        const PROGRESS_LOG_INTERVAL: usize = 1000;
        let mut indexed_count: usize = 0;
        let mut total_scanned: usize = 0;
        let mut dimension_errors: Vec<String> = Vec::new();
        let field_clone = field.to_string();
        let collection_name = self.name.clone();

        self.scan_documents_in_batches(INDEX_BUILD_BATCH_SIZE, |_batch_num, batch_docs| {
            for (doc_id, doc) in batch_docs {
                let id_str = doc_id_to_string(&doc_id);
                // Get the vector field value
                if let Some(vec_value) = get_nested_value(&doc, &field_clone) {
                    if let Some(vector) = Self::extract_f32_vector(vec_value) {
                        // Check dimension
                        if vector.len() != config.dim {
                            dimension_errors.push(format!(
                                "Document '{}': expected dim {}, got {}",
                                id_str,
                                config.dim,
                                vector.len()
                            ));
                            if dimension_errors.len() <= 5 {
                                tracing::warn!(
                                    doc_id = %id_str,
                                    expected = config.dim,
                                    actual = vector.len(),
                                    "Vector dimension mismatch, skipping"
                                );
                            }
                        } else {
                            // Insert into HNSW
                            if let Err(e) = hnsw.insert(&id_str, &vector) {
                                tracing::warn!(
                                    doc_id = %id_str,
                                    error = %e,
                                    "Failed to insert vector, skipping"
                                );
                            } else {
                                indexed_count += 1;
                            }
                        }
                    }
                }
                total_scanned += 1;

                // Progress logging
                if total_scanned % PROGRESS_LOG_INTERVAL == 0 {
                    tracing::info!(
                        collection = %collection_name,
                        scanned = total_scanned,
                        indexed = indexed_count,
                        "Vector index build progress"
                    );
                }
            }
            Ok(())
        })?;

        tracing::info!(
            collection = %self.name,
            total_scanned = total_scanned,
            indexed = indexed_count,
            dimension_errors = dimension_errors.len(),
            "Document scan complete, persisting index"
        );

        // Get database file path for cache file
        let cache_file_name = format!("{}.hnsw", index_name);
        let db_file_path = {
            let storage = self.storage.read();
            storage.get_file_path().to_string()
        };

        // Save HNSW cache file if we have a file-based storage
        if !db_file_path.is_empty() {
            let cache_path = Self::build_hnsw_cache_path(&db_file_path, &cache_file_name);
            let bytes = hnsw.to_bytes()?;
            std::fs::write(&cache_path, &bytes).map_err(|e| {
                IronBaseError::Io(std::io::Error::other(format!(
                    "Failed to write HNSW cache file: {}",
                    e
                )))
            })?;
            tracing::info!(
                cache_path = %cache_path.display(),
                size_bytes = bytes.len(),
                "HNSW cache file saved"
            );
        }

        hnsw.mark_clean();

        // Store the HNSW index in IndexManager for unified index management
        {
            let mut indexes = self.indexes.write();
            indexes.add_loaded_vector_index(index_name.clone(), hnsw);
        }

        // Create metadata
        let metadata = VectorIndexMetadata {
            name: index_name.clone(),
            field: field.to_string(),
            config: config_with_field,
            cache_file: cache_file_name,
            vector_count: indexed_count,
            created_at: chrono::Utc::now().to_rfc3339(),
        };

        // Persist metadata
        {
            let mut storage = self.storage.write();
            if let Some(meta) = storage.get_collection_meta_mut(&self.name) {
                meta.vector_indexes.push(metadata);
                storage.flush()?;
            }
        }

        tracing::info!(
            collection = %self.name,
            index_name = %index_name,
            vector_count = indexed_count,
            "Vector index creation complete"
        );

        Ok(index_name)
    }

    /// Drop a vector index
    pub fn drop_vector_index(&self, index_name: &str) -> Result<()> {
        self.check_not_closed()?;

        // Remove from IndexManager first
        {
            let mut indexes = self.indexes.write();
            // Don't fail if not in IndexManager (might not have been loaded)
            let _ = indexes.drop_vector_index(index_name);
        }

        // Find and remove the index metadata
        let cache_file = {
            let mut storage = self.storage.write();
            if let Some(meta) = storage.get_collection_meta_mut(&self.name) {
                let idx_pos = meta
                    .vector_indexes
                    .iter()
                    .position(|idx| idx.name == index_name);
                if let Some(pos) = idx_pos {
                    let idx_meta = meta.vector_indexes.remove(pos);
                    storage.flush()?;
                    Some(idx_meta.cache_file)
                } else {
                    return Err(IronBaseError::IndexError(format!(
                        "Vector index '{}' not found",
                        index_name
                    )));
                }
            } else {
                return Err(IronBaseError::CollectionNotFound(self.name.clone()));
            }
        };

        // Delete cache file if it exists
        if let Some(cache_file_name) = cache_file {
            let db_file_path = {
                let storage = self.storage.read();
                storage.get_file_path().to_string()
            };

            if !db_file_path.is_empty() {
                let cache_path = Self::build_hnsw_cache_path(&db_file_path, &cache_file_name);
                if cache_path.exists() {
                    if let Err(e) = std::fs::remove_file(&cache_path) {
                        tracing::warn!(
                            cache_path = %cache_path.display(),
                            error = %e,
                            "Failed to delete HNSW cache file"
                        );
                    }
                }
            }
        }

        tracing::info!(
            collection = %self.name,
            index_name = %index_name,
            "Vector index dropped"
        );

        Ok(())
    }

    /// List all vector indexes on this collection
    pub fn list_vector_indexes(&self) -> Result<Vec<VectorIndexMetadata>> {
        self.check_not_closed()?;

        let storage = self.storage.read();
        if let Some(meta) = storage.get_collection_meta(&self.name) {
            Ok(meta.vector_indexes.clone())
        } else {
            Ok(Vec::new())
        }
    }

    /// Ensure all vector indexes are loaded into IndexManager for auto-indexing on insert/update.
    ///
    /// This should be called during warm-up to enable auto-indexing of new documents.
    /// Without this, vector indexes are lazily loaded only during search, which means
    /// documents inserted before the first search won't be auto-indexed.
    ///
    /// # Returns
    /// Number of vector indexes loaded
    pub fn ensure_vector_indexes_loaded(&self) -> Result<usize> {
        self.check_not_closed()?;

        let vec_indexes = self.list_vector_indexes()?;
        let mut loaded_count = 0;

        for meta in vec_indexes {
            let index_name = meta.name.clone();

            // Check if already loaded
            {
                let indexes = self.indexes.read();
                if indexes.get_vector_index(&index_name).is_some() {
                    continue;
                }
            }

            // Load from file and add to IndexManager
            match self.load_hnsw_index_from_file(&meta) {
                Ok(hnsw) => {
                    let vector_count = hnsw.len();
                    let mut indexes = self.indexes.write();
                    indexes.add_loaded_vector_index(index_name.clone(), hnsw);
                    tracing::info!(
                        collection = %self.name,
                        index = %index_name,
                        vectors = vector_count,
                        "Loaded vector index for auto-indexing"
                    );
                    loaded_count += 1;
                }
                Err(e) => {
                    tracing::warn!(
                        collection = %self.name,
                        index = %index_name,
                        error = %e,
                        "Failed to load vector index"
                    );
                }
            }
        }

        Ok(loaded_count)
    }

    /// Perform vector similarity search
    ///
    /// # Arguments
    ///
    /// * `field` - Field name with vector index
    /// * `query_vector` - Query embedding vector
    /// * `limit` - Maximum number of results
    ///
    /// # Returns
    ///
    /// Vector of (document, score) tuples sorted by similarity
    pub fn vector_search(
        &self,
        field: &str,
        query_vector: &[f32],
        limit: usize,
    ) -> Result<Vec<(Value, f32)>> {
        self.check_not_closed()?;

        // Find the vector index metadata
        let index_meta = self.get_vector_index_meta(field)?;
        let index_name = index_meta.name.clone();

        // Validate query dimension
        if query_vector.len() != index_meta.config.dim {
            return Err(IronBaseError::InvalidQuery(format!(
                "Query vector dimension mismatch: expected {}, got {}",
                index_meta.config.dim,
                query_vector.len()
            )));
        }

        // Try to get from IndexManager first (fast path)
        {
            let indexes = self.indexes.read();
            if let Some(hnsw) = indexes.get_vector_index(&index_name) {
                let results = hnsw.search(query_vector, limit);
                // Need to release lock before loading documents
                drop(indexes);
                return self.load_documents_for_results(results);
            }
        }

        // Slow path: load from file and add to IndexManager
        let hnsw = self.load_hnsw_index_from_file(&index_meta)?;
        let results = hnsw.search(query_vector, limit);

        // Add to IndexManager for future queries
        {
            let mut indexes = self.indexes.write();
            indexes.add_loaded_vector_index(index_name, hnsw);
        }

        // Load documents for results
        self.load_documents_for_results(results)
    }

    /// Perform vector similarity search with attribute filter
    ///
    /// # Arguments
    ///
    /// * `field` - Field name with vector index
    /// * `query_vector` - Query embedding vector
    /// * `filter` - MongoDB-style query filter
    /// * `limit` - Maximum number of results
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // Find similar documents only from a specific category
    /// let results = collection.vector_search_with_filter(
    ///     "embedding",
    ///     &query_vec,
    ///     &json!({"category": "science"}),
    ///     10
    /// )?;
    /// ```
    pub fn vector_search_with_filter(
        &self,
        field: &str,
        query_vector: &[f32],
        filter: &Value,
        limit: usize,
    ) -> Result<Vec<(Value, f32)>> {
        self.check_not_closed()?;

        // Find the vector index metadata
        let index_meta = self.get_vector_index_meta(field)?;
        let index_name = index_meta.name.clone();

        // Validate query dimension
        if query_vector.len() != index_meta.config.dim {
            return Err(IronBaseError::InvalidQuery(format!(
                "Query vector dimension mismatch: expected {}, got {}",
                index_meta.config.dim,
                query_vector.len()
            )));
        }

        // Pre-filter: get matching document IDs
        // OOM PROTECTION: Use projection to only fetch _id field + limit based on system RAM
        let allowed_ids: std::collections::HashSet<String> = {
            use crate::find_options::FindOptions;
            use std::collections::HashMap;

            // Only fetch _id field to minimize memory usage
            let mut projection = HashMap::new();
            projection.insert("_id".to_string(), 1);

            // Use safe defaults for RAM-based limits + reasonable pre-filter limit
            // Even with projection, we cap at 100K documents for the pre-filter
            let options = FindOptions::with_safe_defaults()
                .with_projection(projection)
                .with_limit(100_000);

            let matching_docs = self.find_with_options(filter, options)?;
            matching_docs
                .iter()
                .filter_map(|doc| {
                    doc.get("_id").and_then(|id| match id {
                        Value::String(s) => Some(s.clone()),
                        Value::Number(n) => Some(n.to_string()),
                        _ => None,
                    })
                })
                .collect()
        };

        if allowed_ids.is_empty() {
            return Ok(Vec::new());
        }

        // Try to get from IndexManager first (fast path)
        {
            let indexes = self.indexes.read();
            if let Some(hnsw) = indexes.get_vector_index(&index_name) {
                let results =
                    hnsw.search_with_filter(query_vector, limit, |id| allowed_ids.contains(id));
                // Need to release lock before loading documents
                drop(indexes);
                return self.load_documents_for_results(results);
            }
        }

        // Slow path: load from file and add to IndexManager
        let hnsw = self.load_hnsw_index_from_file(&index_meta)?;
        let results = hnsw.search_with_filter(query_vector, limit, |id| allowed_ids.contains(id));

        // Add to IndexManager for future queries
        {
            let mut indexes = self.indexes.write();
            indexes.add_loaded_vector_index(index_name, hnsw);
        }

        // Load documents for results
        self.load_documents_for_results(results)
    }

    // === Private helper methods ===

    /// Get vector index metadata for a field
    fn get_vector_index_meta(&self, field: &str) -> Result<VectorIndexMetadata> {
        let storage = self.storage.read();
        if let Some(meta) = storage.get_collection_meta(&self.name) {
            // Try to find by field name first, then by index name
            let index_name = format!("{}_vec_{}", self.name, field);
            meta.vector_indexes
                .iter()
                .find(|idx| idx.field == field || idx.name == index_name)
                .cloned()
                .ok_or_else(|| {
                    IronBaseError::IndexError(format!(
                        "No vector index found for field '{}' in collection '{}'",
                        field, self.name
                    ))
                })
        } else {
            Err(IronBaseError::CollectionNotFound(self.name.clone()))
        }
    }

    /// Load HNSW index from cache file or rebuild from documents
    fn load_hnsw_index_from_file(&self, meta: &VectorIndexMetadata) -> Result<HnswIndex> {
        let db_file_path = {
            let storage = self.storage.read();
            storage.get_file_path().to_string()
        };

        // Try loading from cache
        if !db_file_path.is_empty() {
            let cache_path = Self::build_hnsw_cache_path(&db_file_path, &meta.cache_file);
            if cache_path.exists() {
                let bytes = std::fs::read(&cache_path).map_err(|e| {
                    IronBaseError::Io(std::io::Error::other(format!(
                        "Failed to read HNSW cache file: {}",
                        e
                    )))
                })?;
                match HnswIndex::from_bytes(&bytes) {
                    Ok(hnsw) => {
                        tracing::debug!(
                            cache_path = %cache_path.display(),
                            vectors = hnsw.len(),
                            "Loaded HNSW index from cache"
                        );
                        return Ok(hnsw);
                    }
                    Err(e) => {
                        tracing::warn!(
                            cache_path = %cache_path.display(),
                            error = %e,
                            "Failed to deserialize HNSW cache, will rebuild"
                        );
                    }
                }
            }
        }

        // Rebuild from documents
        tracing::info!(
            collection = %self.name,
            field = %meta.field,
            "Rebuilding HNSW index from documents"
        );

        let mut hnsw = HnswIndex::new(meta.config.clone());
        let field = meta.field.clone();

        self.scan_documents_in_batches(1000, |_, batch_docs| {
            for (doc_id, doc) in batch_docs {
                if let Some(vec_value) = get_nested_value(&doc, &field) {
                    if let Some(vector) = Self::extract_f32_vector(vec_value) {
                        if vector.len() == meta.config.dim {
                            let id_str = doc_id_to_string(&doc_id);
                            let _ = hnsw.insert(&id_str, &vector);
                        }
                    }
                }
            }
            Ok(())
        })?;

        // Save rebuilt cache
        if !db_file_path.is_empty() {
            let cache_path = Self::build_hnsw_cache_path(&db_file_path, &meta.cache_file);
            if let Ok(bytes) = hnsw.to_bytes() {
                let _ = std::fs::write(&cache_path, &bytes);
            }
        }

        Ok(hnsw)
    }

    /// Load documents for search results
    fn load_documents_for_results(
        &self,
        results: Vec<VectorSearchResult>,
    ) -> Result<Vec<(Value, f32)>> {
        let mut docs_with_scores = Vec::with_capacity(results.len());

        for result in results {
            // Parse the document ID
            let doc_id = string_to_doc_id(&result.id);

            // Load the document
            if let Some(doc) = self.read_document_by_id(&doc_id)? {
                docs_with_scores.push((doc, result.score));
            }
        }

        Ok(docs_with_scores)
    }

    /// Build the path for HNSW cache file
    fn build_hnsw_cache_path(db_file_path: &str, cache_file_name: &str) -> PathBuf {
        let db_path = PathBuf::from(db_file_path);
        let parent = db_path.parent().unwrap_or(&db_path);
        parent.join(cache_file_name)
    }

    /// Extract f32 vector from JSON value
    fn extract_f32_vector(value: &Value) -> Option<Vec<f32>> {
        match value {
            Value::Array(arr) => {
                let mut vector = Vec::with_capacity(arr.len());
                for v in arr {
                    match v {
                        Value::Number(n) => {
                            if let Some(f) = n.as_f64() {
                                vector.push(f as f32);
                            } else {
                                return None;
                            }
                        }
                        _ => return None,
                    }
                }
                Some(vector)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::MemoryStorage;
    use crate::DatabaseCore;
    use serde_json::json;
    use std::collections::HashMap;

    fn create_test_db() -> DatabaseCore<MemoryStorage> {
        DatabaseCore::<MemoryStorage>::open_memory().unwrap()
    }

    fn json_to_hashmap(value: Value) -> HashMap<String, Value> {
        match value {
            Value::Object(map) => map.into_iter().collect(),
            _ => HashMap::new(),
        }
    }

    #[test]
    fn test_create_vector_index() {
        let db = create_test_db();
        let coll = db.collection("test").unwrap();

        // Insert documents with vectors
        for i in 0..10 {
            let vector: Vec<f64> = (0..10).map(|j| ((i + j) % 10) as f64 / 10.0).collect();
            db.insert_one(
                "test",
                json_to_hashmap(json!({
                    "name": format!("doc{}", i),
                    "embedding": vector
                })),
            )
            .unwrap();
        }

        // Create vector index
        let config = VectorIndexConfig::new(10);
        let index_name = coll.create_vector_index("embedding", config).unwrap();

        assert!(index_name.contains("_vec_"));

        // List vector indexes
        let indexes = coll.list_vector_indexes().unwrap();
        assert_eq!(indexes.len(), 1);
        assert_eq!(indexes[0].field, "embedding");
        assert_eq!(indexes[0].config.dim, 10);
    }

    #[test]
    fn test_vector_search() {
        let db = create_test_db();
        let coll = db.collection("test").unwrap();

        // Insert documents with vectors
        for i in 0..20 {
            let vector: Vec<f64> = (0..5).map(|j| ((i + j) % 5) as f64 / 5.0).collect();
            db.insert_one(
                "test",
                json_to_hashmap(json!({
                    "name": format!("doc{}", i),
                    "category": if i % 2 == 0 { "even" } else { "odd" },
                    "embedding": vector
                })),
            )
            .unwrap();
        }

        // Create vector index
        let config = VectorIndexConfig::new(5);
        coll.create_vector_index("embedding", config).unwrap();

        // Search
        let query_vec: Vec<f32> = vec![0.0, 0.2, 0.4, 0.6, 0.8];
        let results = coll.vector_search("embedding", &query_vec, 5).unwrap();

        assert_eq!(results.len(), 5);

        // All results should have a score
        for (doc, score) in &results {
            assert!(doc.get("name").is_some());
            assert!(*score > 0.0);
        }
    }

    #[test]
    fn test_vector_search_with_filter() {
        let db = create_test_db();
        let coll = db.collection("test").unwrap();

        // Insert documents with vectors
        for i in 0..20 {
            let vector: Vec<f64> = (0..5).map(|j| ((i + j) % 5) as f64 / 5.0).collect();
            db.insert_one(
                "test",
                json_to_hashmap(json!({
                    "name": format!("doc{}", i),
                    "category": if i % 2 == 0 { "even" } else { "odd" },
                    "embedding": vector
                })),
            )
            .unwrap();
        }

        // Create vector index
        let config = VectorIndexConfig::new(5);
        coll.create_vector_index("embedding", config).unwrap();

        // Search only in "even" category
        let query_vec: Vec<f32> = vec![0.0, 0.2, 0.4, 0.6, 0.8];
        let results = coll
            .vector_search_with_filter("embedding", &query_vec, &json!({"category": "even"}), 10)
            .unwrap();

        // All results should be from "even" category
        for (doc, _score) in &results {
            assert_eq!(doc.get("category").unwrap(), "even");
        }
    }

    #[test]
    fn test_drop_vector_index() {
        let db = create_test_db();
        let coll = db.collection("test").unwrap();

        // Insert a document with vector
        db.insert_one(
            "test",
            json_to_hashmap(json!({
                "embedding": [0.1, 0.2, 0.3]
            })),
        )
        .unwrap();

        // Create index
        let config = VectorIndexConfig::new(3);
        let index_name = coll.create_vector_index("embedding", config).unwrap();

        // Verify index exists
        assert_eq!(coll.list_vector_indexes().unwrap().len(), 1);

        // Drop index
        coll.drop_vector_index(&index_name).unwrap();

        // Verify index is gone
        assert_eq!(coll.list_vector_indexes().unwrap().len(), 0);
    }

    #[test]
    fn test_vector_dimension_validation() {
        let db = create_test_db();
        let coll = db.collection("test").unwrap();

        // Insert document with 5-dim vector
        db.insert_one(
            "test",
            json_to_hashmap(json!({
                "embedding": [0.1, 0.2, 0.3, 0.4, 0.5]
            })),
        )
        .unwrap();

        // Create index for 5 dimensions
        let config = VectorIndexConfig::new(5);
        coll.create_vector_index("embedding", config).unwrap();

        // Search with wrong dimension should fail
        let wrong_dim_query: Vec<f32> = vec![0.1, 0.2, 0.3]; // 3 dims instead of 5
        let result = coll.vector_search("embedding", &wrong_dim_query, 5);
        assert!(result.is_err());
    }

    /// Test that documents inserted AFTER creating a vector index are automatically indexed.
    /// This is the critical auto-indexing feature.
    #[test]
    fn test_vector_auto_index_on_insert() {
        let db = create_test_db();
        let coll = db.collection("test").unwrap();

        // 1. Create vector index FIRST (empty, no documents yet)
        let config = VectorIndexConfig::new(4); // 4-dim vectors
        let _index_name = coll.create_vector_index("embedding", config).unwrap();

        // Verify index is created but empty
        let indexes = coll.list_vector_indexes().unwrap();
        assert_eq!(indexes.len(), 1);
        assert_eq!(
            indexes[0].vector_count, 0,
            "Index should be empty initially"
        );

        // 2. Insert a document with embedding AFTER index creation
        db.insert_one(
            "test",
            json_to_hashmap(json!({
                "name": "doc1",
                "embedding": [0.1, 0.2, 0.3, 0.4]
            })),
        )
        .unwrap();

        // 3. Verify vector search finds the document
        // vector_search returns Vec<(Value, f32)> - (doc, score)
        let query: Vec<f32> = vec![0.1, 0.2, 0.3, 0.4];
        let results = coll.vector_search("embedding", &query, 10).unwrap();

        assert_eq!(
            results.len(),
            1,
            "Auto-indexing failed: document inserted after index creation should be searchable"
        );
        assert_eq!(
            results[0].0.get("name").and_then(|v| v.as_str()),
            Some("doc1")
        );

        // 4. Insert more documents and verify they're all indexed
        for i in 2..=5 {
            let vector: Vec<f64> = (0..4).map(|j| (i + j) as f64 / 10.0).collect();
            db.insert_one(
                "test",
                json_to_hashmap(json!({
                    "name": format!("doc{}", i),
                    "embedding": vector
                })),
            )
            .unwrap();
        }

        // Search should return all 5 documents
        let results = coll.vector_search("embedding", &query, 10).unwrap();
        assert_eq!(
            results.len(),
            5,
            "All 5 documents should be searchable after auto-indexing"
        );
    }

    /// Test auto-indexing with DatabaseCore API (like MCP adapter uses)
    #[test]
    fn test_vector_auto_index_via_database_api() {
        let db = create_test_db();

        // 1. Get collection and create vector index
        let coll = db.collection("test").unwrap();
        let config = VectorIndexConfig::new(3);
        coll.create_vector_index("vec", config).unwrap();

        // 2. Use DatabaseCore::insert_one (not collection.insert_one)
        // This is what MCP adapter does
        db.insert_one(
            "test",
            json_to_hashmap(json!({
                "data": "test",
                "vec": [1.0, 2.0, 3.0]
            })),
        )
        .unwrap();

        // 3. Get collection again (simulating separate MCP request)
        let coll2 = db.collection("test").unwrap();

        // 4. Vector search should find the document
        let query: Vec<f32> = vec![1.0, 2.0, 3.0];
        let results = coll2.vector_search("vec", &query, 10).unwrap();

        assert_eq!(
            results.len(),
            1,
            "Document inserted via db.insert_one() should be auto-indexed"
        );
    }
}
