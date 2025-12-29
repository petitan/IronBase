// src/database/collections.rs
// Collection, index, and schema management

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
    fn load_persisted_indexes(
        index_manager: &mut IndexManager,
        persisted_indexes: &[crate::index::IndexMetadata],
        id_index_name: &str,
        db_path: &str,
    ) -> Result<()> {
        use crate::log_debug;

        for index_meta in persisted_indexes {
            // Skip _id index (already created)
            if index_meta.name == id_index_name {
                continue;
            }

            // Try to load from .idx file first
            if let Some(loaded_tree) =
                crate::collection_core::try_load_index_from_file(db_path, index_meta)
            {
                log_debug!(
                    "Loaded index '{}' from .idx file (will rebuild from documents)",
                    index_meta.name
                );
                index_manager.add_loaded_index(loaded_tree);
            } else {
                // Fallback: create empty index
                log_debug!(
                    "Creating index '{}' on field '{}' (will rebuild from documents)",
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
        }
        Ok(())
    }

    /// Rebuild all indexes from document catalog
    fn rebuild_indexes_from_catalog<S2: Storage + RawStorage>(
        index_manager: &mut IndexManager,
        storage: &mut S2,
        collection_name: &str,
        catalog: &std::collections::HashMap<DocumentId, u64>,
        persisted_indexes: &[crate::index::IndexMetadata],
        id_index_name: &str,
    ) -> Result<u64> {
        use crate::log_warn;

        let mut rebuilt_count = 0u64;

        // Pre-collect fuzzy index info to avoid repeated lookups in the loop
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

        for (_id_key, offset) in catalog.iter() {
            // Read document from disk
            let doc_bytes = match storage.read_document_at(collection_name, *offset) {
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

            // Rebuild _id index
            let index_key = IndexKey::from(id_value);
            if let Some(id_index) = index_manager.get_btree_index_mut(id_index_name) {
                // BUG #4 FIX: Log duplicate key errors instead of silently ignoring
                // This helps diagnose database inconsistencies after crash recovery
                if let Err(e) = id_index.insert(index_key, doc_id.clone()) {
                    log_warn!(
                        "Index rebuild: duplicate _id key ignored for {}: {:?}",
                        collection_name,
                        e
                    );
                }
            }

            // Rebuild custom indexes
            for index_meta in persisted_indexes {
                if index_meta.name == id_index_name {
                    continue;
                }

                let Some(index) = index_manager.get_btree_index_mut(&index_meta.name) else {
                    continue;
                };

                let key = index.extract_key(&doc);
                let is_all_null = match &key {
                    IndexKey::Null => true,
                    IndexKey::Compound(keys) => keys.iter().all(|k| matches!(k, IndexKey::Null)),
                    _ => false,
                };

                // Include null keys for unique indexes, skip for non-unique
                if !is_all_null || index.metadata.unique {
                    // BUG #4 FIX: Log duplicate key errors instead of silently ignoring
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

            // Rebuild fuzzy indexes (using pre-collected info from outside the loop)
            for (index_name, field) in &fuzzy_info {
                if let Some(value) = crate::value_utils::get_nested_value(&doc, field) {
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
                if let Some(value) = crate::value_utils::get_nested_value(&doc, field) {
                    if let Some(s) = value.as_str() {
                        if let Some(index) = index_manager.get_fulltext_index_mut(index_name) {
                            let _ = index.insert(&doc_id, s);
                            rebuilt_count += 1;
                        }
                    }
                }
            }
        }

        Ok(rebuilt_count)
    }

    /// Initialize an IndexManager for a collection (internal)
    ///
    /// This creates the _id index, loads persisted indexes, and rebuilds
    /// all indexes from the document catalog.
    fn initialize_index_manager(&self, name: &str) -> Result<IndexManager> {
        use crate::log_debug;

        let mut index_manager = IndexManager::new();

        // Create automatic _id index (unique)
        let id_index_name = format!("{}_id", name);
        index_manager.create_btree_index(id_index_name.clone(), "_id".to_string(), true, false)?;

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

        log_debug!(
            "Collection '{}' - catalog size: {}, persisted indexes: {}, fuzzy indexes: {}, fulltext indexes: {}",
            name,
            catalog.len(),
            persisted_indexes.len(),
            persisted_fuzzy_indexes.len(),
            persisted_fulltext_indexes.len()
        );

        // Get db_path for .idx file loading
        let db_path = storage_guard.get_file_path().to_string();

        drop(storage_guard); // Release write lock before rebuilding

        // Load persisted custom indexes (delegated to helper)
        Self::load_persisted_indexes(
            &mut index_manager,
            &persisted_indexes,
            &id_index_name,
            &db_path,
        )?;

        // Create fuzzy indexes from persisted metadata (data will be rebuilt from documents)
        for fuzzy_meta in &persisted_fuzzy_indexes {
            if let Err(e) = index_manager.create_fuzzy_index(
                fuzzy_meta.name.clone(),
                fuzzy_meta.field.clone(),
                fuzzy_meta.algorithm,
                fuzzy_meta.threshold,
            ) {
                log_debug!(
                    "Warning: Failed to recreate fuzzy index '{}': {}",
                    fuzzy_meta.name,
                    e
                );
            }
        }

        // Load or create fulltext indexes from persisted metadata
        for fts_meta in &persisted_fulltext_indexes {
            // Try to load from .ftidx file first
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
            } else {
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

        // Rebuild all indexes from document catalog (delegated to helper)
        log_debug!(
            "Starting index rebuild from {} catalog entries",
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
