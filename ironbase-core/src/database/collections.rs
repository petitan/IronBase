//! Collection, Index, and Schema Management
//!
//! This module handles the lifecycle of collections and their associated resources:
//! IndexManagers, schemas, and the warm-up process on database open.
//!
//! # Responsibilities
//!
//! - **Collection Access**: `collection()`, `get_collection()`, `collection_exists()`
//! - **IndexManager Lifecycle**: Per-collection shared IndexManager with double-checked locking
//! - **Schema Management**: Per-collection compiled schema with shared Arc
//! - **Warm-up**: Loading indexes from `.idx`/`.ftidx`/`.fzidx` files on startup
//! - **Index Rebuild**: Rebuilding indexes from document catalog when needed
//!
//! # Startup Flow (Warm-up)
//!
//! ```text
//! DatabaseCore::open()
//!     │
//!     ▼
//! collection() called
//!     │
//!     ▼
//! get_or_create_index_manager()
//!     │
//!     ▼
//! initialize_index_manager()
//!     ├── Load collection metadata from storage
//!     ├── Create _id index (always)
//!     ├── Load B+ tree indexes from .idx files
//!     ├── Load fulltext indexes from .ftidx files
//!     ├── Load fuzzy indexes from .fzidx files
//!     │
//!     ▼
//! Check: was_clean && all indexes loaded from files?
//!     ├── YES → Skip rebuild (FAST PATH: <1s)
//!     └── NO  → rebuild_indexes_from_catalog() (SLOW PATH: ~100s for 70K docs)
//! ```
//!
//! # Thread Safety
//!
//! - **IndexManager**: Shared via `Arc<RwLock<IndexManager>>` per collection
//! - **Schema**: Shared via `Arc<RwLock<Option<CompiledSchema>>>` per collection
//! - **Collection Write Lock**: `Arc<Mutex<()>>` for Safe mode atomicity
//!
//! # Double-Checked Locking Pattern
//!
//! Used for lazy initialization of per-collection resources:
//! 1. Fast path: Read lock to check if exists → return if found
//! 2. Slow path: Write lock, check again, create if needed
//!
//! This minimizes lock contention while ensuring thread-safe creation.

use parking_lot::{Mutex, RwLock};
use std::sync::Arc;

use serde_json::Value;

use crate::collection_core::{schema::CompiledSchema, CollectionCore};
use crate::document::DocumentId;
use crate::error::{IronBaseError, Result};
use crate::index::{IndexKey, IndexManager};
use crate::storage::{RawStorage, Storage};

use super::DatabaseCore;

// ============================================================================
// COLLECTION AND INDEX MANAGEMENT (Generic Implementation)
// ============================================================================

impl<S: Storage + RawStorage> DatabaseCore<S> {
    // ========== IndexManager Management ==========

    /// Get or create a shared IndexManager for a collection
    ///
    /// This method uses double-checked locking to ensure thread-safe creation
    /// of IndexManagers while minimizing lock contention.
    pub(crate) fn get_or_create_index_manager(
        &self,
        name: &str,
    ) -> Result<Arc<RwLock<IndexManager>>> {
        // Fast path: read lock to check if already exists
        {
            let managers = self.index_managers.read();
            if let Some(manager) = managers.get(name) {
                return Ok(Arc::clone(manager));
            }
        }

        // Slow path: create with write lock (double-checked)
        let mut managers = self.index_managers.write();
        if let Some(manager) = managers.get(name) {
            return Ok(Arc::clone(manager));
        }

        // Initialize the IndexManager for this collection
        let index_manager = self.initialize_index_manager(name)?;
        let shared = Arc::new(RwLock::new(index_manager));
        managers.insert(name.to_string(), Arc::clone(&shared));
        Ok(shared)
    }

    /// Get or create a collection-level write lock for Safe mode atomicity
    ///
    /// This lock ensures that the prepare-WAL-persist sequence is atomic,
    /// preventing race conditions in unique constraint checks.
    pub(crate) fn get_collection_write_lock(&self, name: &str) -> Arc<Mutex<()>> {
        // Fast path: read lock to check if already exists
        {
            let locks = self.collection_write_locks.read();
            if let Some(lock) = locks.get(name) {
                return Arc::clone(lock);
            }
        }

        // Slow path: create with write lock (double-checked)
        let mut locks = self.collection_write_locks.write();
        if let Some(lock) = locks.get(name) {
            return Arc::clone(lock);
        }

        let lock = Arc::new(Mutex::new(()));
        locks.insert(name.to_string(), Arc::clone(&lock));
        lock
    }

