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
use crate::index::{IndexKey, IndexManager, VectorIndexManager};
use crate::storage::{RawStorage, Storage};
use crate::transaction::{Operation, TransactionId};

use super::DatabaseCore;

/// Strategy for handling a persisted index file at startup (task #26 R2).
///
/// Centralises the "trust the file vs rebuild from catalog" decision that
/// was previously scattered across `load_persisted_indexes`, the `_id`
/// load block, and the fuzzy/fulltext/vector load blocks. R4 will extend
/// the input set with `last_flushed_tx_id` and `recovered_ops` once the
/// `was_clean` strict check is replaced with watermark-based reasoning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IndexLoadStrategy {
    /// Attempt to load the persisted file. Skip rebuild on success; the
    /// loaded state is trusted as the latest committed state.
    LoadFromFile,
    /// Do NOT attempt to load. Create an empty in-memory index — caller
    /// must rebuild from the document catalog.
    EmptyForRebuild,
}

/// Decide how to bring a persisted index online at startup (task #26 R4).
///
/// Unified across all four index families (btree / fuzzy / fulltext / HNSW).
/// Trust the persisted file on clean shutdown OR when the watermark-based
/// WAL replay path is viable (`can_try_replay` — bounded pending ops on a
/// dirty reopen). The `9ff48302` Stale Index Loading protection used to
/// gate this on `was_clean` alone for btree, but two newer mechanisms
/// supersede that strictness:
///   1. R7 BTFT footer (`5df37968`) — every `.idx` carries its real
///      `last_flushed_tx_id`. The safety rail inside `try_wal_replay_btree`
///      drops a non-empty btree whose footer watermark is 0 *and* there
///      are pending ops we cannot attribute, falling back to rebuild from
///      catalog.
///   2. `apply_op_to_btree` uses delete-then-insert idempotency, so even
///      when the legacy `IndexChange` apply path in `database/mod.rs`
///      runs in parallel on dirty shutdown the two writers converge to
///      the same single-entry state instead of duplicating keys.
/// Phase A atomic flush (`fab5d97e` / `84638dea` / `26af2ffd`) makes the
/// `.idx` / `.ftidx` / `.fzidx` / `.hnsw` files safe to load even after
/// a crash.
pub(crate) fn decide_index_load_strategy(
    was_clean: bool,
    can_try_replay: bool,
) -> IndexLoadStrategy {
    if was_clean || can_try_replay {
        IndexLoadStrategy::LoadFromFile
    } else {
        IndexLoadStrategy::EmptyForRebuild
    }
}

// ============================================================================
// COLLECTION AND INDEX MANAGEMENT (Generic Implementation)
// ============================================================================

impl<S: Storage + RawStorage> DatabaseCore<S> {
    // ========== IndexManager Management ==========

