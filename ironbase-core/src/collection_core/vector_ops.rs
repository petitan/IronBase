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

use std::collections::HashSet;
use std::path::PathBuf;

use serde_json::Value;

use crate::document::DocumentId;
use crate::error::{IronBaseError, Result};
use crate::execution::ExecutionContext;
use crate::index::{IndexKey, RangeQueryMode, ScanOrder};
use crate::storage::{RawStorage, Storage};
use crate::value_utils::get_nested_value;
use crate::vector::{
    HnswIndex, VectorIndexConfig, VectorIndexMetadata, VectorSearchOptions, VectorSearchResult,
};

use super::CollectionCore;

/// Convert DocumentId to String for HNSW storage
///
/// Uses type-prefixed format to preserve DocumentId variant across roundtrip:
/// - `DocumentId::Int(n)` → `"i:{n}"`
/// - `DocumentId::String(s)` → `"s:{s}"`
/// - `DocumentId::ObjectId(hex)` → `"o:{hex}"`
///
/// NOTE: This is distinct from `adapter.rs::doc_id_to_string()` which produces
/// unprefixed strings for MCP JSON responses (no roundtrip needed there).
pub(crate) fn doc_id_to_string(id: &DocumentId) -> String {
    match id {
        DocumentId::Int(i) => format!("i:{}", i),
        DocumentId::String(s) => format!("s:{}", s),
        DocumentId::ObjectId(oid) => format!("o:{}", oid),
    }
}

/// The unprefixed HNSW node id that live inserts used before audit 2026-10-06
/// #24. Only for removing such nodes from existing graphs; new nodes always
/// use `doc_id_to_string`.
pub(crate) fn legacy_hnsw_id(id: &DocumentId) -> String {
    match id {
        DocumentId::Int(i) => i.to_string(),
        DocumentId::String(s) => s.clone(),
        DocumentId::ObjectId(oid) => oid.clone(),
    }
}