    /// Check if a collection has any unique indexes (excluding _id which is always unique)
    ///
    /// Used for hybrid locking optimization:
    /// - Collections WITH unique indexes use collection-level lock (serialize all ops)
    /// - Collections WITHOUT unique indexes skip the lock (faster, no constraint races)
    pub(crate) fn collection_has_unique_index(&self, name: &str) -> bool {
        let managers = self.index_managers.read();
        if let Some(manager) = managers.get(name) {
            let idx_manager = manager.read();
            idx_manager.has_unique_index()
        } else {
            false
        }
    }

    /// Get or create a shared schema manager for a collection
    ///
    /// Similar to `get_or_create_index_manager`, this ensures all CollectionCore
    /// instances share the same schema Arc, so schema changes propagate correctly.
    pub(crate) fn get_or_create_schema_manager(
        &self,
        name: &str,
    ) -> Result<Arc<RwLock<Option<CompiledSchema>>>> {
        // Fast path: read lock to check if already exists
        {
            let managers = self.schema_managers.read();
            if let Some(manager) = managers.get(name) {
                return Ok(Arc::clone(manager));
            }
        }

        // Slow path: create with write lock (double-checked)
        let mut managers = self.schema_managers.write();
        if let Some(manager) = managers.get(name) {
            return Ok(Arc::clone(manager));
        }

        // Load schema from storage metadata
        let compiled_schema = {
            let storage = self.storage.read();
            match storage.get_collection_meta(name) {
                Some(meta) => {
                    if let Some(raw_schema) = &meta.schema {
                        Some(CompiledSchema::from_value(raw_schema)?)
                    } else {
                        None
                    }
                }
                None => None,
            }
        };

        let shared = Arc::new(RwLock::new(compiled_schema));
        managers.insert(name.to_string(), Arc::clone(&shared));
        Ok(shared)
    }

    /// Load persisted custom indexes from metadata/files
    ///
    /// IMPORTANT: Only loads from .idx files when `was_clean == true`.
    /// On dirty shutdown, stale .idx files may contain entries for tombstoned documents,
    /// so we create empty indexes that will be rebuilt from the catalog.
    /// Load persisted indexes from .idx files or create empty ones.
    /// Returns the set of index names that were successfully loaded from files.
    fn load_persisted_indexes(
        index_manager: &mut IndexManager,
        persisted_indexes: &[crate::index::IndexMetadata],
        id_index_name: &str,
        db_path: &str,
        was_clean: bool,
    ) -> Result<std::collections::HashSet<String>> {
        use crate::log_debug;
        let mut loaded_from_file = std::collections::HashSet::new();

        for index_meta in persisted_indexes {
            // Skip _id index (already created)
            if index_meta.name == id_index_name {
                continue;
            }

            // Only try to load from .idx file if clean shutdown
            // Dirty shutdown may have stale index entries for tombstoned documents
            if was_clean {
                if let Some(loaded_tree) =
                    crate::collection_core::try_load_index_from_file(db_path, index_meta)
                {
                    log_debug!("Loaded index '{}' from .idx file", index_meta.name);
                    loaded_from_file.insert(index_meta.name.clone());
                    index_manager.add_loaded_index(loaded_tree);
                    continue;
                }
            }

            // Create empty index (will be rebuilt from documents if needed)
            log_debug!(
                "Creating empty index '{}' on field '{}' (will rebuild from documents)",
                index_meta.name,
                index_meta.field
            );
            index_manager.create_btree_index(
                index_meta.name.clone(),
                index_meta.field.clone(),
                index_meta.unique,
                index_meta.sparse,
            )?;
        }
        Ok(loaded_from_file)
    }