    /// Get or create a shared IndexManager for a collection
    ///
    /// This method uses double-checked locking to ensure thread-safe creation
    /// of IndexManagers while minimizing lock contention.
    /// Ensure BOTH the IndexManager and the VectorIndexManager exist for a
    /// collection, creating them together (single catalog scan) and keeping the
    /// two per-collection maps in lockstep. Returns clones of both Arcs.
    ///
    /// Lock-ordering: the `index_managers` MAP lock is always taken before the
    /// `vector_managers` MAP lock (and never the reverse), so these two map
    /// locks can't deadlock against each other.
    #[allow(clippy::type_complexity)]
    pub(crate) fn ensure_managers(
        &self,
        name: &str,
    ) -> Result<(Arc<RwLock<IndexManager>>, Arc<RwLock<VectorIndexManager>>)> {
        // Fast path: both already cached
        {
            let im = self.index_managers.read();
            if let Some(m) = im.get(name) {
                if let Some(v) = self.vector_managers.read().get(name) {
                    return Ok((Arc::clone(m), Arc::clone(v)));
                }
            }
        }

        // Slow path: create with write lock (double-checked). index_managers
        // first, then vector_managers — never the reverse.
        let mut im = self.index_managers.write();
        if let Some(m) = im.get(name) {
            // index manager exists → vector manager is in lockstep
            let v = self
                .vector_managers
                .read()
                .get(name)
                .map(Arc::clone)
                .expect("vector_managers must be in lockstep with index_managers");
            return Ok((Arc::clone(m), v));
        }

        // Initialize both managers from one catalog scan
        let (index_manager, vector_manager) = self.initialize_managers(name)?;
        let im_arc = Arc::new(RwLock::new(index_manager));
        let vm_arc = Arc::new(RwLock::new(vector_manager));
        im.insert(name.to_string(), Arc::clone(&im_arc));
        self.vector_managers
            .write()
            .insert(name.to_string(), Arc::clone(&vm_arc));
        Ok((im_arc, vm_arc))
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
        can_try_replay: bool,
    ) -> Result<std::collections::HashSet<String>> {
        use crate::log_debug;
        let mut loaded_from_file = std::collections::HashSet::new();

        // Unified load decision — same rule across btree and non-btree.
        // The R7 BTFT footer carries the real watermark; the safety rail
        // in `try_wal_replay_btree` catches stale legacy `.idx` files
        // (no footer → caller's metadata watermark = 0) when ops are
        // pending and falls back to rebuild from catalog.
        let strategy = decide_index_load_strategy(was_clean, can_try_replay);

        for index_meta in persisted_indexes {
            // Skip _id index (already created)
            if index_meta.name == id_index_name {
                continue;
            }

            if strategy == IndexLoadStrategy::LoadFromFile {
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
        vector_manager: &mut VectorIndexManager,
        storage: &mut S2,
        collection_name: &str,
        catalog: &std::collections::HashMap<DocumentId, u64>,
        persisted_indexes: &[crate::index::IndexMetadata],
        id_index_name: &str,
    ) -> Result<u64> {
        use crate::{log_debug, log_info, log_warn};

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
        // SKIP indexes that already have data (loaded from .fzidx file)
        let fuzzy_info: Vec<_> = index_manager
            .list_fuzzy_indexes()
            .iter()
            .filter(|idx| idx.entry_count() == 0) // Only rebuild empty indexes
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
            vector_manager
                .vector_index_names()
                .into_iter()
                .filter_map(|name| {
                    let idx = vector_manager.get_vector_index(&name)?;
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
            // Uses INFO level so progress is visible during normal operation
            if batch_num % log_interval == 0 || batch_num == total_batches - 1 {
                let progress = (batch_num + 1) * 100 / total_batches;
                let docs_processed =
                    std::cmp::min((batch_num + 1) * REBUILD_BATCH_SIZE, total_docs);
                log_info!(
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
                                let pdid_field = index.parent_doc_id_field().to_string();
                                let parent_doc_id =
                                    crate::value_utils::get_nested_value(doc, &pdid_field)
                                        .and_then(|v| v.as_str().map(|x| x.to_string()));
                                let _ = if let Some(ref pdid) = parent_doc_id {
                                    index.insert_with_parent_doc_id(doc_id, s, pdid)
                                } else {
                                    index.insert(doc_id, s)
                                };
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
                                if let Some(index) = vector_manager.get_vector_index_mut(index_name)
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

        // FIX: Mark all rebuilt indexes as ready (building = false)
        // Without this, indexes remain in "building" state and are invisible
        // to the query planner (explain shows 0 availableIndexes)
        if rebuild_id_index {
            let _ = index_manager.set_index_ready(id_index_name);
        }
        for index_meta in &btree_indexes_to_rebuild {
            let _ = index_manager.set_index_ready(&index_meta.name);
        }
        for (index_name, _) in &fuzzy_info {
            let _ = index_manager.set_index_ready(index_name);
        }
        for (index_name, _) in &fulltext_info {
            let _ = index_manager.set_index_ready(index_name);
        }

        // FIX: Mark all rebuilt indexes dirty so the next checkpoint persists
        // the in-memory state. Without this, the rebuild output never reaches
        // disk and every subsequent SIGKILL repeats the rebuild from catalog
        // (read-mostly DBs especially — never anything else dirties them).
        // Touches all four index families that the rebuild loop populates.
        if rebuild_id_index {
            index_manager.mark_btree_dirty(id_index_name);
        }
        for index_meta in &btree_indexes_to_rebuild {
            index_manager.mark_btree_dirty(&index_meta.name);
        }
        for (index_name, _) in &fuzzy_info {
            index_manager.mark_fuzzy_dirty(index_name);
        }
        for (index_name, _) in &fulltext_info {
            index_manager.mark_fulltext_dirty(index_name);
        }
        for (index_name, _, _) in &vector_indexes_to_rebuild {
            vector_manager.mark_vector_dirty(index_name);
        }

        Ok(rebuilt_count)
    }

    /// Attempt watermark-filtered WAL replay for a collection's non-btree indexes.
    ///
    /// For each loaded fuzzy / fulltext / vector index, iterate `recovered_ops`
    /// and apply those whose `tx_id > index.last_flushed_tx_id()` using the
    /// helpers in `recovery::index_replay_ops`. The helpers are idempotent —
    /// applying an op whose effect is already reflected in the loaded file
    /// state is a no-op.
    ///
    /// Returns:
    /// - `Ok(true)` — replay converged; indexes are consistent with the latest
    ///   committed state and should NOT be rebuilt from catalog.
    /// - `Ok(false)` — a safety rail tripped (a loaded-but-non-empty index has
    ///   `last_flushed_tx_id == 0`, i.e. a legacy V3/pre-watermark file; or a
    ///   helper returned an error). Caller must fall back to rebuild.
    ///
    /// Apply WAL ops to fuzzy, fulltext, and vector (HNSW) indexes via the
    /// watermark pattern. Returns `Ok(false)` if any safety rail trips
    /// (legacy V3/pre-watermark file with non-empty content + non-empty WAL),
    /// signalling the caller to fall back to rebuild for the non-btree side.
    fn try_wal_replay_non_btree(
        index_manager: &mut IndexManager,
        vector_manager: &mut VectorIndexManager,
        recovered_ops: &[(TransactionId, Operation)],
        persisted_fuzzy: &[crate::index::fuzzy::FuzzyIndexMetadata],
        persisted_fulltext: &[crate::fulltext::FulltextIndexMetadata],
        persisted_vector: &[crate::vector::VectorIndexMetadata],
    ) -> Result<bool> {
        // Fuzzy
        for fz_meta in persisted_fuzzy {
            let watermark = match index_manager.get_fuzzy_index(&fz_meta.name) {
                Some(idx) => {
                    // A loaded-but-non-empty index with watermark=0 is a legacy
                    // V3 file. If the WAL has no committed ops to replay, the
                    // file IS the latest state and we trust it. If there are
                    // ops, we don't know which are already applied, so rebuild.
                    if idx.last_flushed_tx_id() == 0
                        && idx.entry_count() > 0
                        && !recovered_ops.is_empty()
                    {
                        return Ok(false);
                    }
                    idx.last_flushed_tx_id()
                }
                None => continue, // index wasn't loaded — rebuild path will handle it
            };
            let field = fz_meta.field.clone();
            if let Some(idx) = index_manager.get_fuzzy_index_mut(&fz_meta.name) {
                for (tx_id, op) in recovered_ops {
                    if *tx_id > watermark {
                        crate::recovery::apply_op_to_fuzzy(idx, op, &field)?;
                    }
                }
            }
        }

        // Fulltext
        for ft_meta in persisted_fulltext {
            let watermark = match index_manager.get_fulltext_index(&ft_meta.name) {
                Some(idx) => {
                    if idx.last_flushed_tx_id() == 0
                        && idx.doc_count() > 0
                        && !recovered_ops.is_empty()
                    {
                        return Ok(false);
                    }
                    idx.last_flushed_tx_id()
                }
                None => continue,
            };
            let field = ft_meta.field.clone();
            if let Some(idx) = index_manager.get_fulltext_index_mut(&ft_meta.name) {
                for (tx_id, op) in recovered_ops {
                    if *tx_id > watermark {
                        crate::recovery::apply_op_to_fulltext(idx, op, &field)?;
                    }
                }
            }
        }

        // Vector (HNSW)
        for vec_meta in persisted_vector {
            let watermark = match vector_manager.get_vector_index(&vec_meta.name) {
                Some(idx) => {
                    if idx.last_flushed_tx_id() == 0 && !idx.is_empty() && !recovered_ops.is_empty()
                    {
                        return Ok(false);
                    }
                    idx.last_flushed_tx_id()
                }
                None => continue,
            };
            let field = vec_meta.field.clone();
            if let Some(idx) = vector_manager.get_vector_index_mut(&vec_meta.name) {
                for (tx_id, op) in recovered_ops {
                    if *tx_id > watermark {
                        crate::recovery::apply_op_to_hnsw(idx, op, &field)?;
                    }
                }
            }
        }

        Ok(true)
    }

    /// Apply WAL ops to B+ tree indexes only. Decoupled from non-btree replay
    /// (`try_wal_replay_non_btree`) so a fuzzy/fulltext/vector load failure
    /// — which would force the slow path for those families — does NOT
    /// strand the btree at its pre-Phase-2 state. The slow-path
    /// `rebuild_indexes_from_catalog` skips already-loaded btrees by `size > 0`,
    /// so without this independent btree replay the btree would silently miss
    /// every Phase 2 op when any non-btree load fails.
    ///
    /// Returns `Ok(false)` on a safety-rail trip (legacy `.idx` with
    /// non-empty content + non-empty WAL); the caller must then rebuild
    /// the btree from catalog. Successful replay marks each btree dirty
    /// (`265f0c2e` rule) so the next checkpoint persists the new watermark.
    fn try_wal_replay_btree(
        index_manager: &mut IndexManager,
        recovered_ops: &[(TransactionId, Operation)],
        persisted_btree: &[crate::index::IndexMetadata],
    ) -> Result<bool> {
        for bt_meta in persisted_btree {
            let watermark = match index_manager.get_btree_index(&bt_meta.name) {
                Some(idx) => {
                    // Loaded-but-non-empty btree with watermark=0 = legacy `.idx`
                    // (pre-task-#26 build). If the WAL has no ops to replay, the
                    // file is the latest state and we trust it. Otherwise we
                    // can't tell which ops are reflected, so rebuild.
                    if idx.last_flushed_tx_id() == 0 && idx.size() > 0 && !recovered_ops.is_empty()
                    {
                        return Ok(false);
                    }
                    idx.last_flushed_tx_id()
                }
                None => continue, // index wasn't loaded — rebuild path will handle it
            };
            if let Some(idx) = index_manager.get_btree_index_mut(&bt_meta.name) {
                for (tx_id, op) in recovered_ops {
                    if *tx_id > watermark {
                        crate::recovery::apply_op_to_btree(idx, op)?;
                    }
                }
            }
            // 265f0c2e rule: any successful mutation must mark the index dirty
            // so the next checkpoint persists the post-replay state (and the
            // updated `last_flushed_tx_id` along with it).
            index_manager.mark_btree_dirty(&bt_meta.name);
        }

        Ok(true)
    }

    /// Initialize an IndexManager for a collection (internal)
    ///
    /// This creates the _id index, loads persisted indexes, and rebuilds
    /// all indexes from the document catalog.
    fn initialize_managers(&self, name: &str) -> Result<(IndexManager, VectorIndexManager)> {
        use crate::{log_debug, log_info, log_warn};

        let mut index_manager = IndexManager::new();
        let mut vector_manager = VectorIndexManager::new();
        let id_index_name = format!("{}_id", name);

        // Ensure collection exists before loading metadata
        {
            let mut storage_guard = self.storage.write();
            if storage_guard.get_collection_meta(name).is_none() {
                storage_guard.create_collection(name)?;
            }
        }

        // Load persisted indexes and schema (read lock: only &self methods used)
        let storage_guard = self.storage.read();
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

        // Task #26 phase 4b: on dirty shutdown, attempt WAL replay for non-btree
        // indexes if the pending ops are bounded. This lets us load the persisted
        // index files (Phase A atomic flush makes them safe to trust) and then
        // apply only the post-watermark ops, instead of doing the O(N docs)
        // rebuild-from-catalog for fulltext / fuzzy / hnsw.
        //
        // The load blocks below gate on `was_clean || can_try_replay` so the
        // dirty-shutdown case also attempts to load. If any safety rail fails
        // (too many ops, legacy index without watermark, helper error), we
        // fall back to the existing rebuild path via `rebuild_indexes_from_catalog`
        // which skips any index that already has data.
        //
        // `take_recovered_operations_for_collection` drains this collection's
        // bucket out of the DB-level map so its memory is freed as soon as the
        // replay finishes. Clean shutdown path returns an empty Vec because the
        // map is always empty on a clean reopen.
        let recovered_ops_for_collection = if was_clean {
            Vec::new()
        } else {
            self.take_recovered_operations_for_collection(name)
        };
        // Run the watermark-checking replay path on every dirty reopen, even
        // when the WAL has no committed ops for this collection. With an
        // empty op set the replay loops trivially complete; the value is the
        // safety-rail check itself, which tells `wal_replay_succeeded` to
        // trust the loaded `.idx`/`.ftidx`/`.fzidx`/`.hnsw` state and skip
        // the rebuild path. Without this, a read-mostly DB with no pending
        // ops can never escape rebuild-from-catalog after a SIGKILL.
        let can_try_replay = !was_clean
            && recovered_ops_for_collection.len() <= crate::limits::MAX_REPLAY_OPS_PER_COLLECTION;

        if can_try_replay {
            log_info!(
                "Collection '{}': attempting WAL replay recovery ({} ops pending)",
                name,
                recovered_ops_for_collection.len()
            );
        } else if !was_clean
            && recovered_ops_for_collection.len() > crate::limits::MAX_REPLAY_OPS_PER_COLLECTION
        {
            log_warn!(
                "Collection '{}': {} pending WAL ops exceed MAX_REPLAY_OPS_PER_COLLECTION ({}), falling back to rebuild-from-catalog",
                name,
                recovered_ops_for_collection.len(),
                crate::limits::MAX_REPLAY_OPS_PER_COLLECTION
            );
        }

        // Try to load _id index from .idx file via the unified decision
        // helper. Empty catalog → nothing to load (the index would always
        // be empty); rebuild path is a no-op in that case.
        let id_index_loaded = if !catalog.is_empty()
            && decide_index_load_strategy(was_clean, can_try_replay)
                == IndexLoadStrategy::LoadFromFile
        {
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
                last_flushed_tx_id: 0,
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

        // Load persisted custom indexes (delegated to helper).
        // Strategy unified across all four index families via
        // unified helper `decide_index_load_strategy(was_clean, can_try_replay)`.
        let btree_indexes_loaded_from_file = Self::load_persisted_indexes(
            &mut index_manager,
            &persisted_indexes,
            &id_index_name,
            &db_path,
            was_clean,
            can_try_replay,
        )?;

        // Load or create fuzzy indexes from persisted metadata.
        //
        // The file is loaded on clean shutdown (trusted as-is) OR on dirty
        // shutdown when WAL replay is viable (`can_try_replay` — bounded pending
        // ops). Phase A atomic flush (commit fab5d97e / 84638dea) makes the
        // file safe to load even after a crash — any post-flush mutations are
        // captured in the WAL and replayed per-index below.
        //
        // `all_non_btree_loaded` tracks whether every persisted non-btree index
        // was successfully loaded. If any load returns None, WAL replay is
        // unsafe (pre-watermark data is gone) and we must fall back to
        // rebuild-from-catalog even when `can_try_replay` is true.
        let mut all_non_btree_loaded = true;
        for fuzzy_meta in &persisted_fuzzy_indexes {
            let mut loaded = false;
            if decide_index_load_strategy(was_clean, can_try_replay)
                == IndexLoadStrategy::LoadFromFile
            {
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
                all_non_btree_loaded = false;
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

        // Load or create fulltext indexes from persisted metadata.
        // Loading rule mirrors fuzzy above: clean shutdown trusts the file,
        // dirty shutdown also loads when WAL replay is viable so post-watermark
        // ops can be applied without doing a full rebuild-from-catalog.
        for fts_meta in &persisted_fulltext_indexes {
            let mut loaded = false;
            if decide_index_load_strategy(was_clean, can_try_replay)
                == IndexLoadStrategy::LoadFromFile
            {
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
                all_non_btree_loaded = false;
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

        // Load or create vector indexes from persisted metadata.
        // Loading rule mirrors fuzzy/fulltext above: dirty shutdown also loads
        // when WAL replay is viable.
        for vec_meta in &persisted_vector_indexes {
            let mut loaded = false;
            if decide_index_load_strategy(was_clean, can_try_replay)
                == IndexLoadStrategy::LoadFromFile
            {
                if let Some(loaded_index) = crate::index::VectorIndexManager::try_load_vector_index(
                    &db_path,
                    &vec_meta.name,
                ) {
                    log_debug!(
                        "Loaded vector index '{}' from .hnsw file ({} vectors)",
                        vec_meta.name,
                        loaded_index.len()
                    );
                    vector_manager.add_loaded_vector_index(vec_meta.name.clone(), loaded_index);
                    loaded = true;
                }
            }
            if !loaded {
                all_non_btree_loaded = false;
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
                let storage_path = crate::index::VectorIndexManager::build_vector_cache_path(
                    &db_path,
                    &vec_meta.name,
                );
                if let Err(e) = vector_manager.create_vector_index(
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

        // Task #26 phase 4b: attempt WAL replay into loaded indexes.
        //
        // Replay is split into two independent invocations:
        //   * `try_wal_replay_btree` — runs on every dirty reopen with bounded
        //     ops. The slow path's `rebuild_indexes_from_catalog` SKIPS
        //     already-loaded btrees by `size > 0`, so if the btree side were
        //     gated on non-btree load success (as it was prior to 2026-04-28),
        //     a single fzidx/ftidx load failure would silently strand the
        //     btree at its pre-Phase-2 state. Independent replay is therefore
        //     correctness-critical, not just an optimisation.
        //   * `try_wal_replay_non_btree` — gated on `all_non_btree_loaded` so
        //     missing fuzzy/fulltext/vector files defer to the rebuild path,
        //     matching the established "load-or-rebuild" semantics.
        //
        // On guardrail failure (legacy index without watermark, helper error),
        // the affected family's rebuild runs in the slow path below.
        if can_try_replay && !all_non_btree_loaded {
            log_warn!(
                "Collection '{}': non-btree WAL replay skipped — some persisted non-btree index files failed to load, rebuild will rescue them (btree replay still runs independently)",
                name
            );
        }
        let btree_replay_succeeded = if can_try_replay {
            match Self::try_wal_replay_btree(
                &mut index_manager,
                &recovered_ops_for_collection,
                &persisted_indexes,
            ) {
                Ok(true) => {
                    log_debug!(
                        "Collection '{}': btree WAL replay applied ({} ops across {} indexes)",
                        name,
                        recovered_ops_for_collection.len(),
                        persisted_indexes.len()
                    );
                    true
                }
                Ok(false) => {
                    log_warn!(
                        "Collection '{}': btree WAL replay guardrail tripped, falling back to rebuild",
                        name
                    );
                    false
                }
                Err(e) => {
                    log_warn!(
                        "Collection '{}': btree WAL replay error ({}), falling back to rebuild",
                        name,
                        e
                    );
                    false
                }
            }
        } else {
            // No replay attempt → btree side is "ok" iff shutdown was clean
            // (loaded `.idx` already reflects every committed write).
            was_clean
        };
        let non_btree_replay_succeeded = if can_try_replay && all_non_btree_loaded {
            match Self::try_wal_replay_non_btree(
                &mut index_manager,
                &mut vector_manager,
                &recovered_ops_for_collection,
                &persisted_fuzzy_indexes,
                &persisted_fulltext_indexes,
                &persisted_vector_indexes,
            ) {
                Ok(true) => {
                    log_info!(
                        "Collection '{}': non-btree WAL replay succeeded ({} ops applied)",
                        name,
                        recovered_ops_for_collection.len()
                    );
                    true
                }
                Ok(false) => {
                    log_warn!(
                        "Collection '{}': non-btree WAL replay guardrail tripped, falling back to rebuild",
                        name
                    );
                    false
                }
                Err(e) => {
                    log_warn!(
                        "Collection '{}': non-btree WAL replay error ({}), falling back to rebuild",
                        name,
                        e
                    );
                    false
                }
            }
        } else {
            false
        };
        let wal_replay_succeeded = btree_replay_succeeded && non_btree_replay_succeeded;

        // (R6: removed `bfad51fa` one-time auto-migration block. R7 BTFT
        // footer + R1 watermark perzisztencia + ad753469 safety rail relax
        // jointly cover the read-mostly legacy-`.idx` case without the
        // pre-emptive flush: the safety rail with `recovered_ops.is_empty()`
        // trusts watermark=0 files and the next real flush stamps the BTFT
        // footer naturally.)

        // Per-type "this kind of index is empty in memory and was not loaded
        // from disk" flags. Drives the FAST PATH decision and the SLOW PATH
        // reason logging below.
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
        let vector_indexes = vector_manager.list_vector_indexes();
        let has_vector_without_cache = vector_indexes.iter().any(|idx| idx.is_empty());

        // Aggregate: every persisted index family has data in memory.
        // Read as: "no rebuild needed for any index type at all".
        let all_indexes_have_data = !has_btree_without_file
            && !has_fuzzy_without_file
            && !has_fulltext_without_file
            && !has_vector_without_cache;

        // FAST PATH (skip rebuild_indexes_from_catalog) — two ways to qualify:
        //   1. Clean shutdown of a non-empty DB with every index family ready
        //      → trust everything as the latest committed state.
        //   2. Dirty shutdown where BOTH `try_wal_replay_btree` AND
        //      `try_wal_replay_non_btree` succeeded AND btree is ready
        //      (either loaded from `.idx` or there are no btree indexes).
        //      The two replay phases are independent (2026-04-28 fix): a
        //      non-btree load failure no longer strands the btree at its
        //      pre-Phase-2 state, but it still drops us out of the fast
        //      path so the slow path can rebuild the affected non-btree
        //      family from catalog.
        let fast_path = (was_clean && !catalog.is_empty() && all_indexes_have_data)
            || (wal_replay_succeeded && !has_btree_without_file);

        if fast_path {
            log_debug!(
                "Skipping index rebuild for '{}' — {} indexes trusted ({} catalog docs, was_clean={}, wal_replay={})",
                name,
                persisted_indexes.len() + persisted_vector_indexes.len(),
                catalog.len(),
                was_clean,
                wal_replay_succeeded
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
                &mut vector_manager,
                &mut *storage_guard,
                name,
                &catalog,
                &persisted_indexes,
                &id_index_name,
            )?;
            drop(storage_guard);

            log_debug!(
                "Index rebuild completed - {} index entries rebuilt",
                rebuilt_count
            );

            // Force flush every rebuilt index immediately so the next reopen
            // (even if it's a SIGKILL away) finds them in the new format with
            // `last_flushed_tx_id > 0` and uses the WAL-replay path instead of
            // rebuilding again. Without this, the freshly-rebuilt state stays
            // in memory until the periodic checkpoint (60s default) and a
            // crash before then forces another rebuild — a known time-sink on
            // read-mostly DBs.
            let watermark = self.watermark_tx_id();
            let flush_btree = index_manager.flush_btree_indexes(&db_path, watermark)?;
            let flush_ft = index_manager.flush_fulltext_indexes(watermark)?;
            let flush_fz = index_manager.flush_fuzzy_indexes(watermark)?;
            let flush_vec = vector_manager.flush_vector_indexes(&db_path, watermark)?;
            log_info!(
                "Post-rebuild flush for '{}': btree={} fulltext={} fuzzy={} vector={} (watermark={})",
                name,
                flush_btree,
                flush_ft,
                flush_fz,
                flush_vec,
                watermark
            );
        }

        // Startup migration: populate chunk_doc_mapping for legacy fulltext indexes
        Self::migrate_chunk_doc_mapping(&self.storage, &mut index_manager, name);

        Ok((index_manager, vector_manager))
    }

    /// Populate chunk_doc_mapping for legacy fulltext indexes that were saved
    /// before chunk_doc_mapping existed. No-op if the mapping is already populated.
    fn migrate_chunk_doc_mapping(
        storage: &Arc<RwLock<S>>,
        index_manager: &mut IndexManager,
        collection_name: &str,
    ) {
        let fts_needing_migration: Vec<String> = index_manager
            .list_fulltext_indexes()
            .iter()
            .filter(|idx| idx.doc_count() > 0 && !idx.has_chunk_doc_mapping())
            .map(|idx| idx.name.clone())
            .collect();

        if fts_needing_migration.is_empty() {
            return;
        }

        let catalog = {
            let guard = storage.read();
            guard
                .get_collection_meta(collection_name)
                .map(|m| m.document_catalog.clone())
        };

        let Some(catalog) = catalog else {
            return;
        };

        for index_name in &fts_needing_migration {
            let (indexed_doc_ids, pdid_field) =
                if let Some(fts) = index_manager.get_fulltext_index(index_name) {
                    let ids: std::collections::HashSet<crate::DocumentId> =
                        fts.doc_tokens_offsets_keys().into_iter().collect();
                    let field = fts.parent_doc_id_field().to_string();
                    (ids, field)
                } else {
                    continue;
                };

            let mut entries: Vec<(crate::DocumentId, String)> = Vec::new();
            let mut storage_guard = storage.write();

            for (doc_id, offset) in &catalog {
                if !indexed_doc_ids.contains(doc_id) {
                    continue;
                }
                if let Ok(bytes) = storage_guard.read_document_at(collection_name, *offset) {
                    if let Ok(doc) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                        if let Some(parent_doc_id) =
                            crate::value_utils::get_nested_value(&doc, &pdid_field)
                                .and_then(|v| v.as_str().map(|s| s.to_string()))
                        {
                            entries.push((doc_id.clone(), parent_doc_id));
                        }
                    }
                }
            }

            drop(storage_guard);

            if !entries.is_empty() {
                if let Some(fts) = index_manager.get_fulltext_index_mut(index_name) {
                    fts.populate_chunk_doc_mapping(entries);
                }
            }
        }
    }

    /// Get collection (creates if doesn't exist)
    ///
    /// Uses shared IndexManager and Schema to fix stale index/schema problems.
    /// Note: This method IMPLICITLY creates the collection if it doesn't exist.
    /// For read-only access without creation, use `get_collection()` instead.
    pub fn collection(&self, name: &str) -> Result<CollectionCore<S>> {
        self.check_not_closed()?;
        let (shared_indexes, shared_vectors) = self.ensure_managers(name)?;
        let shared_schema = self.get_or_create_schema_manager(name)?;
        CollectionCore::with_shared_indexes(
            name.to_string(),
            Arc::clone(&self.storage),
            shared_indexes,
            shared_vectors,
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
        // Fast path: check if index manager is cached (vector manager is in lockstep)
        let (shared_indexes, shared_vectors) = {
            let managers = self.index_managers.read();
            match managers.get(name) {
                Some(manager) => {
                    let vectors = self
                        .vector_managers
                        .read()
                        .get(name)
                        .map(Arc::clone)
                        .expect("vector_managers must be in lockstep with index_managers");
                    (Arc::clone(manager), vectors)
                }
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
                    let (shared_indexes, shared_vectors) = self.ensure_managers(name)?;
                    let shared_schema = self.get_or_create_schema_manager(name)?;
                    return CollectionCore::with_shared_indexes_readonly(
                        name.to_string(),
                        Arc::clone(&self.storage),
                        shared_indexes,
                        shared_vectors,
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
            shared_vectors,
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

        // Collect and delete index files BEFORE removing the IndexManager
        self.cleanup_index_files_for_collection(name);

        // Remove shared IndexManager, SchemaManager, and collection write lock
        self.index_managers.write().remove(name);
        self.vector_managers.write().remove(name);
        self.schema_managers.write().remove(name);
        self.collection_write_locks.write().remove(name);

        let mut storage = self.storage.write();
        storage.drop_collection(name)
    }

    /// Rename a collection, moving its metadata and on-disk index files from
    /// `old_name` to `new_name`.
    ///
    /// Document data is not copied — a `.mlite` is a single append-only file and
    /// the document catalog travels with the collection metadata. Index files
    /// are renamed in place (no rebuild) since index names are collection-keyed.
    /// The in-memory `IndexManager` for the old name is dropped so a fresh one
    /// reloads the renamed files + catalog on next access (this sidesteps
    /// surgically rewriting each index's internal lazy-load path and file handle).
    ///
    /// Ordering: the durable catalog update happens first; the on-disk file
    /// moves follow as best-effort. A crash or partial failure leaves the
    /// catalog under the new name, and any missing/misnamed index file is
    /// recreated by rebuild-from-catalog on the next open.
    pub fn rename_collection(&self, old_name: &str, new_name: &str) -> Result<()> {
        if old_name == new_name {
            return Ok(());
        }

        // Validate existence + protection, and capture the db file path. The
        // new-name conflict is also enforced by the storage layer.
        let db_path = {
            let storage = self.storage.read();
            match storage.get_collection_meta(old_name) {
                None => return Err(IronBaseError::CollectionNotFound(old_name.to_string())),
                Some(meta) if meta.flags.protected => {
                    return Err(IronBaseError::OperationNotAllowed(format!(
                        "Cannot rename protected collection '{}'",
                        old_name
                    )));
                }
                Some(_) => {}
            }
            if storage.get_collection_meta(new_name).is_some() {
                return Err(IronBaseError::CollectionExists(new_name.to_string()));
            }
            storage.get_file_path().to_string()
        };

        // Compute on-disk index-file moves from the *persisted* index lists,
        // before the catalog is renamed. Works whether or not the in-memory
        // IndexManager is currently loaded.
        let file_moves = self.collection_index_file_moves(old_name, new_name, &db_path);

        // 1) Durable catalog update (metadata + persisted index-name reprefix).
        {
            let mut storage = self.storage.write();
            storage.rename_collection(old_name, new_name)?;
        }

        // 2) Move index files on disk (best-effort — rebuild-from-catalog covers
        //    any gap on the next open).
        for (old_path, new_path) in &file_moves {
            if old_path.exists() {
                if let Err(e) = std::fs::rename(old_path, new_path) {
                    tracing::warn!(
                        "rename_collection: failed to move index file {:?} -> {:?}: {}",
                        old_path,
                        new_path,
                        e
                    );
                }
            }
        }

        // 3) Drop the stale in-memory IndexManager (forces a fresh reload from
        //    the renamed files + catalog), and move the schema + write lock.
        self.index_managers.write().remove(old_name);
        self.vector_managers.write().remove(old_name);
        {
            let mut schemas = self.schema_managers.write();
            if let Some(arc) = schemas.remove(old_name) {
                schemas.insert(new_name.to_string(), arc);
            }
        }
        {
            let mut locks = self.collection_write_locks.write();
            if let Some(arc) = locks.remove(old_name) {
                locks.insert(new_name.to_string(), arc);
            }
        }

        Ok(())
    }

    /// Compute the on-disk index-file moves for a collection rename from the
    /// persisted index metadata. Index names follow `{collection}_{suffix}`, so
    /// only the prefix changes; the vector `.hnsw` path is raw (not hashed).
    fn collection_index_file_moves(
        &self,
        old_name: &str,
        new_name: &str,
        db_path: &str,
    ) -> Vec<(std::path::PathBuf, std::path::PathBuf)> {
        use crate::collection_core::{
            build_btree_index_file_path, build_fulltext_index_file_path,
            build_fuzzy_index_file_path,
        };

        if db_path.is_empty() {
            return Vec::new();
        }
        let prefix = format!("{}_", old_name);
        let remap = |name: &str| -> String {
            match name.strip_prefix(&prefix) {
                Some(suffix) => format!("{}_{}", new_name, suffix),
                None => name.to_string(),
            }
        };

        let storage = self.storage.read();
        let meta = match storage.get_collection_meta(old_name) {
            Some(m) => m,
            None => return Vec::new(),
        };

        let mut moves: Vec<(std::path::PathBuf, std::path::PathBuf)> = Vec::new();
        for ix in &meta.indexes {
            if let (Some(o), Some(n)) = (
                build_btree_index_file_path(db_path, &ix.name),
                build_btree_index_file_path(db_path, &remap(&ix.name)),
            ) {
                moves.push((o, n));
            }
        }
        for ix in &meta.fulltext_indexes {
            if let (Some(o), Some(n)) = (
                build_fulltext_index_file_path(db_path, &ix.name),
                build_fulltext_index_file_path(db_path, &remap(&ix.name)),
            ) {
                moves.push((o, n));
            }
        }
        for ix in &meta.fuzzy_indexes {
            if let (Some(o), Some(n)) = (
                build_fuzzy_index_file_path(db_path, &ix.name),
                build_fuzzy_index_file_path(db_path, &remap(&ix.name)),
            ) {
                moves.push((o, n));
            }
        }
        for ix in &meta.vector_indexes {
            moves.push((
                crate::index::VectorIndexManager::build_vector_cache_path(db_path, &ix.name),
                crate::index::VectorIndexManager::build_vector_cache_path(
                    db_path,
                    &remap(&ix.name),
                ),
            ));
        }
        moves
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
        // Collect and delete index files BEFORE removing the IndexManager
        self.cleanup_index_files_for_collection(name);

        // Remove shared IndexManager, SchemaManager, and collection write lock
        self.index_managers.write().remove(name);
        self.vector_managers.write().remove(name);
        self.schema_managers.write().remove(name);
        self.collection_write_locks.write().remove(name);

        let mut storage = self.storage.write();
        storage.drop_collection(name)
    }

    /// Delete all index files (.idx, .ftidx, .fzidx, .hnsw) for a collection.
    ///
    /// Called BEFORE removing the IndexManager so we can still read index names.
    /// Best-effort: file deletion errors are logged but do not fail the drop.
    ///
    /// Uses the same path-building functions that flush/load use, ensuring
    /// consistent file naming.
    fn cleanup_index_files_for_collection(&self, collection_name: &str) {
        let db_path = {
            let storage = self.storage.read();
            storage.get_file_path().to_string()
        };
        if db_path.is_empty() {
            return; // MemoryStorage — no index files on disk
        }

        // Read index names from the IndexManager (if it exists for this collection)
        let index_managers = self.index_managers.read();
        let mgr_arc = match index_managers.get(collection_name) {
            Some(arc) => arc.clone(),
            None => return, // No IndexManager — nothing to clean up
        };
        drop(index_managers); // Release the outer map lock

        let mgr = mgr_arc.read();
        let index_names = mgr.list_indexes();
        drop(mgr);

        // Build and delete file paths for each index type
        for name in &index_names {
            // B+ tree (.idx)
            if let Some(path) = crate::collection_core::build_btree_index_file_path(&db_path, name)
            {
                Self::try_remove_file(&path, name, "idx");
            }
            // Fulltext (.ftidx)
            if let Some(path) =
                crate::collection_core::build_fulltext_index_file_path(&db_path, name)
            {
                Self::try_remove_file(&path, name, "ftidx");
            }
            // Fuzzy (.fzidx)
            if let Some(path) = crate::collection_core::build_fuzzy_index_file_path(&db_path, name)
            {
                Self::try_remove_file(&path, name, "fzidx");
            }
            // HNSW vector (.hnsw)
            let hnsw_path = VectorIndexManager::build_vector_cache_path(&db_path, name);
            Self::try_remove_file(&hnsw_path, name, "hnsw");
        }
    }

    /// Try to remove an index file. Logs on error, does not propagate.
    fn try_remove_file(path: &std::path::Path, index_name: &str, ext: &str) {
        if path.exists() {
            if let Err(e) = std::fs::remove_file(path) {
                tracing::warn!(
                    path = %path.display(),
                    index = %index_name,
                    ext = %ext,
                    error = %e,
                    "Failed to delete orphaned index file during drop_collection"
                );
            } else {
                tracing::debug!(
                    path = %path.display(),
                    index = %index_name,
                    ext = %ext,
                    "Deleted index file during drop_collection"
                );
            }
        }
    }
}