/// Convert String back to DocumentId
///
/// Recognizes type-prefixed format (`"i:"`, `"s:"`, `"o:"`) for lossless roundtrip.
/// Falls back to heuristic guessing for backward compatibility with old .hnsw cache
/// files that stored unprefixed IDs.
fn string_to_doc_id(s: &str) -> DocumentId {
    // New prefixed format — lossless roundtrip
    if let Some(rest) = s.strip_prefix("i:") {
        if let Ok(i) = rest.parse::<i64>() {
            return DocumentId::Int(i);
        }
        // Malformed "i:" prefix — treat as string to avoid data loss
        return DocumentId::String(s.to_string());
    }
    if let Some(rest) = s.strip_prefix("s:") {
        return DocumentId::String(rest.to_string());
    }
    if let Some(rest) = s.strip_prefix("o:") {
        return DocumentId::ObjectId(rest.to_string());
    }

    // Backward compatibility fallback for old unprefixed HNSW cache entries.
    // This heuristic can misidentify DocumentId::String("12345") as Int(12345)
    // or DocumentId::String("aabbccdd11223344aabbccdd") as ObjectId, but this
    // only affects legacy cache data. New inserts use the prefixed format above,
    // so these entries will be replaced on the next HNSW rebuild.
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
        // Register the index BEFORE the scan so writers maintain it while the
        // scan below fills it; built off to the side, every write made during
        // the build was lost from it (T2)
        {
            let mut hnsw = HnswIndex::new(config_with_field.clone());
            hnsw.begin_build();
            self.indexes
                .write()
                .add_loaded_vector_index(index_name.clone(), hnsw);
        }

        // Scan documents and build the index. Small batches: each one holds
        // the IndexManager write lock for its HNSW inserts.
        use crate::limits::FULLTEXT_INDEX_BUILD_BATCH_SIZE as INDEX_BUILD_BATCH_SIZE;
        const PROGRESS_LOG_INTERVAL: usize = 1000;
        let mut indexed_count: usize = 0;
        let mut total_scanned: usize = 0;
        let mut dimension_error_count: u64 = 0;
        let field_clone = field.to_string();
        let collection_name = self.name.clone();
        // Maximum number of dimension mismatch errors to log individually.
        // After this, only the total count is reported at the end.
        const MAX_DIM_ERROR_LOG: u64 = 5;

        let scan = self.scan_documents_in_batches(INDEX_BUILD_BATCH_SIZE, |_batch_num, batch_docs| {
            let mut indexes = self.indexes.write();
            let hnsw = indexes.get_vector_index_mut(&index_name).ok_or_else(|| {
                IronBaseError::IndexError(format!("Vector index '{}' not found", index_name))
            })?;
            for (doc_id, doc) in batch_docs {
                let id_str = doc_id_to_string(&doc_id);
                // Get the vector field value
                if let Some(vec_value) = get_nested_value(&doc, &field_clone) {
                    if let Some(vector) = Self::extract_f32_vector(vec_value) {
                        // Check dimension
                        if vector.len() != config.dim {
                            dimension_error_count += 1;
                            if dimension_error_count <= MAX_DIM_ERROR_LOG {
                                tracing::warn!(
                                    doc_id = %id_str,
                                    expected = config.dim,
                                    actual = vector.len(),
                                    "Vector dimension mismatch, skipping"
                                );
                                if dimension_error_count == MAX_DIM_ERROR_LOG {
                                    tracing::warn!(
                                        "Suppressing further dimension mismatch warnings (limit: {})",
                                        MAX_DIM_ERROR_LOG
                                    );
                                }
                            }
                        } else {
                            // Swallow (log-and-skip), uniform with the lazy
                            // rebuild-from-documents twin (load_or_rebuild_hnsw)
                            // and with add_document_to_indexes: a build hitting the
                            // RAM-derived ceiling produces a (logged) partial index
                            // rather than failing the whole build. P0-2's ceiling
                            // makes a genuine hit reachable only at true RAM
                            // exhaustion.
                            if let Err(e) = hnsw.build_insert(&id_str, &vector) {
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
        });
        if let Err(e) = scan {
            // A failed build leaves no half-populated index behind
            let _ = self.indexes.write().drop_vector_index(&index_name);
            return Err(e);
        }

        // Writers keep maintaining the registered index from here on; the
        // cache file below is a snapshot, later changes are flushed by the
        // checkpoint (the IndexManager marked the index dirty).
        let (bytes, vector_count) = {
            let mut indexes = self.indexes.write();
            let hnsw = indexes.get_vector_index_mut(&index_name).ok_or_else(|| {
                IronBaseError::IndexError(format!("Vector index '{}' not found", index_name))
            })?;
            hnsw.end_build();
            let bytes = hnsw.to_bytes();
            hnsw.mark_clean();
            (bytes, hnsw.len())
        };
        // Until its metadata is persisted, a failure leaves no index behind
        let drop_registered = |e: IronBaseError| {
            let _ = self.indexes.write().drop_vector_index(&index_name);
            e
        };
        let bytes = bytes.map_err(drop_registered)?;

        tracing::info!(
            collection = %self.name,
            total_scanned = total_scanned,
            indexed = indexed_count,
            dimension_errors = dimension_error_count,
            "Document scan complete, persisting index"
        );

        // Get database file path for cache file
        let db_file_path = {
            let storage = self.storage.read();
            storage.get_file_path().to_string()
        };
        let cache_path =
            crate::index::IndexManager::build_vector_cache_path(&db_file_path, &index_name);
        let cache_file_name = cache_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();

        // Save HNSW cache file if we have a file-based storage
        if !db_file_path.is_empty() {
            std::fs::write(&cache_path, &bytes)
                .map_err(|e| {
                    IronBaseError::Io(std::io::Error::other(format!(
                        "Failed to write HNSW cache file: {}",
                        e
                    )))
                })
                .map_err(drop_registered)?;
            tracing::info!(
                cache_path = %cache_path.display(),
                size_bytes = bytes.len(),
                "HNSW cache file saved"
            );
        }

        // Create metadata
        let metadata = VectorIndexMetadata {
            name: index_name.clone(),
            field: field.to_string(),
            config: config_with_field,
            cache_file: cache_file_name,
            vector_count,
            created_at: chrono::Utc::now().to_rfc3339(),
        };

        // Persist metadata
        {
            let mut storage = self.storage.write();
            if let Some(meta) = storage.get_collection_meta_mut(&self.name) {
                meta.vector_indexes.push(metadata);
                storage.persist_metadata()?;
            }
        }

        tracing::info!(
            collection = %self.name,
            index_name = %index_name,
            vector_count = vector_count,
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
                    storage.persist_metadata()?;
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

        // Delete cache file if it exists: the current name, and the name
        // stored when the index was created (`{index}.hnsw` before P10, which
        // is this database's only when no other database shares the directory)
        if let Some(cache_file_name) = cache_file {
            let db_file_path = {
                let storage = self.storage.read();
                storage.get_file_path().to_string()
            };

            if !db_file_path.is_empty() {
                let current =
                    crate::index::IndexManager::build_vector_cache_path(&db_file_path, index_name);
                let stored = Self::build_hnsw_cache_path(&db_file_path, &cache_file_name);
                let mut cache_paths = vec![current.clone()];
                if stored != current
                    && super::is_only_database_in_dir(std::path::Path::new(&db_file_path))
                {
                    cache_paths.push(stored);
                }
                for cache_path in cache_paths.into_iter().filter(|p| p.exists()) {
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
    ///
    /// The returned `vector_count` reflects the actual in-memory HNSW index size
    /// (if loaded), not the potentially stale metadata value.
    pub fn list_vector_indexes(&self) -> Result<Vec<VectorIndexMetadata>> {
        self.check_not_closed()?;

        let storage = self.storage.read();
        if let Some(meta) = storage.get_collection_meta(&self.name) {
            let mut indexes = meta.vector_indexes.clone();
            // Update vector_count from in-memory HNSW if loaded
            let idx_manager = self.indexes.read();
            for idx_meta in &mut indexes {
                if let Some(hnsw) = idx_manager.get_vector_index(&idx_meta.name) {
                    idx_meta.vector_count = hnsw.len();
                }
            }
            Ok(indexes)
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
        self.vector_search_with_ctx(field, query_vector, limit, None)
    }

    /// Vector similarity search with execution context for cancellation support.
    ///
    /// This is the cancellation-aware version of `vector_search`: `ctx` is
    /// checked before the search and while the result documents are loaded
    /// (hybrid review T7).
    pub fn vector_search_with_ctx(
        &self,
        field: &str,
        query_vector: &[f32],
        limit: usize,
        ctx: Option<&ExecutionContext>,
    ) -> Result<Vec<(Value, f32)>> {
        self.vector_search_with_options(
            field,
            query_vector,
            limit,
            &VectorSearchOptions::default(),
            ctx,
        )
    }

    /// Vector similarity search with options: an optional filter and fields
    /// left out of the returned documents (see [`VectorSearchOptions`]), and
    /// an execution context for cancellation support.
    pub fn vector_search_with_options(
        &self,
        field: &str,
        query_vector: &[f32],
        limit: usize,
        options: &VectorSearchOptions,
        ctx: Option<&ExecutionContext>,
    ) -> Result<Vec<(Value, f32)>> {
        let skip: HashSet<String> = options.exclude_fields.iter().cloned().collect();
        match &options.filter {
            Some(filter) => {
                self.filtered_vector_search(field, query_vector, filter, limit, &skip, ctx)
            }
            None => self.unfiltered_vector_search(field, query_vector, limit, &skip, ctx),
        }
    }

    fn unfiltered_vector_search(
        &self,
        field: &str,
        query_vector: &[f32],
        limit: usize,
        skip: &HashSet<String>,
        ctx: Option<&ExecutionContext>,
    ) -> Result<Vec<(Value, f32)>> {
        // Batch mode: see the inserts acknowledged before this call
        self.flush_pending_writes()?;
        self.check_not_closed()?;
        if let Some(exec_ctx) = ctx {
            exec_ctx.check_cancelled()?;
        }

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
                return self.load_documents_for_results(results, skip, ctx);
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
        self.load_documents_for_results(results, skip, ctx)
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
        self.vector_search_with_filter_with_ctx(field, query_vector, filter, limit, None)
    }

    /// Filtered vector search with execution context for cancellation support.
    ///
    /// This is the cancellation-aware version of `vector_search_with_filter`:
    /// `ctx` is checked while the filter is resolved by a scan, while the
    /// allowed documents are scored exactly and while the result documents
    /// are loaded (hybrid review T7).
    pub fn vector_search_with_filter_with_ctx(
        &self,
        field: &str,
        query_vector: &[f32],
        filter: &Value,
        limit: usize,
        ctx: Option<&ExecutionContext>,
    ) -> Result<Vec<(Value, f32)>> {
        let options = VectorSearchOptions {
            filter: Some(filter.clone()),
            ..Default::default()
        };
        self.vector_search_with_options(field, query_vector, limit, &options, ctx)
    }

    fn filtered_vector_search(
        &self,
        field: &str,
        query_vector: &[f32],
        filter: &Value,
        limit: usize,
        skip: &HashSet<String>,
        ctx: Option<&ExecutionContext>,
    ) -> Result<Vec<(Value, f32)>> {
        // Batch mode: see the inserts acknowledged before this call
        self.flush_pending_writes()?;
        self.check_not_closed()?;
        if let Some(exec_ctx) = ctx {
            exec_ctx.check_cancelled()?;
        }

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
        // Fast path: try B-tree index lookup (avoids loading documents from .mlite)
        // Slow path fallback: find_with_options with projection
        let allowed_ids: std::collections::HashSet<String> = {
            if let Some(ids) = self.resolve_filter_to_ids(filter)? {
                ids
            } else {
                // Fallback (complex filters without index): stream the matches
                // and keep only their ids. No cap: a truncated set silently
                // dropped the matches past it (hybrid review H2); the set is
                // no larger than the index's own id map.
                let mut ids = std::collections::HashSet::new();
                let mut cursor = self.find_streaming_with_options(
                    filter,
                    ctx.and_then(ExecutionContext::cancel_flag),
                    ctx.and_then(ExecutionContext::deadline),
                )?;
                let mut iteration = 0;
                while let Some(doc) = cursor.next()? {
                    // The scan reads documents lazily: check here too
                    if let Some(exec_ctx) = ctx {
                        exec_ctx.maybe_check(iteration)?;
                    }
                    iteration += 1;
                    if let Some(did) = doc
                        .get("_id")
                        .and_then(|id| serde_json::from_value::<DocumentId>(id.clone()).ok())
                    {
                        ids.insert(doc_id_to_string(&did));
                    }
                }
                ids
            }
        };

        if allowed_ids.is_empty() {
            return Ok(Vec::new());
        }

        // Try to get from IndexManager first (fast path)
        {
            let indexes = self.indexes.read();
            if let Some(hnsw) = indexes.get_vector_index(&index_name) {
                let results = hnsw.search_with_allowed(query_vector, limit, &allowed_ids, ctx)?;
                // Need to release lock before loading documents
                drop(indexes);
                return self.load_documents_for_results(results, skip, ctx);
            }
        }

        // Slow path: load from file and add to IndexManager
        let hnsw = self.load_hnsw_index_from_file(&index_meta)?;
        let results = hnsw.search_with_allowed(query_vector, limit, &allowed_ids, ctx);

        // Add to IndexManager for future queries (also when the search was
        // stopped: the load is done)
        {
            let mut indexes = self.indexes.write();
            indexes.add_loaded_vector_index(index_name, hnsw);
        }

        // Load documents for results
        self.load_documents_for_results(results?, skip, ctx)
    }

    // === Private helper methods ===

    /// Try to resolve a simple equality filter directly from B-tree index.
    ///
    /// Returns `Some(HashSet<doc_id>)` if the filter is a simple equality on a B-tree indexed field.
    /// Returns `None` for complex filters or fields without B-tree index → caller falls back to find_with_options.
    ///
    /// Supported filter forms:
    /// - `{"field": value}` where value is not an operator object
    /// - `{"field": {"$eq": value}}`
    ///
    /// Pattern: mod.rs:2557-2577 (QueryPlan::IndexScan equality execution via range_query)
    /// Memory: O(k) where k = matching doc count (uncapped, like the fallback: hybrid review H2)
    fn resolve_filter_to_ids(
        &self,
        filter: &Value,
    ) -> Result<Option<std::collections::HashSet<String>>> {
        let map = match filter.as_object() {
            Some(m) => m,
            None => return Ok(None),
        };

        // Only handle single-field equality filters
        // Multi-field or logical operators ($and, $or) → fallback
        if map.len() != 1 {
            return Ok(None);
        }

        let (field, value) = map.iter().next().unwrap();

        // Skip logical operators
        if field.starts_with('$') {
            return Ok(None);
        }

        // Extract equality value (pattern: query_planner.rs:600-622)
        let eq_value = if let Value::Object(ref val_map) = value {
            if val_map.len() == 1 {
                if let Some(eq_val) = val_map.get("$eq") {
                    eq_val
                } else if val_map.keys().any(|k| k.starts_with('$')) {
                    // Other operators ($gt, $in, etc.) → fallback
                    return Ok(None);
                } else {
                    value
                }
            } else if val_map.keys().any(|k| k.starts_with('$')) {
                return Ok(None);
            } else {
                value
            }
        } else {
            value
        };

        // Find B-tree index for this field (pattern: mod.rs:1218-1224)
        let indexes = self.indexes.read();
        let index_infos = indexes.list_indexes_with_compound_info();

        let index_info = match index_infos
            .iter()
            .find(|info| info.prefix_field == *field && !info.is_compound)
        {
            Some(info) => info,
            None => return Ok(None), // No index → fallback
        };

        let btree = match indexes.get_btree_index(&index_info.index_name) {
            Some(idx) => idx,
            None => return Ok(None),
        };

        // Convert value to IndexKey, respecting case_insensitive setting
        let key = if btree.metadata.case_insensitive {
            if let Value::String(s) = eq_value {
                IndexKey::String(s.to_lowercase())
            } else {
                IndexKey::from(eq_value.clone())
            }
        } else {
            IndexKey::from(eq_value.clone())
        };

        // Point lookup via range_query (pattern: mod.rs:2576-2577)
        let mode = RangeQueryMode::Scan {
            skip: 0,
            limit: None,
            order: ScanOrder::Asc,
        };
        let doc_ids = btree
            .range_query(&key, &key, true, true, mode)
            .unwrap_docs();

        let id_set: std::collections::HashSet<String> = doc_ids
            .into_iter()
            .map(|did| doc_id_to_string(&did))
            .collect();

        Ok(Some(id_set))
    }

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
                            if let Err(e) = hnsw.insert(&id_str, &vector) {
                                tracing::warn!(
                                    doc_id = %id_str,
                                    error = %e,
                                    "Failed to insert vector during HNSW rebuild, skipping"
                                );
                            }
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
        skip: &HashSet<String>,
        ctx: Option<&ExecutionContext>,
    ) -> Result<Vec<(Value, f32)>> {
        let mut docs_with_scores = Vec::with_capacity(results.len());

        for (iteration, result) in results.into_iter().enumerate() {
            // Check for cancellation every N iterations
            if let Some(exec_ctx) = ctx {
                exec_ctx.maybe_check(iteration)?;
            }

            // Parse the document ID
            let doc_id = string_to_doc_id(&result.id);

            // Load the document
            if let Some(doc) = self.read_document_by_id_skipping(&doc_id, skip)? {
                docs_with_scores.push((doc, result.score));
            }
        }

        Ok(docs_with_scores)
    }

    /// Build the path for HNSW cache file. Only the file-name component of
    /// the stored name is used, so metadata written before the fix cannot
    /// point outside the database directory either.
    fn build_hnsw_cache_path(db_file_path: &str, cache_file_name: &str) -> PathBuf {
        let db_path = PathBuf::from(db_file_path);
        let parent = db_path.parent().unwrap_or(&db_path);
        let file_name = std::path::Path::new(cache_file_name)
            .file_name()
            .unwrap_or_else(|| std::ffi::OsStr::new("index.hnsw"));
        parent.join(file_name)
    }

    /// Extract f32 vector from JSON value
    fn extract_f32_vector(value: &Value) -> Option<Vec<f32>> {
        match value {
            Value::Array(arr) => {
                let mut vector = Vec::with_capacity(arr.len());
                for v in arr {
                    match v {
                        Value::Number(n) => {
                            vector.push(n.as_f64()? as f32);
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

    #[test]
    fn test_resolve_filter_to_ids_with_btree_index() {
        let db = create_test_db();
        let coll = db.collection("test").unwrap();

        // Insert documents with category field
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

        // Create B-tree index on category
        coll.create_index("category".to_string(), false, false)
            .unwrap();

        // Create vector index
        let config = VectorIndexConfig::new(5);
        coll.create_vector_index("embedding", config).unwrap();

        // Test: filter with B-tree indexed field → fast path
        let query_vec: Vec<f32> = vec![0.0, 0.2, 0.4, 0.6, 0.8];
        let results = coll
            .vector_search_with_filter("embedding", &query_vec, &json!({"category": "even"}), 10)
            .unwrap();

        // All results should be from "even" category
        for (doc, _score) in &results {
            assert_eq!(doc.get("category").unwrap(), "even");
        }
        // Should have results (10 even docs out of 20)
        assert!(!results.is_empty());
    }

    #[test]
    fn test_resolve_filter_to_ids_fallback_for_complex_filter() {
        let db = create_test_db();
        let coll = db.collection("test").unwrap();

        for i in 0..10 {
            let vector: Vec<f64> = (0..3).map(|j| ((i + j) % 3) as f64 / 3.0).collect();
            db.insert_one(
                "test",
                json_to_hashmap(json!({
                    "name": format!("doc{}", i),
                    "score": i * 10,
                    "embedding": vector
                })),
            )
            .unwrap();
        }

        let config = VectorIndexConfig::new(3);
        coll.create_vector_index("embedding", config).unwrap();

        // $gt filter → no B-tree fast path, falls back to find_with_options
        let query_vec: Vec<f32> = vec![0.1, 0.2, 0.3];
        let results = coll
            .vector_search_with_filter("embedding", &query_vec, &json!({"score": {"$gt": 50}}), 10)
            .unwrap();

        // All results should have score > 50
        for (doc, _score) in &results {
            let s = doc.get("score").unwrap().as_i64().unwrap();
            assert!(s > 50);
        }
    }

    #[test]
    fn test_resolve_filter_to_ids_eq_operator() {
        let db = create_test_db();
        let coll = db.collection("test").unwrap();

        for i in 0..10 {
            let vector: Vec<f64> = (0..3).map(|j| ((i + j) % 3) as f64 / 3.0).collect();
            db.insert_one(
                "test",
                json_to_hashmap(json!({
                    "name": format!("doc{}", i),
                    "category": if i % 3 == 0 { "a" } else if i % 3 == 1 { "b" } else { "c" },
                    "embedding": vector
                })),
            )
            .unwrap();
        }

        // Create B-tree index on category
        coll.create_index("category".to_string(), false, false)
            .unwrap();

        let config = VectorIndexConfig::new(3);
        coll.create_vector_index("embedding", config).unwrap();

        // {"category": {"$eq": "a"}} should also use fast path
        let query_vec: Vec<f32> = vec![0.1, 0.2, 0.3];
        let results = coll
            .vector_search_with_filter(
                "embedding",
                &query_vec,
                &json!({"category": {"$eq": "a"}}),
                10,
            )
            .unwrap();

        for (doc, _score) in &results {
            assert_eq!(doc.get("category").unwrap(), "a");
        }
        assert!(!results.is_empty());
    }

    /// Hybrid review H2: a selective filter must still return the true
    /// nearest matching documents. The filter used to run only on the
    /// `ef_search * 3` nearest candidates, so with 1% of documents matching
    /// most of the matching neighbours were never seen.
    #[test]
    fn test_vector_search_with_selective_filter_is_exact() {
        let mut seed = 42u64;
        let mut next = || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((seed >> 33) as f32 / (1u64 << 31) as f32) - 0.5
        };
        for with_btree in [false, true] {
            let db = create_test_db();
            let coll = db.collection("test").unwrap();
            let mut acme: Vec<(String, Vec<f32>)> = Vec::new();
            for i in 0..2000 {
                let vector: Vec<f32> = (0..16).map(|_| next()).collect();
                let customer = if i % 100 == 0 { "acme" } else { "other" };
                let name = format!("doc{}", i);
                if customer == "acme" {
                    acme.push((name.clone(), vector.clone()));
                }
                db.insert_one(
                    "test",
                    json_to_hashmap(json!({
                        "name": name, "customer": customer, "embedding": vector
                    })),
                )
                .unwrap();
            }
            if with_btree {
                coll.create_index("customer".to_string(), false, false)
                    .unwrap();
            }
            coll.create_vector_index("embedding", VectorIndexConfig::new(16))
                .unwrap();

            let query: Vec<f32> = (0..16).map(|_| next()).collect();
            acme.sort_by(|a, b| {
                let sa = crate::vector::simd::cosine_similarity(&query, &a.1);
                let sb = crate::vector::simd::cosine_similarity(&query, &b.1);
                sb.total_cmp(&sa)
            });
            let expected: Vec<&str> = acme.iter().take(10).map(|(n, _)| n.as_str()).collect();

            let results = coll
                .vector_search_with_filter("embedding", &query, &json!({"customer": "acme"}), 10)
                .unwrap();
            let got: Vec<&str> = results
                .iter()
                .map(|(doc, _)| doc["name"].as_str().unwrap())
                .collect();
            assert_eq!(got, expected, "with_btree={}", with_btree);
        }
    }

    /// T2: skipped fields are not parsed; identity and deletion fields are
    /// kept even when listed; the rest of the document is unchanged
    #[test]
    fn test_parse_document_skipping() {
        let bytes = br#"{"_id":3,"_collection":"c","embedding":[1.0,2.0],"meta":{"embedding":1},"content":"x"}"#;
        let skip: HashSet<String> = ["embedding", "_id", "_collection"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let doc = crate::collection_core::parse_document_skipping(bytes, &skip).unwrap();
        assert_eq!(
            doc,
            json!({"_id":3,"_collection":"c","meta":{"embedding":1},"content":"x"})
        );
        assert!(crate::collection_core::parse_document_skipping(b"{} x", &skip).is_err());
    }

    /// 6 chunks with a 3-dimensional "embedding", a "tag" and fulltext "content"
    fn create_t2_db() -> DatabaseCore<MemoryStorage> {
        let db = create_test_db();
        for i in 0..6 {
            db.insert_one(
                "kb",
                json_to_hashmap(json!({
                    "content": format!("maintenance note {i}"),
                    "tag": i % 2,
                    "embedding": [1.0, i as f64, 0.5]
                })),
            )
            .unwrap();
        }
        let coll = db.collection("kb").unwrap();
        coll.create_vector_index("embedding", VectorIndexConfig::new(3))
            .unwrap();
        coll.create_fulltext_index("content".to_string(), "english", None, None)
            .unwrap();
        db
    }

    /// T2: `exclude_fields` leaves the field out of the returned documents;
    /// the results are otherwise those of a plain search
    #[test]
    fn test_vector_search_with_options_excludes_fields() {
        let db = create_t2_db();
        let coll = db.collection("kb").unwrap();
        let query = [1.0, 2.0, 0.5];

        let plain = coll.vector_search("embedding", &query, 4).unwrap();
        let options = VectorSearchOptions {
            exclude_fields: vec!["embedding".to_string()],
            ..Default::default()
        };
        let lean = coll
            .vector_search_with_options("embedding", &query, 4, &options, None)
            .unwrap();
        assert_eq!(lean.len(), plain.len());
        for ((lean_doc, lean_score), (doc, score)) in lean.iter().zip(&plain) {
            let mut expected = doc.clone();
            expected.as_object_mut().unwrap().remove("embedding");
            assert_eq!(lean_doc, &expected);
            assert_eq!(lean_score, score);
        }

        let filtered = VectorSearchOptions {
            filter: Some(json!({"tag": 1})),
            exclude_fields: vec!["embedding".to_string()],
        };
        let results = coll
            .vector_search_with_options("embedding", &query, 4, &filtered, None)
            .unwrap();
        assert_eq!(results.len(), 3);
        assert!(results
            .iter()
            .all(|(doc, _)| doc["tag"] == json!(1) && doc.get("embedding").is_none()));
    }

    /// T2: a fulltext exclusion projection is applied while parsing, but a
    /// field the filter reads is still parsed for the filter
    #[test]
    fn test_fulltext_projection_skips_while_parsing() {
        let db = create_t2_db();
        let coll = db.collection("kb").unwrap();
        let options = crate::fulltext::FulltextSearchOptions::new()
            .with_limit(10)
            .with_filter(json!({"tag": 1}))
            .with_projection(
                [("embedding".to_string(), 0), ("tag".to_string(), 0)]
                    .into_iter()
                    .collect(),
            );
        let results = coll
            .fulltext_search_ext("content", "maintenance", options)
            .unwrap();
        assert_eq!(
            results.len(),
            3,
            "the filter on the projected-out tag still applies"
        );
        for r in &results {
            assert!(r.document.get("embedding").is_none());
            assert!(r.document.get("tag").is_none());
            assert!(r.document.get("content").is_some());
        }
    }

    /// T2: fields the filter or the search reads are never skipped
    #[test]
    fn test_projection_parse_skip_keeps_read_fields() {
        use crate::collection_core::search::projection_parse_skip;
        let proj: HashMap<String, i32> = [("embedding", 0), ("tag", 0), ("content", 0)]
            .iter()
            .map(|(k, v)| (k.to_string(), *v))
            .collect();
        let filter = json!({"$or": [{"tag.x": 1}, {"other": 2}]});
        let skip = projection_parse_skip(Some(&proj), &["content"], Some(&filter));
        assert_eq!(skip, ["embedding".to_string()].into_iter().collect());
        assert!(
            projection_parse_skip(Some(&proj), &["content"], Some(&json!({"$expr": {}})))
                .is_empty()
        );
        let include: HashMap<String, i32> = [("content".to_string(), 1)].into_iter().collect();
        assert!(projection_parse_skip(Some(&include), &["content"], None).is_empty());
    }
}