    /// Rebuild all indexes from document catalog
    ///
    /// OPTIMIZATION: Skips B+ tree indexes that already have data (loaded from .idx files)
    /// This allows fast startup even when fuzzy indexes need rebuild.
    fn rebuild_indexes_from_catalog<S2: Storage + RawStorage>(
        index_manager: &mut IndexManager,
        storage: &mut S2,
        collection_name: &str,
        catalog: &std::collections::HashMap<DocumentId, u64>,
        persisted_indexes: &[crate::index::IndexMetadata],
        id_index_name: &str,
    ) -> Result<u64> {
        use crate::{log_debug, log_warn};

        // Batch size for memory-efficient rebuilding
        // Uses smaller batch for safety during warm-up
        use crate::limits::INDEX_REBUILD_BATCH_SIZE as REBUILD_BATCH_SIZE;

        let mut rebuilt_count = 0u64;

        // Check if _id index needs rebuild (empty = not loaded from file)
        let rebuild_id_index = index_manager
            .get_btree_index(id_index_name)
            .map(|idx| idx.size() == 0)
            .unwrap_or(true);

        // Pre-collect B+ tree indexes that need rebuild (empty = not loaded from file)
        let btree_indexes_to_rebuild: Vec<_> = persisted_indexes
            .iter()
            .filter(|meta| meta.name != id_index_name)
            .filter(|meta| {
                index_manager
                    .get_btree_index(&meta.name)
                    .map(|idx| idx.size() == 0) // Only rebuild empty indexes
                    .unwrap_or(true)
            })
            .cloned()
            .collect();

        // Pre-collect fuzzy index info to avoid repeated lookups in the loop
        // Fuzzy indexes ALWAYS need rebuild (no file persistence yet)
        let fuzzy_info: Vec<_> = index_manager
            .list_fuzzy_indexes()
            .iter()
            .map(|idx| (idx.metadata.name.clone(), idx.metadata.field.clone()))
            .collect();

        // Pre-collect fulltext index info to avoid repeated lookups in the loop
        // SKIP indexes that already have data (loaded from .ftidx file)
        let fulltext_info: Vec<_> = index_manager
            .list_fulltext_indexes()
            .iter()
            .filter(|idx| idx.doc_count() == 0) // Only rebuild empty indexes
            .map(|idx| (idx.name.clone(), idx.field.clone()))
            .collect();

        // Pre-collect vector index info that need rebuilding
        // SKIP indexes that already have data (loaded from .hnsw file)
        let vector_indexes_to_rebuild: Vec<(String, String, usize)> = {
            let prefix = format!("{}_vec_", collection_name);
            index_manager
                .list_indexes()
                .into_iter()
                .filter_map(|name| {
                    let idx = index_manager.get_vector_index(&name)?;
                    if !idx.is_empty() {
                        return None; // Skip non-empty indexes
                    }
                    // Extract field from index name: {collection}_vec_{field}
                    let field = name.strip_prefix(&prefix)?.to_string();
                    Some((name, field, idx.config().dim))
                })
                .collect()
        };

        // Sort offsets for sequential disk reads (optimization)
        let mut sorted_entries: Vec<_> = catalog.iter().collect();
        sorted_entries.sort_by_key(|(_, offset)| *offset);

        let total_docs = sorted_entries.len();
        let total_batches = total_docs.div_ceil(REBUILD_BATCH_SIZE);
        let log_interval = std::cmp::max(1, total_batches / 10); // Log every 10%

        // Process documents in batches to prevent OOM
        for (batch_num, batch) in sorted_entries.chunks(REBUILD_BATCH_SIZE).enumerate() {
            // Progress logging every 10% (or at least every batch for small collections)
            if batch_num % log_interval == 0 || batch_num == total_batches - 1 {
                let progress = (batch_num + 1) * 100 / total_batches;
                let docs_processed =
                    std::cmp::min((batch_num + 1) * REBUILD_BATCH_SIZE, total_docs);
                log_debug!(
                    "Index rebuild progress: {}/{} batches ({}%), {}/{} docs",
                    batch_num + 1,
                    total_batches,
                    progress,
                    docs_processed,
                    total_docs
                );
            }

            // Read and parse documents for this batch
            let mut batch_docs: Vec<(DocumentId, Value)> = Vec::with_capacity(batch.len());

            for (_id_key, offset) in batch {
                // Read document from disk
                let doc_bytes = match storage.read_document_at(collection_name, **offset) {
                    Ok(bytes) => bytes,
                    Err(e) => {
                        log_warn!(
                            "Failed to read document at offset during index rebuild: {:?}",
                            e
                        );
                        continue;
                    }
                };

                // Parse JSON
                let doc: Value = match serde_json::from_slice(&doc_bytes) {
                    Ok(d) => d,
                    Err(e) => {
                        log_warn!(
                            "Failed to parse document JSON during index rebuild: {:?}",
                            e
                        );
                        continue;
                    }
                };

                // Skip tombstones
                if doc
                    .get("_tombstone")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                {
                    continue;
                }

                // Extract _id
                let Some(id_value) = doc.get("_id") else {
                    continue;
                };
                let Ok(doc_id) = serde_json::from_value::<DocumentId>(id_value.clone()) else {
                    continue;
                };

                batch_docs.push((doc_id, doc));
            }

            // Process batch: rebuild all indexes
            for (doc_id, doc) in &batch_docs {
                let id_value = doc.get("_id").unwrap(); // Safe: we extracted _id above

                // Rebuild _id index ONLY if needed (not loaded from .idx file)
                if rebuild_id_index {
                    let index_key = IndexKey::from(id_value);
                    if let Some(id_index) = index_manager.get_btree_index_mut(id_index_name) {
                        if let Err(e) = id_index.insert(index_key, doc_id.clone()) {
                            log_warn!(
                                "Index rebuild: duplicate _id key ignored for {}: {:?}",
                                collection_name,
                                e
                            );
                        }
                    }
                }

                // Rebuild custom B+ tree indexes ONLY if needed (not loaded from .idx file)
                for index_meta in &btree_indexes_to_rebuild {
                    let Some(index) = index_manager.get_btree_index_mut(&index_meta.name) else {
                        continue;
                    };

                    let keys = index.extract_keys(doc);
                    let mut seen = std::collections::HashSet::new();
                    for key in keys {
                        if !seen.insert(key.clone()) {
                            continue;
                        }
                        let is_all_null = IndexManager::is_key_all_null(&key);

                        // Include null keys for unique indexes, skip for non-unique
                        if !is_all_null || index.metadata.unique {
                            if let Err(e) = index.insert(key, doc_id.clone()) {
                                log_warn!(
                                    "Index rebuild: duplicate key ignored for index {} in {}: {:?}",
                                    index_meta.name,
                                    collection_name,
                                    e
                                );
                            }
                            rebuilt_count += 1;
                        }
                    }
                }

                // Rebuild fuzzy indexes (using pre-collected info from outside the loop)
                for (index_name, field) in &fuzzy_info {
                    if let Some(value) = crate::value_utils::get_nested_value(doc, field) {
                        if let Some(s) = value.as_str() {
                            if let Some(index) = index_manager.get_fuzzy_index_mut(index_name) {
                                index.insert(s, doc_id.clone());
                                rebuilt_count += 1;
                            }
                        }
                    }
                }

                // Rebuild fulltext indexes (using pre-collected info from outside the loop)
                for (index_name, field) in &fulltext_info {
                    if let Some(value) = crate::value_utils::get_nested_value(doc, field) {
                        if let Some(s) = value.as_str() {
                            if let Some(index) = index_manager.get_fulltext_index_mut(index_name) {
                                let _ = index.insert(doc_id, s);
                                rebuilt_count += 1;
                            }
                        }
                    }
                }

                // Rebuild vector indexes (using pre-collected info from outside the loop)
                for (index_name, field, dim) in &vector_indexes_to_rebuild {
                    if let Some(value) = crate::value_utils::get_nested_value(doc, field) {
                        // Extract f32 vector from JSON array
                        if let Some(arr) = value.as_array() {
                            let mut vector = Vec::with_capacity(arr.len());
                            let mut valid = true;
                            for v in arr {
                                if let Some(f) = v.as_f64() {
                                    vector.push(f as f32);
                                } else {
                                    valid = false;
                                    break;
                                }
                            }
                            if valid && vector.len() == *dim {
                                let id_str = match doc_id {
                                    DocumentId::Int(i) => i.to_string(),
                                    DocumentId::String(s) => s.clone(),
                                    DocumentId::ObjectId(oid) => oid.clone(),
                                };
                                if let Some(index) = index_manager.get_vector_index_mut(index_name)
                                {
                                    if index.insert(&id_str, &vector).is_ok() {
                                        rebuilt_count += 1;
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Log progress for large collections
            if total_docs > 1000 && (batch_num + 1) % 10 == 0 {
                let processed = (batch_num + 1) * REBUILD_BATCH_SIZE;
                log_debug!(
                    "Index rebuild progress: {}/{} docs ({}%)",
                    processed.min(total_docs),
                    total_docs,
                    (processed * 100 / total_docs).min(100)
                );
            }

            // Drop batch_docs to free memory before next batch
            drop(batch_docs);
        }

        Ok(rebuilt_count)
    }

    /// Initialize an IndexManager for a collection (internal)
    ///
    /// This creates the _id index, loads persisted indexes, and rebuilds
    /// all indexes from the document catalog.
    fn initialize_index_manager(&self, name: &str) -> Result<IndexManager> {
        use crate::{log_debug, log_warn};

        let mut index_manager = IndexManager::new();
        let id_index_name = format!("{}_id", name);

        // Ensure collection exists before loading metadata
        {
            let mut storage_guard = self.storage.write();
            if storage_guard.get_collection_meta(name).is_none() {
                storage_guard.create_collection(name)?;
            }
        }

        // Load persisted indexes and schema
        let storage_guard = self.storage.write();
        let meta = storage_guard
            .get_collection_meta(name)
            .ok_or_else(|| crate::error::IronBaseError::CollectionNotFound(name.to_string()))?;

        let catalog = meta.document_catalog.clone();
        let persisted_indexes = meta.indexes.clone();
        let persisted_fuzzy_indexes = meta.fuzzy_indexes.clone();
        let persisted_fulltext_indexes = meta.fulltext_indexes.clone();
        let persisted_vector_indexes = meta.vector_indexes.clone();

        log_debug!(
            "Collection '{}' - catalog size: {}, persisted indexes: {}, fuzzy indexes: {}, fulltext indexes: {}, vector indexes: {}",
            name,
            catalog.len(),
            persisted_indexes.len(),
            persisted_fuzzy_indexes.len(),
            persisted_fulltext_indexes.len(),
            persisted_vector_indexes.len()
        );

        // Get db_path for .idx file loading
        let db_path = storage_guard.get_file_path().to_string();
        let was_clean = storage_guard.was_clean_shutdown();

        drop(storage_guard); // Release write lock before rebuilding

        // Try to load _id index from .idx file if clean shutdown
        let id_index_loaded = if was_clean && !catalog.is_empty() {
            // Create a fake metadata for _id index loading
            let id_meta = crate::index::IndexMetadata {
                name: id_index_name.clone(),
                field: "_id".to_string(),
                fields: vec!["_id".to_string()],
                unique: true,
                sparse: false,
                multikey: false,
                case_insensitive: false,
                num_keys: 0, // Will be synced on load
                tree_height: 1,
                root_offset: 0,
                stats: crate::index::IndexStats::default(),
                building: false, // _id index is ready when loaded
            };
            if let Some(loaded_tree) =
                crate::collection_core::try_load_index_from_file(&db_path, &id_meta)
            {
                log_debug!(
                    "Loaded _id index '{}' from .idx file ({} keys)",
                    id_index_name,
                    loaded_tree.size()
                );
                index_manager.add_loaded_index(loaded_tree);
                true
            } else {
                false
            }
        } else {
            false
        };

        // Create empty _id index if not loaded from file
        if !id_index_loaded {
            index_manager.create_btree_index(
                id_index_name.clone(),
                "_id".to_string(),
                true,
                false,
            )?;
        }

        // Load persisted custom indexes (delegated to helper)
        // Only load from .idx files if clean shutdown - stale files may have tombstone entries
        let btree_indexes_loaded_from_file = Self::load_persisted_indexes(
            &mut index_manager,
            &persisted_indexes,
            &id_index_name,
            &db_path,
            was_clean,
        )?;

        // Load or create fuzzy indexes from persisted metadata
        // Only load from files if clean shutdown (stale files may have tombstone entries)
        for fuzzy_meta in &persisted_fuzzy_indexes {
            let mut loaded = false;
            if was_clean {
                if let Some(loaded_index) =
                    crate::collection_core::try_load_fuzzy_index_from_file(&db_path, fuzzy_meta)
                {
                    log_debug!(
                        "Loaded fuzzy index '{}' from .fzidx file ({} entries)",
                        fuzzy_meta.name,
                        loaded_index.entry_count()
                    );
                    index_manager.add_loaded_fuzzy_index(loaded_index);
                    loaded = true;
                }
            }
            if !loaded {
                // Create new index with disk storage (will be rebuilt from documents)
                let storage_path =
                    crate::collection_core::build_fuzzy_index_file_path(&db_path, &fuzzy_meta.name);
                if let Err(e) = index_manager.create_fuzzy_index_with_storage(
                    fuzzy_meta.name.clone(),
                    fuzzy_meta.field.clone(),
                    fuzzy_meta.algorithm,
                    fuzzy_meta.threshold,
                    storage_path,
                ) {
                    log_debug!(
                        "Warning: Failed to recreate fuzzy index '{}': {}",
                        fuzzy_meta.name,
                        e
                    );
                }
            }
        }

        // Load or create fulltext indexes from persisted metadata
        // Only load from files if clean shutdown (stale files may have tombstone entries)
        for fts_meta in &persisted_fulltext_indexes {
            let mut loaded = false;
            if was_clean {
                if let Some(loaded_index) =
                    crate::collection_core::try_load_fulltext_index_from_file(&db_path, fts_meta)
                {
                    log_debug!(
                        "Loaded fulltext index '{}' from .ftidx file ({} docs, {} tokens)",
                        fts_meta.name,
                        loaded_index.doc_count(),
                        loaded_index.token_count()
                    );
                    index_manager.add_loaded_fulltext_index(loaded_index);
                    loaded = true;
                }
            }
            if !loaded {
                // Create new index with disk storage (will be rebuilt from documents)
                let storage_path = crate::collection_core::build_fulltext_index_file_path(
                    &db_path,
                    &fts_meta.name,
                );
                if let Err(e) = index_manager.create_fulltext_index_with_storage(
                    fts_meta.name.clone(),
                    fts_meta.field.clone(),
                    fts_meta.language,
                    Some(fts_meta.min_word_length),
                    Some(fts_meta.accent_folding),
                    storage_path,
                ) {
                    log_debug!(
                        "Warning: Failed to recreate fulltext index '{}': {}",
                        fts_meta.name,
                        e
                    );
                }
            }
        }

        // Load or create vector indexes from persisted metadata
        // Only load from files if clean shutdown (stale files may have tombstone entries)
        for vec_meta in &persisted_vector_indexes {
            let mut loaded = false;
            if was_clean {
                if let Some(loaded_index) =
                    crate::index::IndexManager::try_load_vector_index(&db_path, &vec_meta.name)
                {
                    log_debug!(
                        "Loaded vector index '{}' from .hnsw file ({} vectors)",
                        vec_meta.name,
                        loaded_index.len()
                    );
                    index_manager.add_loaded_vector_index(vec_meta.name.clone(), loaded_index);
                    loaded = true;
                }
            }
            if !loaded {
                // Create new index (will be rebuilt from documents)
                log_debug!(
                    "Vector index '{}' {} - creating empty (will rebuild)",
                    vec_meta.name,
                    if was_clean {
                        "cache not found"
                    } else {
                        "dirty shutdown"
                    }
                );
                let storage_path =
                    crate::index::IndexManager::build_vector_cache_path(&db_path, &vec_meta.name);
                if let Err(e) = index_manager.create_vector_index(
                    vec_meta.name.clone(),
                    vec_meta.field.clone(),
                    vec_meta.config.clone(),
                    Some(storage_path),
                ) {
                    log_debug!(
                        "Warning: Failed to create vector index '{}': {}",
                        vec_meta.name,
                        e
                    );
                }
            }
        }

        // Determine what needs rebuilding
        // (was_clean is already known from earlier)

        // Check if any B+ tree indexes need rebuilding
        // Skip indexes that were successfully loaded from .idx files (even if empty/sparse)
        let has_btree_without_file = persisted_indexes.iter().any(|meta| {
            // Skip indexes loaded from .idx files
            if btree_indexes_loaded_from_file.contains(&meta.name) {
                return false;
            }
            // Skip _id index if it was loaded
            if meta.name == id_index_name && id_index_loaded {
                return false;
            }
            // Check if index is empty (needs rebuild)
            index_manager
                .get_btree_index(&meta.name)
                .map(|idx| idx.size() == 0)
                .unwrap_or(true)
        });

        let fuzzy_indexes = index_manager.list_fuzzy_indexes();
        let has_fuzzy_without_file = fuzzy_indexes.iter().any(|idx| idx.entry_count() == 0);
        let fulltext_indexes = index_manager.list_fulltext_indexes();
        let has_fulltext_without_file = fulltext_indexes.iter().any(|idx| idx.doc_count() == 0);
        let vector_indexes = index_manager.list_vector_indexes();
        let has_vector_without_cache = vector_indexes.iter().any(|idx| idx.is_empty());

        // FAST PATH: If clean shutdown AND no empty indexes (ANY type), skip rebuild
        if was_clean
            && !catalog.is_empty()
            && !has_btree_without_file
            && !has_fuzzy_without_file
            && !has_fulltext_without_file
            && !has_vector_without_cache
        {
            log_debug!(
                "Clean shutdown detected - trusting {} indexes from files (skipping rebuild of {} docs)",
                persisted_indexes.len() + persisted_vector_indexes.len(),
                catalog.len()
            );
        } else {
            // SLOW PATH: Rebuild indexes from document catalog
            let reason = if !was_clean {
                "dirty shutdown/crash"
            } else if has_btree_without_file {
                "B+ tree indexes missing .idx file"
            } else if has_fuzzy_without_file {
                "fuzzy indexes missing .fzidx file"
            } else if has_fulltext_without_file {
                "fulltext indexes missing .ftidx file"
            } else if has_vector_without_cache {
                "vector indexes missing .hnsw file"
            } else {
                "empty catalog"
            };
            // WARN level so users can see why startup is slow
            log_warn!(
                "Rebuilding indexes due to {} - {} catalog entries (this may take a while)",
                reason,
                catalog.len()
            );

            let mut storage_guard = self.storage.write();
            let rebuilt_count = Self::rebuild_indexes_from_catalog(
                &mut index_manager,
                &mut *storage_guard,
                name,
                &catalog,
                &persisted_indexes,
                &id_index_name,
            )?;

            log_debug!(
                "Index rebuild completed - {} index entries rebuilt",
                rebuilt_count
            );
        }

        Ok(index_manager)
    }

    /// Get collection (creates if doesn't exist)
    ///
    /// Uses shared IndexManager and Schema to fix stale index/schema problems.
    /// Note: This method IMPLICITLY creates the collection if it doesn't exist.
    /// For read-only access without creation, use `get_collection()` instead.
    pub fn collection(&self, name: &str) -> Result<CollectionCore<S>> {
        self.check_not_closed()?;
        let shared_indexes = self.get_or_create_index_manager(name)?;
        let shared_schema = self.get_or_create_schema_manager(name)?;
        CollectionCore::with_shared_indexes(
            name.to_string(),
            Arc::clone(&self.storage),
            shared_indexes,
            shared_schema,
            Arc::clone(&self.is_closed),
        )
    }

    /// Get collection WITHOUT implicit creation - returns error if not exists
    ///
    /// Use this for read operations (find, count, etc.) to avoid creating
    /// empty collections from typos or invalid names.
    ///
    /// Optimized for READ operations - uses READ locks only on the hot path.
    pub fn get_collection(&self, name: &str) -> Result<CollectionCore<S>> {
        self.check_not_closed()?;
        // Fast path: check if index manager is cached
        let shared_indexes = {
            let managers = self.index_managers.read();
            match managers.get(name) {
                Some(manager) => Arc::clone(manager),
                None => {
                    // No index manager = collection was never accessed via collection()
                    // Check storage directly
                    let storage = self.storage.read();
                    if storage.get_collection_meta(name).is_none() {
                        return Err(IronBaseError::CollectionNotFound(name.to_string()));
                    }
                    // Collection exists in storage but no index manager yet
                    // This can happen after database reopen - need to create manager
                    drop(storage);
                    drop(managers);
                    let shared_indexes = self.get_or_create_index_manager(name)?;
                    let shared_schema = self.get_or_create_schema_manager(name)?;
                    return CollectionCore::with_shared_indexes_readonly(
                        name.to_string(),
                        Arc::clone(&self.storage),
                        shared_indexes,
                        shared_schema,
                        Arc::clone(&self.is_closed),
                    );
                }
            }
        };

        // Use readonly path - no write locks
        let shared_schema = self.get_or_create_schema_manager(name)?;
        CollectionCore::with_shared_indexes_readonly(
            name.to_string(),
            Arc::clone(&self.storage),
            shared_indexes,
            shared_schema,
            Arc::clone(&self.is_closed),
        )
    }

    /// Check if a collection exists (without creating it)
    pub fn collection_exists(&self, name: &str) -> bool {
        let storage = self.storage.read();
        storage.get_collection_meta(name).is_some()
    }

    /// Set or clear JSON schema for a collection
    pub fn set_collection_schema(&self, name: &str, schema: Option<Value>) -> Result<()> {
        let collection = self.collection(name)?;
        collection.set_schema(schema)
    }

    /// List visible collection names (excludes hidden collections)
    pub fn list_collections(&self) -> Vec<String> {
        let storage = self.storage.read();
        storage
            .list_collections()
            .into_iter()
            .filter(|name| {
                storage
                    .get_collection_meta(name)
                    .map(|meta| !meta.flags.hidden)
                    .unwrap_or(true)
            })
            .collect()
    }

    /// List ALL collection names including hidden/system collections
    pub fn list_all_collections(&self) -> Vec<String> {
        let storage = self.storage.read();
        storage.list_collections()
    }

    /// Drop collection (fails if protected)
    pub fn drop_collection(&self, name: &str) -> Result<()> {
        // Check if collection is protected
        {
            let storage = self.storage.read();
            if let Some(meta) = storage.get_collection_meta(name) {
                if meta.flags.protected {
                    return Err(IronBaseError::OperationNotAllowed(format!(
                        "Cannot drop protected collection '{}'",
                        name
                    )));
                }
            }
        }

        // Remove shared IndexManager, SchemaManager, and collection write lock
        self.index_managers.write().remove(name);
        self.schema_managers.write().remove(name);
        self.collection_write_locks.write().remove(name);

        let mut storage = self.storage.write();
        storage.drop_collection(name)
    }

    /// Set collection flags (system, protected, hidden)
    pub fn set_collection_flags(
        &self,
        name: &str,
        flags: crate::storage::CollectionFlags,
    ) -> Result<()> {
        let mut storage = self.storage.write();
        let meta = storage
            .get_collection_meta_mut(name)
            .ok_or_else(|| IronBaseError::CollectionNotFound(name.to_string()))?;
        meta.flags = flags;
        // CRITICAL FIX: Flush metadata to persist flag changes
        // Without this, flags could be lost on crash (bug found 2024-12-26)
        storage.flush()?;
        Ok(())
    }

    /// Create a system collection (auto-sets is_system + protected flags)
    /// System collections use `_system.` prefix by convention
    pub fn create_system_collection(&self, name: &str) -> Result<()> {
        // Create the collection first
        {
            let mut storage = self.storage.write();
            storage.create_collection(name)?;
        }

        // Set system flags
        let flags = crate::storage::CollectionFlags {
            is_system: true,
            protected: true,
            hidden: true, // System collections hidden by default
        };
        self.set_collection_flags(name, flags)
    }

    /// Get collection flags (returns default flags if collection not found)
    pub fn get_collection_flags(&self, name: &str) -> Option<crate::storage::CollectionFlags> {
        let storage = self.storage.read();
        storage.get_collection_meta(name).map(|meta| meta.flags)
    }

    /// Force drop a protected collection (admin only)
    /// Use with caution - bypasses protection checks
    pub fn force_drop_collection(&self, name: &str) -> Result<()> {
        // Remove shared IndexManager, SchemaManager, and collection write lock
        self.index_managers.write().remove(name);
        self.schema_managers.write().remove(name);
        self.collection_write_locks.write().remove(name);

        let mut storage = self.storage.write();
        storage.drop_collection(name)
    }
}
