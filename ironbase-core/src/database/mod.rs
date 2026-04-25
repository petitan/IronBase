//! Database Core Module
//!
//! This module provides the central database management API for IronBase.
//! It is language-independent (no PyO3 dependencies) and generic over storage backends.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                     DatabaseCore<S>                              │
//! │  ┌─────────────┐  ┌──────────────┐  ┌───────────────────────┐  │
//! │  │   Storage   │  │  Index       │  │  Transaction          │  │
//! │  │   (Arc)     │  │  Managers    │  │  Coordination         │  │
//! │  │             │  │  (per-coll)  │  │  (write locks)        │  │
//! │  └─────────────┘  └──────────────┘  └───────────────────────┘  │
//! └─────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Storage Backends
//!
//! - [`DatabaseCore<StorageEngine>`] - Production file-based storage with WAL
//! - [`DatabaseCore<MemoryStorage>`] - Fast in-memory storage for testing (10-100x faster)
//!
//! # Durability Modes
//!
//! - **Safe** (default): Every operation is immediately persisted (like SQL databases)
//! - **Batch**: Operations buffered until batch_size reached or explicit flush
//! - **Unsafe**: No auto-commit, manual checkpoint required
//!
//! # Thread Safety
//!
//! - Storage access: `Arc<RwLock<S>>` - shared read, exclusive write
//! - IndexManagers: Per-collection `Arc<RwLock<IndexManager>>` - prevents stale index state
//! - Write transactions: SQLite-style exclusive lock (one writer at a time)
//! - Collection writes: Fine-grained `Mutex` per collection for Safe mode atomicity
//!
//! # Submodules
//!
//! - [`collections`] - Collection creation, IndexManager lifecycle, warm-up
//! - [`durability`] - Durability mode implementation, batch flush
//! - [`maintenance`] - Flush, compact, close, checkpoint operations
//! - [`transactions`] - Transaction begin/commit/rollback, write lock management

mod collections;
mod durability;
mod maintenance;
mod transactions;

pub use durability::BatchFlush;

use parking_lot::{Condvar, Mutex, RwLock};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use crate::collection_core::schema::CompiledSchema;
use crate::collection_core::{
    DeleteManyPrepared, DeleteOnePreparedBatch, InsertOnePrepared, UpdateManyPrepared,
    UpdateOnePreparedBatch,
};
use crate::durability::DurabilityMode;
use crate::error::{IronBaseError, Result};
use crate::index::IndexManager;
use crate::storage::{MemoryStorage, RawStorage, Storage, StorageEngine};
use crate::transaction::{Operation, Transaction, TransactionId};

/// Maximum memory for batch document buffer (10MB safety limit)
/// When exceeded, triggers an early flush even if batch_size not reached
pub const MAX_BATCH_MEMORY_BYTES: usize = 10 * 1024 * 1024;

/// Buffer for documents prepared but not yet persisted in Batch mode
///
/// In Batch mode, operations are buffered here after prepare()
/// but BEFORE storage write. The actual persist happens in flush_batch()
/// AFTER WAL commit (fsync), ensuring proper WAL-first ordering.
///
/// WAL ORDERING FIX: Now includes updates and deletes, not just inserts.
#[derive(Debug, Default)]
pub struct BatchDocBuffer {
    /// Buffered insert operations by collection name
    pub(crate) inserts: HashMap<String, Vec<InsertOnePrepared>>,
    /// Buffered update operations by collection name (WAL ORDERING FIX)
    pub(crate) updates: HashMap<String, Vec<UpdateOnePreparedBatch>>,
    /// Buffered delete operations by collection name (WAL ORDERING FIX)
    pub(crate) deletes: HashMap<String, Vec<DeleteOnePreparedBatch>>,
    /// Buffered update_many operations by collection name (WAL ORDERING FIX for _many)
    pub(crate) update_many_ops: HashMap<String, Vec<UpdateManyPrepared>>,
    /// Buffered delete_many operations by collection name (WAL ORDERING FIX for _many)
    pub(crate) delete_many_ops: HashMap<String, Vec<DeleteManyPrepared>>,
    /// Approximate memory usage in bytes
    pub(crate) memory_bytes: usize,
}

impl BatchDocBuffer {
    /// Create a new empty buffer
    pub fn new() -> Self {
        Self {
            inserts: HashMap::new(),
            updates: HashMap::new(),
            deletes: HashMap::new(),
            update_many_ops: HashMap::new(),
            delete_many_ops: HashMap::new(),
            memory_bytes: 0,
        }
    }

    /// Check if the buffer is empty
    pub fn is_empty(&self) -> bool {
        self.inserts.is_empty()
            && self.updates.is_empty()
            && self.deletes.is_empty()
            && self.update_many_ops.is_empty()
            && self.delete_many_ops.is_empty()
    }

    /// Clear all buffered documents
    pub fn clear(&mut self) {
        self.inserts.clear();
        self.updates.clear();
        self.deletes.clear();
        self.update_many_ops.clear();
        self.delete_many_ops.clear();
        self.memory_bytes = 0;
    }

    /// Shrink internal buffers to release memory back to the allocator.
    pub fn shrink_to_fit(&mut self) {
        self.inserts.shrink_to_fit();
        self.updates.shrink_to_fit();
        self.deletes.shrink_to_fit();
        self.update_many_ops.shrink_to_fit();
        self.delete_many_ops.shrink_to_fit();
    }

    /// Clear buffers and optionally shrink if recent usage was high.
    pub fn clear_and_shrink_if_large(&mut self, previous_bytes: usize) {
        self.clear();
        if previous_bytes >= MAX_BATCH_MEMORY_BYTES {
            self.shrink_to_fit();
        }
    }

    /// Add a prepared insert to the buffer
    pub fn add_insert(&mut self, collection: String, prepared: InsertOnePrepared) {
        // Estimate memory usage (rough approximation)
        let doc_size = serde_json::to_string(&prepared.document)
            .map(|s| s.len())
            .unwrap_or(100);
        self.memory_bytes += doc_size;

        self.inserts.entry(collection).or_default().push(prepared);
    }

    /// Add a prepared update to the buffer (WAL ORDERING FIX)
    pub fn add_update(&mut self, collection: String, prepared: UpdateOnePreparedBatch) {
        // Estimate memory usage
        let doc_size = serde_json::to_string(&prepared.new_doc)
            .map(|s| s.len())
            .unwrap_or(100);
        self.memory_bytes += doc_size;

        self.updates.entry(collection).or_default().push(prepared);
    }

    /// Add a prepared delete to the buffer (WAL ORDERING FIX)
    pub fn add_delete(&mut self, collection: String, prepared: DeleteOnePreparedBatch) {
        // Estimate memory usage
        let doc_size = serde_json::to_string(&prepared.old_doc)
            .map(|s| s.len())
            .unwrap_or(100);
        self.memory_bytes += doc_size;

        self.deletes.entry(collection).or_default().push(prepared);
    }

    /// Add a prepared update_many to the buffer (WAL ORDERING FIX for _many)
    pub fn add_update_many(&mut self, collection: String, prepared: UpdateManyPrepared) {
        // Estimate memory usage from WAL entries
        for (_, _, new_doc) in &prepared.wal_entries {
            let doc_size = serde_json::to_string(new_doc)
                .map(|s| s.len())
                .unwrap_or(100);
            self.memory_bytes += doc_size;
        }

        self.update_many_ops
            .entry(collection)
            .or_default()
            .push(prepared);
    }

    /// Add a prepared delete_many to the buffer (WAL ORDERING FIX for _many)
    pub fn add_delete_many(&mut self, collection: String, prepared: DeleteManyPrepared) {
        // Estimate memory usage from tombstone writes
        for (_, tombstone_json) in &prepared.tombstone_writes {
            self.memory_bytes += tombstone_json.len();
        }

        self.delete_many_ops
            .entry(collection)
            .or_default()
            .push(prepared);
    }

    /// Check if memory limit exceeded
    pub fn memory_limit_exceeded(&self) -> bool {
        self.memory_bytes >= MAX_BATCH_MEMORY_BYTES
    }
}

// NOTE: Use DocumentId::extract_from_value() for _id extraction with Result
// This avoids code duplication across modules.

/// Pure Rust IronBase Database - language-independent
///
/// The central database struct that coordinates all operations: storage, indexing,
/// transactions, and durability. Generic over storage backend for flexibility.
///
/// # Type Parameters
///
/// * `S` - Storage backend implementing [`Storage`] + [`RawStorage`] traits
///   - [`StorageEngine`] - Production file-based storage with WAL and crash recovery
///   - [`MemoryStorage`] - Fast in-memory storage for testing (no persistence)
///
/// # Thread Safety Model
///
/// ```text
/// DatabaseCore fields and their locking:
/// ┌─────────────────────────┬─────────────────────────────────────────┐
/// │ Field                   │ Locking Strategy                        │
/// ├─────────────────────────┼─────────────────────────────────────────┤
/// │ storage                 │ Arc<RwLock> - read for queries, write   │
/// │                         │ for mutations                           │
/// ├─────────────────────────┼─────────────────────────────────────────┤
/// │ index_managers          │ Arc<RwLock<HashMap>> - per-collection   │
/// │                         │ IndexManager shared across instances    │
/// ├─────────────────────────┼─────────────────────────────────────────┤
/// │ write_transaction_lock  │ SQLite-style: only one write tx at a    │
/// │                         │ time, 5s timeout, Condvar notification  │
/// ├─────────────────────────┼─────────────────────────────────────────┤
/// │ collection_write_locks  │ Fine-grained Mutex per collection for   │
/// │                         │ Safe mode prepare-WAL-persist atomicity │
/// └─────────────────────────┴─────────────────────────────────────────┘
/// ```
///
/// # Durability Modes
///
/// | Mode   | Commit        | Performance   | Data Loss Risk |
/// |--------|---------------|---------------|----------------|
/// | Safe   | Every op      | ~1-5K ops/s   | None           |
/// | Batch  | Every N ops   | ~20-50K ops/s | Up to N ops    |
/// | Unsafe | Manual only   | ~50-100K ops/s| All uncommitted|
///
/// # Example
///
/// ```rust,no_run
/// use ironbase_core::{DatabaseCore, DurabilityMode};
/// use ironbase_core::storage::StorageEngine;
///
/// // Open with Safe mode (default)
/// let db = DatabaseCore::<StorageEngine>::open("app.mlite")?;
///
/// // Create/access collections
/// let users = db.collection("users")?;
///
/// // CRUD operations (auto-committed in Safe mode)
/// db.insert_one("users", [("name".to_string(), serde_json::json!("Alice"))].into())?;
/// # Ok::<(), ironbase_core::IronBaseError>(())
/// ```
#[allow(clippy::type_complexity)]
pub struct DatabaseCore<S: Storage + RawStorage> {
    pub(crate) storage: Arc<RwLock<S>>,
    pub(crate) db_path: String,
    pub(crate) next_tx_id: AtomicU64,
    /// Watermark for WAL-based index recovery: tx_id of the most recent
    /// transaction whose commit has been acknowledged (WAL fsynced and
    /// Operations applied to storage + in-memory indexes). Monotonically
    /// non-decreasing. Used by periodic index flushes to stamp
    /// `last_flushed_tx_id` into the index file metadata so that on
    /// crash recovery, only ops with `tx_id > watermark` need replay.
    pub(crate) max_committed_tx_id: AtomicU64,
    pub(crate) active_transactions:
        Arc<RwLock<std::collections::HashMap<TransactionId, Transaction>>>,

    // NEW: Durability mode (safe by default like SQL databases)
    pub(crate) durability_mode: DurabilityMode,

    // NEW: Batch buffer for Batch mode (WAL operations)
    pub(crate) batch_buffer: Arc<RwLock<Vec<Operation>>>,

    // NEW: Batch document buffer for Batch mode (documents prepared but not persisted)
    // This enables WAL-first ordering: prepare → buffer → WAL commit → persist
    pub(crate) batch_doc_buffer: Arc<RwLock<BatchDocBuffer>>,

    // NEW: Operation counter for Unsafe mode auto-checkpoint
    pub(crate) unsafe_op_counter: AtomicU64,

    // Shared IndexManagers per collection (fixes stale index problem)
    // Each collection shares its IndexManager across all CollectionCore instances
    pub(crate) index_managers: Arc<RwLock<HashMap<String, Arc<RwLock<IndexManager>>>>>,

    // Shared SchemaManagers per collection (fixes stale schema problem)
    // Each collection shares its CompiledSchema across all CollectionCore instances
    pub(crate) schema_managers: Arc<RwLock<HashMap<String, Arc<RwLock<Option<CompiledSchema>>>>>>,

    // Transaction-level exclusive write lock for Read Committed isolation
    // Only one write transaction can be active at a time (SQLite-style)
    // None = no active write transaction
    // Some(tx_id) = this transaction holds the exclusive write lock
    pub(crate) write_transaction_lock: Arc<Mutex<Option<TransactionId>>>,
    // Condvar notified on release_write_lock — waiters wake up instead of polling
    pub(crate) write_lock_condvar: Arc<Condvar>,

    // Flag to prevent operations after close() is called
    // Arc-wrapped so CollectionCore can share the same flag
    pub(crate) is_closed: Arc<AtomicBool>,

    // Collection-level write locks for Safe mode atomicity
    // Ensures prepare-WAL-persist sequence is atomic per collection
    // Prevents race conditions in unique constraint checks
    pub(crate) collection_write_locks: Arc<RwLock<HashMap<String, Arc<Mutex<()>>>>>,

    // Guard: true while compact_nonblocking() is running
    // Prevents concurrent compaction from direct Rust consumers
    // (MCP server has its own adapter-level guard; this protects core-level callers)
    pub(crate) is_compacting: AtomicBool,

    // Last compaction result size (size_after from CompactionStats)
    // Used by storage_wastage() to estimate bloat ratio.
    // Starts at 0 (never compacted) — bloat_ratio = infinity until first compact.
    pub(crate) last_compact_size: AtomicU64,

    /// WAL-recovered operations awaiting per-collection non-btree index replay,
    /// **grouped by collection name** for O(1) lookup and drain-style consumption.
    ///
    /// Populated once by `open_with_durability()` from
    /// `storage.recover_from_wal()` — the ungrouped `Vec<(tx_id, Op)>` is
    /// walked once and each entry routed into its collection's bucket.
    /// Consumed by `initialize_index_manager()` via
    /// `take_recovered_operations_for_collection(name)`, which **removes**
    /// the bucket from the map so the memory is freed as each collection
    /// finishes its replay.
    ///
    /// Empty after clean shutdown or MemoryStorage (no WAL replay).
    pub(crate) recovered_operations: Arc<RwLock<HashMap<String, Vec<(TransactionId, Operation)>>>>,
}

// ============================================================================
// STORAGEENGINE-SPECIFIC IMPLEMENTATION (WAL recovery)
// ============================================================================

impl DatabaseCore<StorageEngine> {
    /// Open or create database with StorageEngine (production)
    ///
    /// Uses Safe durability mode by default (like SQL databases).
    /// For other modes, use `open_with_durability()`.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        Self::open_with_durability(path, DurabilityMode::default())
    }

    /// Open or create database with explicit durability mode
    ///
    /// # Arguments
    /// * `path` - Database file path
    /// * `mode` - Durability mode (Safe, Batch, or Unsafe)
    ///
    /// # Examples
    /// ```rust,no_run
    /// use ironbase_core::{DatabaseCore, DurabilityMode};
    /// use ironbase_core::storage::StorageEngine;
    ///
    /// // Safe mode (default, like SQL databases)
    /// let db = DatabaseCore::<StorageEngine>::open_with_durability(
    ///     "app.mlite",
    ///     DurabilityMode::Safe
    /// )?;
    ///
    /// // Batch mode (good balance)
    /// let db = DatabaseCore::<StorageEngine>::open_with_durability(
    ///     "app.mlite",
    ///     DurabilityMode::Batch { batch_size: 100 }
    /// )?;
    ///
    /// // Unsafe mode - manual checkpoint only
    /// let db = DatabaseCore::<StorageEngine>::open_with_durability(
    ///     "app.mlite",
    ///     DurabilityMode::unsafe_manual()
    /// )?;
    ///
    /// // Unsafe mode - auto checkpoint every 10000 ops
    /// let db = DatabaseCore::<StorageEngine>::open_with_durability(
    ///     "app.mlite",
    ///     DurabilityMode::unsafe_auto(10000)
    /// )?;
    /// # Ok::<(), ironbase_core::IronBaseError>(())
    /// ```
    pub fn open_with_durability<P: AsRef<Path>>(path: P, mode: DurabilityMode) -> Result<Self> {
        let path_str = path.as_ref().to_string_lossy().to_string();

        let mut storage = StorageEngine::open(&path_str)?;

        // Recover from WAL (includes both data and index changes + applied ops for per-index replay)
        let (recovered_tx_count, recovered_index_changes, recovered_ops) =
            storage.recover_from_wal()?;

        // Group recovered ops by collection once so each collection's index
        // init is O(1) lookup instead of O(N) filter, and the buckets can
        // be drained individually to free memory as we go.
        let mut recovered_ops_by_collection: HashMap<String, Vec<(TransactionId, Operation)>> =
            HashMap::new();
        for (tx_id, op) in recovered_ops {
            let collection = crate::recovery::collection_of(&op).to_string();
            recovered_ops_by_collection
                .entry(collection)
                .or_default()
                .push((tx_id, op));
        }

        // CRITICAL FIX: Flush metadata after WAL recovery to persist updated data_end_offset
        //
        // Scenario without this fix:
        // 1. flush() writes MetadataSnapshot to WAL, flush_metadata(), wal.clear()
        // 2. New writes update data_end_offset IN MEMORY ONLY
        // 3. Crash (no flush happened)
        // 4. Restart: load_metadata() succeeds (file metadata intact) → no WAL metadata recovery
        // 5. recover_from_wal() replays ops, updates data_end_offset in memory, then wal.clear()
        // 6. Second crash before next flush
        // 7. Restart: WAL is EMPTY, file has OLD data_end_offset → CORRUPTION!
        //
        // The fix: flush_metadata() after WAL recovery ensures data_end_offset is persisted
        // before clearing the WAL. (Bug found 2024-12-26)
        if recovered_tx_count > 0 {
            storage.flush_metadata()?;
        }

        // NOTE: WAL recovery now uses write_document() which updates the catalog.
        // The document_catalog is loaded from metadata by StorageEngine::open(),
        // and recover_from_wal() properly updates it for any recovered operations.

        // Create DatabaseCore instance with specified mode
        let db = DatabaseCore {
            storage: Arc::new(RwLock::new(storage)),
            db_path: path_str,
            next_tx_id: AtomicU64::new(1),
            max_committed_tx_id: AtomicU64::new(0),
            active_transactions: Arc::new(RwLock::new(std::collections::HashMap::new())),
            durability_mode: mode,
            batch_buffer: Arc::new(RwLock::new(Vec::new())),
            batch_doc_buffer: Arc::new(RwLock::new(BatchDocBuffer::new())),
            unsafe_op_counter: AtomicU64::new(0),
            index_managers: Arc::new(RwLock::new(HashMap::new())),
            schema_managers: Arc::new(RwLock::new(HashMap::new())),
            write_transaction_lock: Arc::new(Mutex::new(None)),
            write_lock_condvar: Arc::new(Condvar::new()),
            is_closed: Arc::new(AtomicBool::new(false)),
            collection_write_locks: Arc::new(RwLock::new(HashMap::new())),
            is_compacting: AtomicBool::new(false),
            last_compact_size: AtomicU64::new(0),
            recovered_operations: Arc::new(RwLock::new(recovered_ops_by_collection)),
        };

        // Apply recovered index changes to collections
        // Group index changes by collection name
        let mut changes_by_collection: HashMap<String, Vec<crate::storage::RecoveredIndexChange>> =
            HashMap::new();

        for change in recovered_index_changes {
            // Group by collection name (now properly included in RecoveredIndexChange)
            changes_by_collection
                .entry(change.collection.clone())
                .or_default()
                .push(change);
        }

        // Apply changes to each collection's indexes
        for (collection_name, changes) in changes_by_collection {
            // Get collection (creates if doesn't exist)
            if let Ok(collection) = db.collection(&collection_name) {
                for change in changes {
                    // Apply the index change to the collection's indexes
                    let mut indexes = collection.indexes.write();
                    if let Some(btree_index) = indexes.get_btree_index_mut(&change.index_name) {
                        match change.operation {
                            crate::transaction::IndexOperation::Insert => {
                                btree_index.insert(change.key.clone(), change.doc_id)?;
                            }
                            crate::transaction::IndexOperation::Delete => {
                                btree_index.delete(&change.key, &change.doc_id)?;
                            }
                        }
                    }
                    // Mark dirty so WAL-replayed changes are persisted at next checkpoint
                    indexes.mark_btree_dirty(&change.index_name);
                }
            }
        }

        Ok(db)
    }

    /// Simulate an unclean (kill -9) shutdown. **Test-only.**
    ///
    /// Releases the `StorageEngine` file lock so the same process can reopen
    /// the database, marks the in-memory handle as closed so `Drop` is a no-op,
    /// then leaks `self` so `StorageEngine::Drop` also does not run. The
    /// on-disk `clean_shutdown` byte therefore stays at its pre-session value
    /// (false after any prior write or first open), making the next open see
    /// `was_clean_shutdown() == false` and take the recovery path.
    ///
    /// Gated on `#[cfg(test)]` so the symbol never appears in a production
    /// binary — previous incarnations (`__simulate_crash_for_test`,
    /// `#[doc(hidden)] pub`) were reachable from callers of this crate
    /// outside of test builds, which deliberately leaks a `StorageEngine`
    /// and can keep a file lock alive for the rest of the process.
    #[cfg(test)]
    fn simulate_crash_for_test(self) {
        self.is_closed.store(true, Ordering::SeqCst);
        if let Some(storage) = self.storage.try_read() {
            let _ = storage.release_lock();
        }
        std::mem::forget(self);
    }
}

// ============================================================================
// MEMORYSTORAGE-SPECIFIC IMPLEMENTATION (in-memory, no WAL)
// ============================================================================

impl DatabaseCore<MemoryStorage> {
    /// Create an in-memory database (for testing)
    ///
    /// This provides a fast, ephemeral database that doesn't persist to disk.
    /// Perfect for unit tests where you don't need data to survive restarts.
    ///
    /// # Performance
    ///
    /// - **10-100x faster** than file-based storage
    /// - No file I/O overhead
    /// - No WAL recovery needed
    ///
    /// # Examples
    ///
    /// ```rust
    /// use ironbase_core::DatabaseCore;
    /// use ironbase_core::storage::MemoryStorage;
    ///
    /// let db = DatabaseCore::<MemoryStorage>::open_memory()?;
    ///
    /// // Use DatabaseCore methods for CRUD with durability
    /// db.insert_one("users", std::collections::HashMap::from([
    ///     ("name".to_string(), serde_json::json!("Alice")),
    /// ]))?;
    ///
    /// let users = db.collection("users")?;
    /// let count = users.count_documents(&serde_json::json!({}))?;
    /// assert_eq!(count, 1);
    /// # Ok::<(), ironbase_core::IronBaseError>(())
    /// ```
    pub fn open_memory() -> Result<Self> {
        let storage = MemoryStorage::new();

        Ok(DatabaseCore {
            storage: Arc::new(RwLock::new(storage)),
            db_path: String::new(), // No file path for memory storage
            next_tx_id: AtomicU64::new(1),
            max_committed_tx_id: AtomicU64::new(0),
            active_transactions: Arc::new(RwLock::new(std::collections::HashMap::new())),
            durability_mode: DurabilityMode::default(),
            batch_buffer: Arc::new(RwLock::new(Vec::new())),
            batch_doc_buffer: Arc::new(RwLock::new(BatchDocBuffer::new())),
            unsafe_op_counter: AtomicU64::new(0),
            index_managers: Arc::new(RwLock::new(HashMap::new())),
            schema_managers: Arc::new(RwLock::new(HashMap::new())),
            write_transaction_lock: Arc::new(Mutex::new(None)),
            write_lock_condvar: Arc::new(Condvar::new()),
            is_closed: Arc::new(AtomicBool::new(false)),
            collection_write_locks: Arc::new(RwLock::new(HashMap::new())),
            is_compacting: AtomicBool::new(false),
            last_compact_size: AtomicU64::new(0),
            recovered_operations: Arc::new(RwLock::new(HashMap::new())),
        })
    }
}

// ============================================================================
// GENERIC IMPLEMENTATION (all storage backends)
// ============================================================================

impl<S: Storage + RawStorage> DatabaseCore<S> {
    /// Check if database is closed, return error if so
    pub(crate) fn check_not_closed(&self) -> Result<()> {
        if self.is_closed.load(Ordering::SeqCst) {
            return Err(IronBaseError::DatabaseClosed);
        }
        Ok(())
    }

    /// Current watermark tx_id for WAL-based index recovery.
    ///
    /// Returns the `transaction_id` of the most recent transaction whose
    /// commit is durable (WAL fsynced, Operations applied to storage +
    /// in-memory indexes). Monotonically non-decreasing.
    ///
    /// Index flushes stamp this value into their file metadata as
    /// `last_flushed_tx_id`. On crash recovery, only committed Operations
    /// with `tx_id > last_flushed_tx_id` need replay into the index.
    pub fn watermark_tx_id(&self) -> u64 {
        self.max_committed_tx_id.load(Ordering::SeqCst)
    }

    /// Take (remove and return) the WAL-recovered operations for a single
    /// collection, freeing the bucket inside `recovered_operations`.
    ///
    /// Called once per collection by `initialize_index_manager`. The
    /// drain-style transfer means ops for a collection that has already
    /// finished its replay no longer occupy memory while later collections
    /// are still being initialised — critical because `Operation::Insert`
    /// carries the full document JSON. Returns an empty Vec when the
    /// collection had no pending ops (clean shutdown, or no writes to
    /// this collection since the last checkpoint).
    pub(crate) fn take_recovered_operations_for_collection(
        &self,
        collection: &str,
    ) -> Vec<(TransactionId, Operation)> {
        self.recovered_operations
            .write()
            .remove(collection)
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collection_core::RawOperations;
    use crate::document::DocumentId;
    use crate::transaction::Operation;
    use serde_json::json;
    use tempfile::TempDir;

    #[test]
    fn test_begin_transaction() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");
        let db = DatabaseCore::open(&db_path).unwrap();

        let tx_id = db.begin_transaction();
        assert_eq!(tx_id, 1);

        let tx_id2 = db.begin_transaction();
        assert_eq!(tx_id2, 2);

        // Verify transaction is in active list
        let tx = db.get_transaction(tx_id);
        assert!(tx.is_some());
        assert_eq!(tx.unwrap().id, tx_id);
    }

    #[test]
    fn test_commit_empty_transaction() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");
        let db = DatabaseCore::open(&db_path).unwrap();

        let tx_id = db.begin_transaction();

        // Commit empty transaction
        let result = db.commit_transaction(tx_id);
        assert!(result.is_ok());

        // Transaction should be removed from active list
        let tx = db.get_transaction(tx_id);
        assert!(tx.is_none());
    }

    #[test]
    fn test_rollback_transaction() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");
        let db = DatabaseCore::open(&db_path).unwrap();

        let tx_id = db.begin_transaction();

        // Add an operation
        let mut tx = db.get_transaction(tx_id).unwrap();
        tx.add_operation(Operation::Insert {
            collection: "users".to_string(),
            doc_id: DocumentId::Int(1),
            doc: json!({"name": "Alice"}),
        })
        .unwrap();
        db.update_transaction(tx_id, tx).unwrap();

        // Rollback
        let result = db.rollback_transaction(tx_id);
        assert!(result.is_ok());

        // Transaction should be removed from active list
        let tx = db.get_transaction(tx_id);
        assert!(tx.is_none());
    }

    #[test]
    fn test_commit_with_operations() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");
        let db = DatabaseCore::open(&db_path).unwrap();

        // Create collection first
        db.collection("users").unwrap();

        let tx_id = db.begin_transaction();

        // Add operations
        let mut tx = db.get_transaction(tx_id).unwrap();
        tx.add_operation(Operation::Insert {
            collection: "users".to_string(),
            doc_id: DocumentId::Int(1),
            doc: json!({"name": "Alice", "age": 30}),
        })
        .unwrap();
        tx.add_operation(Operation::Insert {
            collection: "users".to_string(),
            doc_id: DocumentId::Int(2),
            doc: json!({"name": "Bob", "age": 25}),
        })
        .unwrap();
        db.update_transaction(tx_id, tx).unwrap();

        // Commit
        let result = db.commit_transaction(tx_id);
        assert!(result.is_ok());
    }

    #[test]
    fn test_commit_nonexistent_transaction() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");
        let db = DatabaseCore::open(&db_path).unwrap();

        // Try to commit non-existent transaction
        let result = db.commit_transaction(999);
        assert!(result.is_err());
    }

    // ========== Two-Phase Commit Tests ==========

    #[test]
    fn test_commit_with_indexes_basic() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");
        let db = DatabaseCore::open(&db_path).unwrap();

        // Create collection and index
        let collection = db.collection("users").unwrap();
        collection
            .create_index("age".to_string(), false, false)
            .unwrap();

        // Begin transaction
        let tx_id = db.begin_transaction();

        // Add insert operation with index change
        db.with_transaction(tx_id, |tx| {
            tx.add_operation(Operation::Insert {
                collection: "users".to_string(),
                doc_id: DocumentId::Int(1),
                doc: json!({"name": "Alice", "age": 30}),
            })?;

            // Track index change
            tx.add_index_change(
                "users_age".to_string(),
                crate::transaction::IndexChange {
                    operation: crate::transaction::IndexOperation::Insert,
                    key: crate::index::IndexKey::Int(30),
                    doc_id: DocumentId::Int(1),
                },
            )?;

            Ok(())
        })
        .unwrap();

        // Commit with indexes
        let result = db.commit_transaction(tx_id);
        assert!(result.is_ok());

        // Verify transaction removed from active list
        assert!(db.get_transaction(tx_id).is_none());
    }

    #[test]
    fn test_commit_with_indexes_no_index_changes() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");
        let db = DatabaseCore::open(&db_path).unwrap();

        // Create collection
        db.collection("users").unwrap();

        // Begin transaction
        let tx_id = db.begin_transaction();

        // Add operation WITHOUT index changes
        db.with_transaction(tx_id, |tx| {
            tx.add_operation(Operation::Insert {
                collection: "users".to_string(),
                doc_id: DocumentId::Int(1),
                doc: json!({"name": "Bob"}),
            })?;
            Ok(())
        })
        .unwrap();

        // Commit with indexes (should delegate to simple commit)
        let result = db.commit_transaction(tx_id);
        assert!(result.is_ok());
    }

    #[test]
    fn test_commit_with_indexes_nonexistent_transaction() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");
        let db = DatabaseCore::open(&db_path).unwrap();

        // Try to commit non-existent transaction
        let result = db.commit_transaction(999);
        assert!(result.is_err());

        // Should be TransactionAborted error
        match result {
            Err(crate::error::IronBaseError::TransactionAborted(_)) => {}
            _ => panic!("Expected TransactionAborted error"),
        }
    }

    #[test]
    fn test_get_index_file_path() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("mydb.mlite");
        let db = DatabaseCore::open(&db_path).unwrap();

        let path = db.get_index_file_path("users", "users_age");

        // Verify path format: {db_path_without_ext}.{index_name}.idx
        let expected = temp_dir.path().join("mydb.users_age.idx");
        assert_eq!(path, expected);
    }

    #[test]
    fn test_get_collection_from_transaction() {
        let mut transaction = crate::transaction::Transaction::new(1);

        // Add insert operation
        transaction
            .add_operation(Operation::Insert {
                collection: "users".to_string(),
                doc_id: DocumentId::Int(1),
                doc: json!({"name": "Alice"}),
            })
            .unwrap();

        // Extract collection name
        let collection_name = DatabaseCore::get_collection_from_transaction(&transaction);
        assert_eq!(collection_name, Some("users".to_string()));
    }

    #[test]
    fn test_get_collection_from_empty_transaction() {
        let transaction = crate::transaction::Transaction::new(1);

        // Empty transaction has no operations
        let collection_name = DatabaseCore::get_collection_from_transaction(&transaction);
        assert_eq!(collection_name, None);
    }

    // ========== MemoryStorage Tests ==========

    #[test]
    fn test_open_memory() {
        let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();

        // Should be able to create collections
        let coll = db.collection("users").unwrap();

        // And insert documents
        let doc = std::collections::HashMap::from([("name".to_string(), json!("Alice"))]);
        let id = coll.insert_one_raw(doc).unwrap();
        assert!(matches!(id, DocumentId::Int(_)));
    }

    #[test]
    fn test_memory_crud_operations() {
        let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
        let coll = db.collection("users").unwrap();

        // Insert
        let doc = std::collections::HashMap::from([
            ("name".to_string(), json!("Alice")),
            ("age".to_string(), json!(30)),
        ]);
        let id = coll.insert_one_raw(doc).unwrap();

        // Find
        let found = coll.find_one(&json!({"_id": id})).unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap()["name"], "Alice");

        // Update
        coll.update_one_raw(&json!({"_id": id}), &json!({"$set": {"age": 31}}))
            .unwrap();
        let updated = coll.find_one(&json!({"_id": id})).unwrap().unwrap();
        assert_eq!(updated["age"], 31);

        // Delete
        coll.delete_one_raw(&json!({"_id": id})).unwrap();
        let count = coll.count_documents(&json!({})).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_memory_multiple_collections() {
        let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();

        let users = db.collection("users").unwrap();
        let posts = db.collection("posts").unwrap();

        users
            .insert_one_raw(std::collections::HashMap::from([(
                "name".to_string(),
                json!("Alice"),
            )]))
            .unwrap();
        posts
            .insert_one_raw(std::collections::HashMap::from([(
                "title".to_string(),
                json!("Hello"),
            )]))
            .unwrap();

        assert_eq!(users.count_documents(&json!({})).unwrap(), 1);
        assert_eq!(posts.count_documents(&json!({})).unwrap(), 1);

        let collections = db.list_collections();
        assert_eq!(collections.len(), 2);
    }

    #[test]
    fn test_memory_aggregation() {
        let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
        let coll = db.collection("sales").unwrap();

        for (city, amount) in &[("NYC", 100), ("LA", 200), ("NYC", 150), ("LA", 50)] {
            coll.insert_one_raw(std::collections::HashMap::from([
                ("city".to_string(), json!(city)),
                ("amount".to_string(), json!(amount)),
            ]))
            .unwrap();
        }

        let results = coll
            .aggregate(&json!([
                {"$group": {"_id": "$city", "total": {"$sum": "$amount"}}}
            ]))
            .unwrap();

        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_memory_index() {
        let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
        let coll = db.collection("users").unwrap();

        // Create index
        let index_name = coll.create_index("age".to_string(), false, false).unwrap();
        assert!(index_name.contains("age"));

        // Insert with index
        for i in 0..10 {
            coll.insert_one_raw(std::collections::HashMap::from([(
                "age".to_string(),
                json!(i * 10),
            )]))
            .unwrap();
        }

        // Query using index
        let results = coll.find(&json!({"age": {"$gte": 50}})).unwrap();
        assert_eq!(results.len(), 5);
    }

    // ==================== System Collections Tests ====================

    #[test]
    fn test_system_collection_protected_from_drop() {
        let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();

        // Create system collection
        db.create_system_collection("_system.api_keys").unwrap();

        // Try to drop - should fail
        let result = db.drop_collection("_system.api_keys");
        assert!(result.is_err());

        // Verify error message
        let err = result.unwrap_err();
        assert!(err.to_string().contains("protected"));
    }

    #[test]
    fn test_hidden_collection_not_in_list() {
        let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();

        // Create normal collection
        db.collection("users").unwrap();

        // Create hidden collection
        db.collection("_internal").unwrap();
        db.set_collection_flags(
            "_internal",
            crate::storage::CollectionFlags {
                is_system: false,
                protected: false,
                hidden: true,
            },
        )
        .unwrap();

        // list_collections should only show "users"
        let visible = db.list_collections();
        assert_eq!(visible.len(), 1);
        assert!(visible.contains(&"users".to_string()));
        assert!(!visible.contains(&"_internal".to_string()));

        // list_all_collections should show both
        let all = db.list_all_collections();
        assert_eq!(all.len(), 2);
        assert!(all.contains(&"_internal".to_string()));
    }

    #[test]
    fn test_system_collection_hidden_by_default() {
        let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();

        // Create system collection (hidden = true by default)
        db.create_system_collection("_system.scripts").unwrap();

        // System collections are NOT visible in list_collections by default
        let visible = db.list_collections();
        assert!(!visible.contains(&"_system.scripts".to_string()));

        // But visible in list_all_collections
        let all = db.list_all_collections();
        assert!(all.contains(&"_system.scripts".to_string()));
    }

    #[test]
    fn test_normal_collection_can_be_dropped() {
        let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();

        // Create normal collection
        db.collection("temp").unwrap();

        // Drop should succeed
        let result = db.drop_collection("temp");
        assert!(result.is_ok());

        // Collection should be gone
        let collections = db.list_collections();
        assert!(!collections.contains(&"temp".to_string()));
    }

    // ========== Write Lock Isolation Tests ==========

    #[test]
    fn test_write_lock_acquired_on_first_write() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");
        let db = DatabaseCore::open(&db_path).unwrap();

        let tx1 = db.begin_transaction();

        // No lock held initially
        assert!(!db.holds_write_lock(tx1));
        assert!(!db.has_active_write_transaction());

        // First write acquires lock
        let doc = HashMap::from([("name".to_string(), json!("Alice"))]);
        db.insert_one_tx("users", doc, tx1).unwrap();

        // Now lock is held
        assert!(db.holds_write_lock(tx1));
        assert!(db.has_active_write_transaction());
        assert_eq!(db.get_write_lock_holder(), Some(tx1));

        db.commit_transaction(tx1).unwrap();

        // Lock released after commit
        assert!(!db.has_active_write_transaction());
    }

    #[test]
    fn test_second_write_transaction_waits_and_succeeds() {
        use std::sync::Arc;
        use std::thread;
        use std::time::Duration;

        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");
        let db = Arc::new(DatabaseCore::open(&db_path).unwrap());

        let tx1 = db.begin_transaction();

        // tx1 acquires write lock
        let doc1 = HashMap::from([("name".to_string(), json!("Alice"))]);
        db.insert_one_tx("users", doc1, tx1).unwrap();

        // Start tx2 in another thread - it will wait for tx1 to release lock
        let db2 = Arc::clone(&db);
        let tx2 = db.begin_transaction();
        let handle = thread::spawn(move || {
            let doc2 = HashMap::from([("name".to_string(), json!("Bob"))]);
            // This should block until tx1 commits
            let result = db2.insert_one_tx("users", doc2, tx2);
            (result.is_ok(), tx2)
        });

        // Wait a bit to ensure tx2 is waiting
        thread::sleep(Duration::from_millis(50));

        // tx1 commits, releasing the lock
        db.commit_transaction(tx1).unwrap();

        // tx2 should now succeed
        let (tx2_success, tx2) = handle.join().unwrap();
        assert!(tx2_success, "tx2 should succeed after tx1 commits");

        // Commit tx2
        db.commit_transaction(tx2).unwrap();

        // Verify both documents exist
        let coll = db.collection("users").unwrap();
        assert_eq!(coll.count_documents(&json!({})).unwrap(), 2);
    }

    #[test]
    fn test_write_lock_timeout() {
        use std::sync::Arc;
        use std::time::{Duration, Instant};

        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");
        let db = Arc::new(DatabaseCore::open(&db_path).unwrap());

        let tx1 = db.begin_transaction();

        // tx1 acquires write lock
        let doc1 = HashMap::from([("name".to_string(), json!("Alice"))]);
        db.insert_one_tx("users", doc1, tx1).unwrap();

        // tx2 should timeout waiting for lock (using short timeout)
        let tx2 = db.begin_transaction();
        let start = Instant::now();
        let result = db.acquire_write_lock_with_timeout(tx2, Duration::from_millis(100));

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("Timeout waiting for write lock"));

        // Should have waited approximately 100ms
        let elapsed = start.elapsed();
        assert!(
            elapsed >= Duration::from_millis(90),
            "Should wait at least 90ms"
        );
        assert!(
            elapsed < Duration::from_millis(200),
            "Should not wait more than 200ms"
        );

        // Cleanup
        db.commit_transaction(tx1).unwrap();
    }

    #[test]
    fn test_write_lock_released_on_rollback() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");
        let db = DatabaseCore::open(&db_path).unwrap();

        let tx1 = db.begin_transaction();

        // tx1 acquires write lock
        let doc = HashMap::from([("name".to_string(), json!("Alice"))]);
        db.insert_one_tx("users", doc, tx1).unwrap();
        assert!(db.has_active_write_transaction());

        // Rollback releases lock
        db.rollback_transaction(tx1).unwrap();
        assert!(!db.has_active_write_transaction());

        // New transaction can write now
        let tx2 = db.begin_transaction();
        let doc2 = HashMap::from([("name".to_string(), json!("Bob"))]);
        db.insert_one_tx("users", doc2, tx2).unwrap();
        db.commit_transaction(tx2).unwrap();
    }

    #[test]
    fn test_same_transaction_can_write_multiple_times() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");
        let db = DatabaseCore::open(&db_path).unwrap();

        let tx1 = db.begin_transaction();

        // Multiple writes in same transaction should work
        let doc1 = HashMap::from([("name".to_string(), json!("Alice"))]);
        db.insert_one_tx("users", doc1, tx1).unwrap();

        let doc2 = HashMap::from([("name".to_string(), json!("Bob"))]);
        db.insert_one_tx("users", doc2, tx1).unwrap();

        let doc3 = HashMap::from([("name".to_string(), json!("Charlie"))]);
        db.insert_one_tx("users", doc3, tx1).unwrap();

        db.update_one_tx(
            "users",
            &json!({"name": "Alice"}),
            json!({"$set": {"age": 30}}),
            tx1,
        )
        .unwrap();

        db.commit_transaction(tx1).unwrap();

        // Verify result - all 3 should be inserted
        let coll = db.collection("users").unwrap();
        assert_eq!(coll.count_documents(&json!({})).unwrap(), 3);
    }

    #[test]
    fn test_read_operations_dont_need_lock() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");
        let db = DatabaseCore::open(&db_path).unwrap();

        // Insert initial data (committed)
        let doc = HashMap::from([("name".to_string(), json!("Alice"))]);
        db.insert_one("users", doc).unwrap();

        let tx1 = db.begin_transaction();

        // tx1 acquires write lock and adds to buffer
        let doc2 = HashMap::from([("name".to_string(), json!("Bob"))]);
        db.insert_one_tx("users", doc2, tx1).unwrap();

        // Read operations should work and only see committed data
        // (tx operations are buffered, not yet committed)
        let coll = db.collection("users").unwrap();
        let count = coll.count_documents(&json!({})).unwrap();
        // Should only see Alice (Bob is not yet committed - Read Committed isolation!)
        assert_eq!(count, 1);

        // After commit, Bob becomes visible
        db.commit_transaction(tx1).unwrap();
        let count_after = coll.count_documents(&json!({})).unwrap();
        assert_eq!(count_after, 2);
    }

    #[test]
    fn test_auto_commit_waits_for_transaction() {
        use std::sync::Arc;
        use std::thread;

        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");
        let db = Arc::new(DatabaseCore::open(&db_path).unwrap());

        // Start a transaction and acquire write lock
        let tx1 = db.begin_transaction();
        let doc1 = HashMap::from([("name".to_string(), json!("Alice"))]);
        db.insert_one_tx("users", doc1, tx1).unwrap();

        // Auto-commit waits for transaction to complete
        let db_clone = Arc::clone(&db);
        let handle = thread::spawn(move || {
            // This will wait for tx1 to commit
            let doc2 = HashMap::from([("name".to_string(), json!("Bob"))]);
            db_clone.insert_one("users", doc2)
        });

        // Small delay, then commit tx1
        thread::sleep(std::time::Duration::from_millis(50));
        db.commit_transaction(tx1).unwrap();

        // Auto-commit should succeed now
        let result = handle.join().unwrap();
        assert!(result.is_ok());

        // Verify - both Alice and Bob are inserted
        let coll = db.collection("users").unwrap();
        assert_eq!(coll.count_documents(&json!({})).unwrap(), 2);
    }

    #[test]
    fn test_auto_commit_timeout_during_long_transaction() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");
        let db = DatabaseCore::open(&db_path).unwrap();

        // Start a transaction and acquire write lock
        let tx1 = db.begin_transaction();
        let doc1 = HashMap::from([("name".to_string(), json!("Alice"))]);
        db.insert_one_tx("users", doc1, tx1).unwrap();

        // Auto-commit will timeout (default 5s) - we'll use custom timeout via different method
        // For this test, we just verify the transaction blocks other writes
        assert!(db.has_active_write_transaction());

        // Commit tx1
        db.commit_transaction(tx1).unwrap();

        // Now auto-commit should work
        let doc2 = HashMap::from([("name".to_string(), json!("Bob"))]);
        assert!(db.insert_one("users", doc2).is_ok());

        // Verify
        let coll = db.collection("users").unwrap();
        assert_eq!(coll.count_documents(&json!({})).unwrap(), 2);
    }

    // ========== Collection Existence Tests ==========

    #[test]
    fn test_get_collection_not_found() {
        use crate::storage::MemoryStorage;

        let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
        let result = db.get_collection("nonexistent");
        assert!(matches!(result, Err(IronBaseError::CollectionNotFound(_))));
    }

    #[test]
    fn test_collection_implicit_create() {
        use crate::storage::MemoryStorage;

        let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
        // collection() still creates implicitly
        let _coll = db.collection("new_coll").unwrap();
        assert!(db.collection_exists("new_coll"));
    }

    #[test]
    fn test_collection_exists_after_insert() {
        use crate::storage::MemoryStorage;

        let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
        assert!(!db.collection_exists("users"));

        // Insert creates collection implicitly
        let doc = HashMap::from([("name".to_string(), json!("Alice"))]);
        db.insert_one("users", doc).unwrap();

        assert!(db.collection_exists("users"));
        // Now get_collection should work
        assert!(db.get_collection("users").is_ok());
    }

    #[test]
    fn test_update_on_nonexistent_collection_fails() {
        use crate::storage::MemoryStorage;

        let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
        let result = db.update_one("nonexistent", &json!({}), &json!({"$set": {"x": 1}}));
        assert!(matches!(result, Err(IronBaseError::CollectionNotFound(_))));
    }

    #[test]
    fn test_delete_on_nonexistent_collection_fails() {
        use crate::storage::MemoryStorage;

        let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
        let result = db.delete_one("nonexistent", &json!({}));
        assert!(matches!(result, Err(IronBaseError::CollectionNotFound(_))));
    }

    #[test]
    fn test_operations_after_close_fail() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");
        let db = DatabaseCore::open(&db_path).unwrap();

        // Insert some data before close
        let doc = HashMap::from([("name".to_string(), json!("Alice"))]);
        db.insert_one("users", doc).unwrap();

        // Close the database
        db.close().unwrap();

        // All operations after close should fail with DatabaseClosed error
        let result = db.insert_one("users", HashMap::new());
        assert!(matches!(result, Err(IronBaseError::DatabaseClosed)));

        let result = db.update_one("users", &json!({}), &json!({"$set": {"x": 1}}));
        assert!(matches!(result, Err(IronBaseError::DatabaseClosed)));

        let result = db.delete_one("users", &json!({}));
        assert!(matches!(result, Err(IronBaseError::DatabaseClosed)));

        let result = db.collection("users");
        assert!(matches!(result, Err(IronBaseError::DatabaseClosed)));
    }

    #[test]
    fn test_operations_after_close_fail_memory() {
        use crate::storage::MemoryStorage;

        let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();

        // Insert some data
        let doc = HashMap::from([("name".to_string(), json!("Alice"))]);
        db.insert_one("users", doc).unwrap();

        // Mark as closed (MemoryStorage doesn't have close() method, so we test via the flag)
        db.is_closed
            .store(true, std::sync::atomic::Ordering::SeqCst);

        // All operations should fail
        let result = db.insert_one("users", HashMap::new());
        assert!(matches!(result, Err(IronBaseError::DatabaseClosed)));

        let result = db.update_one("users", &json!({}), &json!({"$set": {"x": 1}}));
        assert!(matches!(result, Err(IronBaseError::DatabaseClosed)));

        let result = db.delete_one("users", &json!({}));
        assert!(matches!(result, Err(IronBaseError::DatabaseClosed)));
    }
}

#[cfg(test)]
mod wal_replay_tests {
    //! End-to-end crash recovery tests for task #26 phase 4/5.
    //!
    //! Scenario shape:
    //!   1. Create DB, create non-btree index, insert batch A.
    //!   2. Explicit `checkpoint()` — index file flushed with watermark=W.
    //!   3. Insert batch B (in WAL only, not yet in index file).
    //!   4. Simulate crash via `simulate_crash_for_test` — releases the file
    //!      lock and `mem::forget`s the DB so Drop never runs
    //!      `mark_clean_shutdown`.
    //!   5. Reopen — `initialize_index_manager` takes either the WAL-replay
    //!      path (load file at W, apply ops with tx_id > W) or the
    //!      rebuild-from-catalog fallback when a guardrail trips.
    //!   6. Verify every doc from batch A ∪ B is findable.
    //!
    //! Asserts end state only — both the replay path and the rebuild fallback
    //! converge to the same correct result. The replay path is additionally
    //! covered by `recovery::index_replay_ops::tests`.
    //!
    //! Previously lived in `tests/wal_replay_recovery_test.rs` and required
    //! `__simulate_crash_for_test` to be `pub #[doc(hidden)]` in the lib.
    //! Moved inline so `simulate_crash_for_test` can be `#[cfg(test)] fn`
    //! — never reachable from production callers.

    use super::*;
    use std::collections::HashMap;
    use tempfile::TempDir;

    fn doc(tag: &str, content: &str) -> HashMap<String, serde_json::Value> {
        let mut m = HashMap::new();
        m.insert("tag".to_string(), serde_json::json!(tag));
        m.insert("content".to_string(), serde_json::json!(content));
        m
    }

    #[test]
    fn fulltext_wal_replay_recovery_preserves_all_docs() {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("fts_replay.mlite");

        {
            let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
            let coll = db.collection("docs").unwrap();
            coll.create_fulltext_index("content".to_string(), "english", None, None)
                .unwrap();

            for i in 0..10 {
                db.insert_one(
                    "docs",
                    doc(&format!("a{}", i), &format!("alpha document number {}", i)),
                )
                .unwrap();
            }

            db.checkpoint().unwrap();

            for i in 0..5 {
                db.insert_one(
                    "docs",
                    doc(&format!("b{}", i), &format!("beta document number {}", i)),
                )
                .unwrap();
            }

            db.simulate_crash_for_test();
        }

        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        let coll = db.collection("docs").unwrap();

        let alpha = coll
            .fulltext_search("content", "alpha", Some(100), None, None, None)
            .unwrap();
        assert_eq!(
            alpha.len(),
            10,
            "batch-A docs lost after crash: expected 10, got {}",
            alpha.len()
        );

        let beta = coll
            .fulltext_search("content", "beta", Some(100), None, None, None)
            .unwrap();
        assert_eq!(
            beta.len(),
            5,
            "batch-B WAL-only docs lost after crash: expected 5, got {}",
            beta.len()
        );

        assert_eq!(
            db.count_documents("docs", &serde_json::json!({})).unwrap(),
            15
        );
    }

    #[test]
    fn fuzzy_wal_replay_recovery_preserves_all_docs() {
        use crate::index::fuzzy::FuzzyAlgorithm;

        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("fz_replay.mlite");

        {
            let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
            let coll = db.collection("names").unwrap();
            coll.create_fuzzy_index("name".to_string(), FuzzyAlgorithm::JaroWinkler, 0.7)
                .unwrap();

            for n in &["Alice", "Bob", "Charles", "Diana", "Eve"] {
                let mut d = HashMap::new();
                d.insert("name".to_string(), serde_json::json!(n));
                db.insert_one("names", d).unwrap();
            }

            db.checkpoint().unwrap();

            for n in &["Frank", "Grace", "Henry"] {
                let mut d = HashMap::new();
                d.insert("name".to_string(), serde_json::json!(n));
                db.insert_one("names", d).unwrap();
            }

            db.simulate_crash_for_test();
        }

        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        let coll = db.collection("names").unwrap();

        for exact in &[
            "Alice", "Bob", "Charles", "Diana", "Eve", "Frank", "Grace", "Henry",
        ] {
            let found = coll
                .find(&serde_json::json!({ "name": { "$fuzzy": exact } }))
                .unwrap();
            assert!(
                !found.is_empty(),
                "Fuzzy search for '{}' returned 0 results post-crash",
                exact
            );
        }

        assert_eq!(
            db.count_documents("names", &serde_json::json!({})).unwrap(),
            8
        );
    }

    #[test]
    fn fulltext_survives_checkpoint_then_crash_no_post_inserts() {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("fts_cp_crash.mlite");
        {
            let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
            let coll = db.collection("docs").unwrap();
            coll.create_fulltext_index("content".to_string(), "english", None, None)
                .unwrap();
            for i in 0..5 {
                db.insert_one("docs", doc(&format!("n{}", i), "alpha here"))
                    .unwrap();
            }
            db.checkpoint().unwrap();
            db.simulate_crash_for_test();
        }
        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        let coll = db.collection("docs").unwrap();
        let hits = coll
            .fulltext_search("content", "alpha", Some(100), None, None, None)
            .unwrap();
        assert_eq!(hits.len(), 5);
    }

    #[test]
    fn fulltext_survives_checkpoint_then_clean_close() {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("fts_cp.mlite");
        {
            let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
            let coll = db.collection("docs").unwrap();
            coll.create_fulltext_index("content".to_string(), "english", None, None)
                .unwrap();
            for i in 0..5 {
                db.insert_one("docs", doc(&format!("n{}", i), "alpha here"))
                    .unwrap();
            }
            db.checkpoint().unwrap();
        }
        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        let coll = db.collection("docs").unwrap();
        let hits = coll
            .fulltext_search("content", "alpha", Some(100), None, None, None)
            .unwrap();
        assert_eq!(hits.len(), 5);
    }

    #[test]
    fn fulltext_survives_normal_reopen_no_checkpoint() {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("fts_plain.mlite");
        {
            let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
            let coll = db.collection("docs").unwrap();
            coll.create_fulltext_index("content".to_string(), "english", None, None)
                .unwrap();
            for i in 0..5 {
                db.insert_one("docs", doc(&format!("n{}", i), "word alpha here"))
                    .unwrap();
            }
        }
        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        let coll = db.collection("docs").unwrap();
        let post = coll
            .fulltext_search("content", "alpha", Some(100), None, None, None)
            .unwrap();
        assert_eq!(post.len(), 5);
    }

    #[test]
    fn clean_shutdown_does_not_trigger_replay_path() {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("fts_clean.mlite");

        {
            let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
            let coll = db.collection("docs").unwrap();
            coll.create_fulltext_index("content".to_string(), "english", None, None)
                .unwrap();

            for i in 0..5 {
                db.insert_one(
                    "docs",
                    doc(&format!("c{}", i), &format!("clean shutdown doc {}", i)),
                )
                .unwrap();
            }
        }

        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        let coll = db.collection("docs").unwrap();
        let hits = coll
            .fulltext_search("content", "clean", Some(100), None, None, None)
            .unwrap();
        assert_eq!(hits.len(), 5);
    }

    // ========================================================================
    // Perf benches for task #26 Phase 4b WAL replay.
    //
    // Run:
    //   cargo test --release -p ironbase-core --lib \
    //     wal_replay_tests::bench -- --ignored --nocapture --test-threads=1
    //
    // Ignored by default because they write tens of thousands of docs and
    // measure I/O-bound wall clock.
    // ========================================================================

    #[test]
    #[ignore]
    fn bench_replay_vs_rebuild_fulltext() {
        const BASE_DOCS: usize = 10_000;
        const POST_CHECKPOINT_DOCS: usize = 500;

        let tmp = TempDir::new().unwrap();

        // Scenario A: crash-and-replay path.
        //   checkpoint writes a V4 .ftidx with watermark, so the reopen
        //   takes the load+replay branch and only the 500 post-checkpoint
        //   docs are re-applied from the WAL.
        let replay_path = tmp.path().join("replay.mlite");
        {
            let db = DatabaseCore::<StorageEngine>::open(&replay_path).unwrap();
            let coll = db.collection("docs").unwrap();
            coll.create_fulltext_index("content".to_string(), "english", None, None)
                .unwrap();
            for i in 0..BASE_DOCS {
                db.insert_one(
                    "docs",
                    doc(&format!("a{}", i), &format!("alpha document number {}", i)),
                )
                .unwrap();
            }
            db.checkpoint().unwrap();
            for i in 0..POST_CHECKPOINT_DOCS {
                db.insert_one(
                    "docs",
                    doc(&format!("b{}", i), &format!("beta document number {}", i)),
                )
                .unwrap();
            }
            db.simulate_crash_for_test();
        }

        let t0 = std::time::Instant::now();
        let db = DatabaseCore::<StorageEngine>::open(&replay_path).unwrap();
        let coll = db.collection("docs").unwrap();
        // Touching the collection drives initialize_index_manager; the
        // fulltext search then proves the index is usable afterwards.
        let hits_alpha = coll
            .fulltext_search("content", "alpha", Some(10), None, None, None)
            .unwrap();
        let hits_beta = coll
            .fulltext_search("content", "beta", Some(10), None, None, None)
            .unwrap();
        let replay_elapsed = t0.elapsed();
        assert!(
            !hits_alpha.is_empty() && !hits_beta.is_empty(),
            "replay path produced unusable index"
        );
        drop(db);

        // Scenario B: no checkpoint → no .ftidx on disk → dirty shutdown
        //   reload has no file to load → all_non_btree_loaded=false (because
        //   the load returns None), the safety rail forces rebuild from
        //   catalog. This mirrors the pre-Phase-4b behaviour for every
        //   dirty-shutdown reopen.
        let rebuild_path = tmp.path().join("rebuild.mlite");
        {
            let db = DatabaseCore::<StorageEngine>::open(&rebuild_path).unwrap();
            let coll = db.collection("docs").unwrap();
            coll.create_fulltext_index("content".to_string(), "english", None, None)
                .unwrap();
            // Same total document count so the rebuild work is comparable.
            for i in 0..(BASE_DOCS + POST_CHECKPOINT_DOCS) {
                db.insert_one(
                    "docs",
                    doc(&format!("x{}", i), &format!("alpha document number {}", i)),
                )
                .unwrap();
            }
            // NO checkpoint — the .ftidx never makes it to disk, so the
            // reopen cannot use replay no matter what.
            db.simulate_crash_for_test();
        }

        let t1 = std::time::Instant::now();
        let db = DatabaseCore::<StorageEngine>::open(&rebuild_path).unwrap();
        let coll = db.collection("docs").unwrap();
        let hits = coll
            .fulltext_search("content", "alpha", Some(10), None, None, None)
            .unwrap();
        let rebuild_elapsed = t1.elapsed();
        assert!(!hits.is_empty(), "rebuild path produced unusable index");

        let ratio = rebuild_elapsed.as_secs_f64() / replay_elapsed.as_secs_f64();

        eprintln!();
        eprintln!("=== Task #26 Phase 4b bench: fulltext WAL replay vs rebuild-from-catalog ===");
        eprintln!("   base docs:              {} (checkpointed)", BASE_DOCS);
        eprintln!(
            "   post-checkpoint docs:   {} (WAL-only)",
            POST_CHECKPOINT_DOCS
        );
        eprintln!(
            "   replay path reopen:     {:>8.2?}  (load .ftidx @ watermark + replay {} ops)",
            replay_elapsed, POST_CHECKPOINT_DOCS
        );
        eprintln!(
            "   rebuild path reopen:    {:>8.2?}  (full rebuild from {} catalog entries)",
            rebuild_elapsed,
            BASE_DOCS + POST_CHECKPOINT_DOCS
        );
        eprintln!("   speedup:                {:>8.2}x", ratio);
        eprintln!();
    }

    // ========================================================================
    // Production-readiness E2E
    //
    // This is the single most-important test for the week's changes
    // (atomic flushes + watermark tracking + WAL replay recovery).
    // It exercises every non-trivial code path the production MCP server
    // uses on startup / shutdown / crash, against all four index types at
    // once, and asserts that every search still returns the expected data
    // after each phase.
    //
    // Run:
    //   cargo test --release -p ironbase-core --lib \
    //     wal_replay_tests::production_readiness_e2e \
    //     -- --ignored --nocapture --test-threads=1
    // ========================================================================

    #[test]
    #[ignore]
    fn production_readiness_e2e() {
        use crate::index::fuzzy::FuzzyAlgorithm;
        use crate::vector::{DistanceMetric, VectorIndexConfig};
        use serde_json::json;

        const BASE_DOCS: usize = 300;
        const POST_CHECKPOINT_DOCS: usize = 50;
        const POST_CRASH_DOCS: usize = 25;
        const TOTAL: usize = BASE_DOCS + POST_CHECKPOINT_DOCS + POST_CRASH_DOCS;

        // Synthetic 64-dim vector so the HNSW index has real data to exercise
        // the replay + orphan-compaction paths without needing an embedding
        // provider. Values are deterministic per-id so the same doc always
        // hashes to the same vector.
        fn synth_vec(seed: usize) -> Vec<f32> {
            (0..64)
                .map(|i| ((seed.wrapping_mul(31).wrapping_add(i)) as f32).sin())
                .collect()
        }

        fn make_doc(
            i: usize,
            content: &str,
            tag: &str,
        ) -> std::collections::HashMap<String, serde_json::Value> {
            let mut m = std::collections::HashMap::new();
            m.insert("tag".to_string(), json!(tag));
            m.insert("content".to_string(), json!(content));
            m.insert("name".to_string(), json!(format!("person_{}", i)));
            m.insert("embedding".to_string(), json!(synth_vec(i)));
            m
        }

        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("e2e.mlite");

        // -------------------- Phase A: cold create + seed --------------------
        let t_seed = std::time::Instant::now();
        {
            let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
            let coll = db.collection("docs").unwrap();

            // Btree index on "tag", fuzzy on "name", fulltext on "content",
            // HNSW on "embedding" — all four non-trivial index types.
            coll.create_index("tag".to_string(), false, false).unwrap();
            coll.create_fuzzy_index("name".to_string(), FuzzyAlgorithm::JaroWinkler, 0.7)
                .unwrap();
            coll.create_fulltext_index("content".to_string(), "english", None, None)
                .unwrap();
            coll.create_vector_index(
                "embedding",
                VectorIndexConfig {
                    dim: 64,
                    metric: DistanceMetric::Cosine,
                    ..VectorIndexConfig::default()
                },
            )
            .unwrap();

            for i in 0..BASE_DOCS {
                db.insert_one(
                    "docs",
                    make_doc(i, &format!("alpha document number {}", i), "tag-A"),
                )
                .unwrap();
            }
            db.close().unwrap();
        }
        let seed_elapsed = t_seed.elapsed();

        // -------------------- Phase B: reopen + search sanity --------------------
        let t_reopen1 = std::time::Instant::now();
        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        let reopen1_elapsed = t_reopen1.elapsed();
        let coll = db.collection("docs").unwrap();

        let fts_hits = coll
            .fulltext_search("content", "alpha", Some(500), None, None, None)
            .unwrap();
        assert_eq!(fts_hits.len(), BASE_DOCS, "fulltext lost seeded docs");

        let fuzzy_hits = coll
            .find(&json!({ "name": { "$fuzzy": "person_1" } }))
            .unwrap();
        assert!(
            !fuzzy_hits.is_empty(),
            "fuzzy returned nothing for seeded name"
        );

        let tag_hits = coll.find(&json!({ "tag": "tag-A" })).unwrap();
        assert_eq!(tag_hits.len(), BASE_DOCS, "btree lost tag-A docs");

        // -------------------- Phase C: more inserts + checkpoint --------------------
        for i in 0..POST_CHECKPOINT_DOCS {
            let id = BASE_DOCS + i;
            db.insert_one(
                "docs",
                make_doc(id, &format!("beta document number {}", id), "tag-B"),
            )
            .unwrap();
        }
        let watermark_pre = db.watermark_tx_id();
        db.checkpoint().unwrap();
        assert!(watermark_pre > 0, "watermark did not advance past 0");

        // Verify all 4 search types AFTER checkpoint (index state consistent).
        assert_eq!(
            coll.fulltext_search("content", "beta", Some(500), None, None, None)
                .unwrap()
                .len(),
            POST_CHECKPOINT_DOCS,
            "fulltext missed post-checkpoint beta docs"
        );
        assert_eq!(
            coll.find(&json!({ "tag": "tag-B" })).unwrap().len(),
            POST_CHECKPOINT_DOCS,
            "btree missed tag-B docs"
        );

        // -------------------- Phase D: crash simulation --------------------
        // Keep the DB open — do a few more inserts that will live in the WAL
        // only (no post-checkpoint flush), then pull the plug.
        for i in 0..POST_CRASH_DOCS {
            let id = BASE_DOCS + POST_CHECKPOINT_DOCS + i;
            db.insert_one(
                "docs",
                make_doc(id, &format!("gamma document number {}", id), "tag-C"),
            )
            .unwrap();
        }
        db.simulate_crash_for_test();

        // -------------------- Phase E: reopen after crash, full verify --------------------
        let t_recover = std::time::Instant::now();
        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        let coll = db.collection("docs").unwrap();
        let recover_elapsed = t_recover.elapsed();

        // Every committed doc must survive the crash (replay or rebuild, either is fine).
        assert_eq!(
            db.count_documents("docs", &json!({})).unwrap() as usize,
            TOTAL,
            "catalog doc count mismatch after crash recovery"
        );
        assert_eq!(
            coll.fulltext_search("content", "alpha", Some(1000), None, None, None)
                .unwrap()
                .len(),
            BASE_DOCS,
            "fulltext lost batch A after crash"
        );
        assert_eq!(
            coll.fulltext_search("content", "beta", Some(500), None, None, None)
                .unwrap()
                .len(),
            POST_CHECKPOINT_DOCS,
            "fulltext lost batch B (post-checkpoint) after crash"
        );
        assert_eq!(
            coll.fulltext_search("content", "gamma", Some(500), None, None, None)
                .unwrap()
                .len(),
            POST_CRASH_DOCS,
            "fulltext lost batch C (WAL-only) after crash"
        );
        assert_eq!(
            coll.find(&json!({ "tag": "tag-A" })).unwrap().len(),
            BASE_DOCS,
            "btree tag-A shrunk"
        );
        assert_eq!(
            coll.find(&json!({ "tag": "tag-B" })).unwrap().len(),
            POST_CHECKPOINT_DOCS,
            "btree tag-B shrunk"
        );
        assert_eq!(
            coll.find(&json!({ "tag": "tag-C" })).unwrap().len(),
            POST_CRASH_DOCS,
            "btree tag-C shrunk"
        );
        let fuzzy_late = coll
            .find(&json!({ "name": { "$fuzzy": "person_350" } }))
            .unwrap();
        assert!(
            !fuzzy_late.is_empty(),
            "fuzzy lost post-checkpoint name after crash"
        );

        // -------------------- Phase F: post-crash writes + clean close --------------------
        db.insert_one(
            "docs",
            make_doc(TOTAL, "delta document post-recovery", "tag-D"),
        )
        .unwrap();
        db.update_many(
            "docs",
            &json!({"tag": "tag-C"}),
            &json!({"$set": {"tag": "tag-C-updated"}}),
        )
        .unwrap();
        db.delete_many("docs", &json!({"tag": "tag-C-updated"}))
            .unwrap();
        assert_eq!(
            db.count_documents("docs", &json!({})).unwrap() as usize,
            TOTAL + 1 - POST_CRASH_DOCS,
            "doc count wrong after delete_many"
        );
        // Explicit close so the file lock is released before Phase G's reopen.
        // `drop(db)` would also work on StorageEngine, but close() is more
        // readable and matches what production callers are expected to do.
        db.close().unwrap();

        // -------------------- Phase G: clean reopen (fast path) --------------------
        let t_reopen2 = std::time::Instant::now();
        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        let coll = db.collection("docs").unwrap();
        let reopen2_elapsed = t_reopen2.elapsed();

        // All deletions + updates must be visible.
        assert_eq!(
            coll.find(&json!({ "tag": "tag-C" })).unwrap().len(),
            0,
            "deleted docs reappeared"
        );
        assert_eq!(
            coll.find(&json!({ "tag": "tag-D" })).unwrap().len(),
            1,
            "post-recovery insert lost"
        );
        assert_eq!(
            coll.fulltext_search("content", "delta", Some(10), None, None, None)
                .unwrap()
                .len(),
            1,
            "fulltext lost delta doc"
        );

        // -------------------- Report --------------------
        eprintln!();
        eprintln!("=== Week's-changes E2E: production readiness ===");
        eprintln!(
            "   seed {} docs + close:            {:>8.2?}",
            BASE_DOCS, seed_elapsed
        );
        eprintln!(
            "   first clean reopen:              {:>8.2?}",
            reopen1_elapsed
        );
        eprintln!(
            "   recovery reopen (crash path):    {:>8.2?}  ({} docs, {} WAL-only)",
            recover_elapsed, TOTAL, POST_CRASH_DOCS
        );
        eprintln!(
            "   second clean reopen (fast path): {:>8.2?}",
            reopen2_elapsed
        );
        eprintln!("   all index types + find + update + delete + fuzzy + fulltext verified ✓");
        eprintln!();
    }

    // ========================================================================
    // Fast-path EDGE CASES
    //
    // Each test exercises one boundary of the recovery decision tree
    // (clean fast / WAL replay / rebuild fallback). The goal is to
    // pin the behavior at every branch the week's changes touched.
    // ========================================================================

    fn doc_with_content(
        i: usize,
        content: &str,
    ) -> std::collections::HashMap<String, serde_json::Value> {
        let mut m = std::collections::HashMap::new();
        m.insert("_id".to_string(), serde_json::json!(format!("d{}", i)));
        m.insert("content".to_string(), serde_json::json!(content));
        m
    }

    /// Edge: empty DB. First open creates the file, immediate clean close,
    /// reopen — must not panic, must report 0 collections, 0 docs.
    #[test]
    fn fastpath_edge_empty_db() {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("empty.mlite");
        {
            let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
            db.close().unwrap();
        }
        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        // No collections, listing must succeed and be empty.
        let names = db.list_collections();
        assert!(names.is_empty(), "fresh DB should have no collections");
    }

    /// Edge: collection exists but is completely empty (no docs ever).
    /// Reopen must take fast path (was_clean=true, no rebuild) and search
    /// must return 0 results without error.
    #[test]
    fn fastpath_edge_empty_collection_with_index() {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("empty_coll.mlite");
        {
            let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
            let coll = db.collection("docs").unwrap();
            coll.create_fulltext_index("content".to_string(), "english", None, None)
                .unwrap();
            // No inserts, no checkpoint — close immediately.
            db.close().unwrap();
        }
        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        let coll = db.collection("docs").unwrap();
        let hits = coll
            .fulltext_search("content", "anything", Some(10), None, None, None)
            .unwrap();
        assert_eq!(hits.len(), 0);
    }

    /// Edge: dirty shutdown but the WAL is empty (clean shutdown didn't
    /// run, but no operations happened since the last checkpoint).
    /// `recovered_ops_for_collection` is empty → `can_try_replay=false`
    /// → fall through to fast path / rebuild based on file state.
    #[test]
    fn fastpath_edge_dirty_shutdown_empty_wal() {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("empty_wal.mlite");
        {
            let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
            let coll = db.collection("docs").unwrap();
            coll.create_fulltext_index("content".to_string(), "english", None, None)
                .unwrap();
            for i in 0..5 {
                db.insert_one("docs", doc_with_content(i, "alpha")).unwrap();
            }
            db.checkpoint().unwrap(); // .ftidx written + WAL cleared
                                      // No further writes → WAL stays empty even though we crash next.
            db.simulate_crash_for_test();
        }
        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        let coll = db.collection("docs").unwrap();
        let hits = coll
            .fulltext_search("content", "alpha", Some(20), None, None, None)
            .unwrap();
        assert_eq!(
            hits.len(),
            5,
            "post-checkpoint docs lost on empty-WAL crash"
        );
    }

    /// Edge: exactly one post-watermark op. Verifies the `tx_id > watermark`
    /// boundary fires correctly (and does NOT replay ops AT the watermark).
    #[test]
    fn fastpath_edge_one_op_above_watermark() {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("boundary.mlite");
        {
            let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
            let coll = db.collection("docs").unwrap();
            coll.create_fulltext_index("content".to_string(), "english", None, None)
                .unwrap();
            for i in 0..3 {
                db.insert_one("docs", doc_with_content(i, "alpha")).unwrap();
            }
            db.checkpoint().unwrap();
            // Single post-checkpoint op.
            db.insert_one("docs", doc_with_content(3, "beta")).unwrap();
            db.simulate_crash_for_test();
        }
        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        let coll = db.collection("docs").unwrap();
        assert_eq!(
            coll.fulltext_search("content", "alpha", Some(20), None, None, None)
                .unwrap()
                .len(),
            3
        );
        assert_eq!(
            coll.fulltext_search("content", "beta", Some(20), None, None, None)
                .unwrap()
                .len(),
            1
        );
    }

    /// Edge: replay applied twice (server reopened, no further writes,
    /// reopened again). Helpers must be idempotent under repeat — second
    /// reopen sees the now-clean state and skips replay.
    #[test]
    fn fastpath_edge_replay_then_clean_reopen_twice() {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("repeat.mlite");
        {
            let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
            let coll = db.collection("docs").unwrap();
            coll.create_fulltext_index("content".to_string(), "english", None, None)
                .unwrap();
            for i in 0..5 {
                db.insert_one("docs", doc_with_content(i, "alpha")).unwrap();
            }
            db.checkpoint().unwrap();
            for i in 5..8 {
                db.insert_one("docs", doc_with_content(i, "beta")).unwrap();
            }
            db.simulate_crash_for_test();
        }
        // First reopen: replay path (3 ops > watermark).
        {
            let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
            let coll = db.collection("docs").unwrap();
            assert_eq!(
                coll.fulltext_search("content", "alpha", Some(20), None, None, None)
                    .unwrap()
                    .len(),
                5
            );
            assert_eq!(
                coll.fulltext_search("content", "beta", Some(20), None, None, None)
                    .unwrap()
                    .len(),
                3
            );
            db.close().unwrap();
        }
        // Second reopen: clean shutdown happened, fast path; counts must hold.
        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        let coll = db.collection("docs").unwrap();
        assert_eq!(
            coll.fulltext_search("content", "beta", Some(20), None, None, None)
                .unwrap()
                .len(),
            3
        );
    }

    /// Edge: multi-collection, only one has writes since checkpoint.
    /// Each collection's recovery decision is independent; the quiet
    /// collection takes fast path even though the active one needs replay.
    #[test]
    fn fastpath_edge_multi_collection_selective_replay() {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("multi.mlite");
        {
            let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
            let active = db.collection("active").unwrap();
            active
                .create_fulltext_index("content".to_string(), "english", None, None)
                .unwrap();
            let quiet = db.collection("quiet").unwrap();
            quiet
                .create_fulltext_index("content".to_string(), "english", None, None)
                .unwrap();

            for i in 0..3 {
                db.insert_one("active", doc_with_content(i, "alpha"))
                    .unwrap();
                db.insert_one("quiet", doc_with_content(i, "alpha"))
                    .unwrap();
            }
            db.checkpoint().unwrap();

            // Only `active` gets post-checkpoint writes.
            for i in 3..5 {
                db.insert_one("active", doc_with_content(i, "beta"))
                    .unwrap();
            }
            db.simulate_crash_for_test();
        }
        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        // Active: 3 alpha + 2 beta from replay or rebuild.
        let active = db.collection("active").unwrap();
        assert_eq!(
            active
                .fulltext_search("content", "alpha", Some(20), None, None, None)
                .unwrap()
                .len(),
            3
        );
        assert_eq!(
            active
                .fulltext_search("content", "beta", Some(20), None, None, None)
                .unwrap()
                .len(),
            2
        );
        // Quiet: untouched, must still have its 3 docs.
        let quiet = db.collection("quiet").unwrap();
        assert_eq!(
            quiet
                .fulltext_search("content", "alpha", Some(20), None, None, None)
                .unwrap()
                .len(),
            3
        );
    }

    /// Edge: WAL contains ops for a collection that does not exist (e.g.
    /// metadata not yet committed). Must not panic; the unrelated
    /// collection must recover correctly. Modeled by inserting before
    /// the collection's index gets created — though in practice the
    /// catalog is always written first, this test ensures any future
    /// race or metadata-only op cannot crash recovery.
    #[test]
    fn fastpath_edge_replay_skips_indexless_collection() {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("indexless.mlite");
        {
            let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
            // Collection has NO non-btree indexes — so even with WAL ops
            // there is nothing for `try_wal_replay_for_collection` to do.
            db.collection("nofts").unwrap();
            for i in 0..3 {
                db.insert_one("nofts", doc_with_content(i, "alpha"))
                    .unwrap();
            }
            db.checkpoint().unwrap();
            for i in 3..6 {
                db.insert_one("nofts", doc_with_content(i, "beta")).unwrap();
            }
            db.simulate_crash_for_test();
        }
        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        // All 6 docs must be in the catalog regardless of replay path.
        assert_eq!(
            db.count_documents("nofts", &serde_json::json!({})).unwrap() as usize,
            6
        );
    }

    /// Edge: two consecutive crashes. First crash → replay → close → second
    /// crash → another replay over a now-fresh watermark. Verifies the
    /// watermark advances after replay-driven reopens too.
    #[test]
    fn fastpath_edge_consecutive_crashes() {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("doublecrash.mlite");

        // Round 1.
        {
            let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
            let coll = db.collection("docs").unwrap();
            coll.create_fulltext_index("content".to_string(), "english", None, None)
                .unwrap();
            for i in 0..3 {
                db.insert_one("docs", doc_with_content(i, "alpha")).unwrap();
            }
            db.checkpoint().unwrap();
            for i in 3..5 {
                db.insert_one("docs", doc_with_content(i, "beta")).unwrap();
            }
            db.simulate_crash_for_test();
        }
        // Round 2: reopen + clean close so the next crash sees a watermark
        // that already covers batches A+B.
        {
            let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
            // Verify post-replay state before doing anything else.
            assert_eq!(
                db.count_documents("docs", &serde_json::json!({})).unwrap() as usize,
                5
            );
            db.checkpoint().unwrap();
            for i in 5..7 {
                db.insert_one("docs", doc_with_content(i, "gamma")).unwrap();
            }
            db.simulate_crash_for_test();
        }
        // Round 3: final reopen, all 7 docs must be findable.
        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        let coll = db.collection("docs").unwrap();
        assert_eq!(
            db.count_documents("docs", &serde_json::json!({})).unwrap() as usize,
            7
        );
        assert_eq!(
            coll.fulltext_search("content", "alpha", Some(20), None, None, None)
                .unwrap()
                .len(),
            3
        );
        assert_eq!(
            coll.fulltext_search("content", "beta", Some(20), None, None, None)
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            coll.fulltext_search("content", "gamma", Some(20), None, None, None)
                .unwrap()
                .len(),
            2
        );
    }

    /// Edge: no checkpoint ever, then crash. The .ftidx file never reaches
    /// disk → load returns None → `all_non_btree_loaded=false` → safety
    /// rail forces rebuild. All data still recovered via rebuild path.
    #[test]
    fn fastpath_edge_no_checkpoint_then_crash() {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("nockpt.mlite");
        {
            let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
            let coll = db.collection("docs").unwrap();
            coll.create_fulltext_index("content".to_string(), "english", None, None)
                .unwrap();
            for i in 0..7 {
                db.insert_one("docs", doc_with_content(i, "alpha")).unwrap();
            }
            // NO checkpoint — index file never reaches disk.
            db.simulate_crash_for_test();
        }
        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        let coll = db.collection("docs").unwrap();
        // Rebuild fallback must restore all 7 docs.
        assert_eq!(
            coll.fulltext_search("content", "alpha", Some(20), None, None, None)
                .unwrap()
                .len(),
            7,
            "rebuild fallback failed to recover all docs"
        );
    }

    /// Edge: legacy V3 file simulation. We can't easily produce a
    /// `last_flushed_tx_id == 0 && doc_count > 0` file from the live code,
    /// but we can confirm the safety rail by deleting the .ftidx mid-cycle
    /// — which forces `all_non_btree_loaded=false` and then rebuild.
    /// (Equivalent code path in the safety rail.)
    #[test]
    fn fastpath_edge_missing_ftidx_forces_rebuild() {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("legacy.mlite");
        let db_dir = tmp.path();
        {
            let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
            let coll = db.collection("docs").unwrap();
            coll.create_fulltext_index("content".to_string(), "english", None, None)
                .unwrap();
            for i in 0..4 {
                db.insert_one("docs", doc_with_content(i, "alpha")).unwrap();
            }
            db.checkpoint().unwrap();
            for i in 4..6 {
                db.insert_one("docs", doc_with_content(i, "beta")).unwrap();
            }
            db.simulate_crash_for_test();
        }

        // Simulate index file loss: delete every .ftidx in the directory
        // before reopen. The safety rail must catch the missing-file case
        // and rebuild from catalog (without panicking).
        for entry in std::fs::read_dir(db_dir).unwrap().flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("ftidx") {
                std::fs::remove_file(&path).unwrap();
            }
        }

        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        let coll = db.collection("docs").unwrap();
        // Rebuild from catalog must restore all 6 docs.
        assert_eq!(
            coll.fulltext_search("content", "alpha", Some(20), None, None, None)
                .unwrap()
                .len(),
            4,
            "rebuild must reproduce alpha docs after .ftidx loss"
        );
        assert_eq!(
            coll.fulltext_search("content", "beta", Some(20), None, None, None)
                .unwrap()
                .len(),
            2,
            "rebuild must reproduce beta docs after .ftidx loss"
        );
    }

    // ========================================================================
    // Completed-task EDGE CASES
    //
    // One test per completed TODO entry from this week (excluding #26 which
    // is exhaustively covered by the rest of this module). Each test pins
    // the specific behavior the fix landed.
    // ========================================================================

    /// Task #11: orphaned `.fzidx.tmp` left by an interrupted fuzzy flush
    /// must be removed by `StorageEngine::open` so the next flush starts
    /// from a clean slate. Test creates a stray `.fzidx.tmp` before opening
    /// the DB and verifies it is gone afterwards.
    #[test]
    fn task11_fzidx_tmp_orphan_cleanup() {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("orphan.mlite");

        // Create an empty DB so the directory exists.
        DatabaseCore::<StorageEngine>::open(&db_path)
            .unwrap()
            .close()
            .unwrap();

        // Plant an orphan `.fzidx.tmp` (mimics a fuzzy flush that crashed
        // mid-rename). Same dir as the .mlite, arbitrary content.
        let orphan = tmp.path().join("docs_collection_field_fuzzy.fzidx.tmp");
        std::fs::write(&orphan, b"crash-leftover").unwrap();
        assert!(orphan.exists());

        // Reopen — the storage engine must clean it up.
        let _db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        assert!(!orphan.exists(), ".fzidx.tmp should be removed on open");
    }

    /// Task #18 + storage cleanup: orphaned `.hnsw.tmp` from an interrupted
    /// HNSW atomic flush is removed by `StorageEngine::open`.
    #[test]
    fn task18_hnsw_tmp_orphan_cleanup() {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("orphan_hnsw.mlite");
        DatabaseCore::<StorageEngine>::open(&db_path)
            .unwrap()
            .close()
            .unwrap();

        let orphan = tmp.path().join("foo_vec_embedding.hnsw.tmp");
        std::fs::write(&orphan, b"crash-leftover").unwrap();
        assert!(orphan.exists());

        let _db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        assert!(!orphan.exists(), ".hnsw.tmp should be removed on open");
    }

    /// Task #18: HNSW flush should leave NO `.hnsw.tmp` behind (atomic
    /// rename completed). Insert vectors, checkpoint, scan dir for stragglers.
    #[test]
    fn task18_hnsw_flush_leaves_no_tmp() {
        use crate::vector::{DistanceMetric, VectorIndexConfig};

        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("hnsw_atomic.mlite");
        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        let coll = db.collection("docs").unwrap();
        coll.create_vector_index(
            "embedding",
            VectorIndexConfig {
                dim: 8,
                metric: DistanceMetric::Cosine,
                ..VectorIndexConfig::default()
            },
        )
        .unwrap();

        for i in 0..10 {
            let mut d = std::collections::HashMap::new();
            let vec: Vec<f32> = (0..8).map(|j| ((i * 8 + j) as f32).sin()).collect();
            d.insert("embedding".to_string(), serde_json::json!(vec));
            db.insert_one("docs", d).unwrap();
        }
        db.checkpoint().unwrap();

        let leftover = std::fs::read_dir(tmp.path())
            .unwrap()
            .flatten()
            .filter(|e| {
                e.path()
                    .extension()
                    .and_then(|s| s.to_str())
                    .map(|s| s == "tmp")
                    .unwrap_or(false)
            })
            .count();
        assert_eq!(
            leftover, 0,
            ".hnsw.tmp should not survive a successful flush"
        );
    }

    /// Task #16 / #17: fulltext flush leaves NO `.ftidx.tmp` (atomic
    /// commit completed). Same shape as the HNSW test above for fulltext.
    #[test]
    fn task16_17_fulltext_flush_leaves_no_tmp() {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("ft_atomic.mlite");
        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        let coll = db.collection("docs").unwrap();
        coll.create_fulltext_index("content".to_string(), "english", None, None)
            .unwrap();
        for i in 0..20 {
            let mut d = std::collections::HashMap::new();
            d.insert(
                "content".to_string(),
                serde_json::json!(format!("alpha document {}", i)),
            );
            db.insert_one("docs", d).unwrap();
        }
        db.checkpoint().unwrap();

        let leftover = std::fs::read_dir(tmp.path())
            .unwrap()
            .flatten()
            .filter(|e| {
                e.path()
                    .extension()
                    .and_then(|s| s.to_str())
                    .map(|s| s == "tmp")
                    .unwrap_or(false)
            })
            .count();
        assert_eq!(leftover, 0, "no .tmp files should survive flush");
    }

    /// Task #8 + #10: `commit_flush` is retry-safe and prunes
    /// `deleted_doc_ids`. Concretely: insert → delete → checkpoint →
    /// reopen — the deleted doc must not appear in search results, and
    /// the on-disk state must reflect the deletion (no ghost via stale
    /// `deleted_doc_ids` entry).
    #[test]
    fn task8_10_commit_flush_drain_and_prune() {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("commit_flush.mlite");
        {
            let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
            let coll = db.collection("docs").unwrap();
            coll.create_fulltext_index("content".to_string(), "english", None, None)
                .unwrap();
            for i in 0..10 {
                let mut d = std::collections::HashMap::new();
                d.insert("_id".to_string(), serde_json::json!(format!("d{}", i)));
                d.insert(
                    "content".to_string(),
                    serde_json::json!(format!("alpha document {}", i)),
                );
                db.insert_one("docs", d).unwrap();
            }
            db.checkpoint().unwrap();

            // Now delete half — the in-memory index records this in
            // `deleted_doc_ids` (lazy mode); commit_flush must prune them.
            for i in 0..5 {
                db.delete_one("docs", &serde_json::json!({"_id": format!("d{}", i)}))
                    .unwrap();
            }
            db.checkpoint().unwrap();
            db.close().unwrap();
        }
        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        let coll = db.collection("docs").unwrap();
        let hits = coll
            .fulltext_search("content", "alpha", Some(50), None, None, None)
            .unwrap();
        assert_eq!(
            hits.len(),
            5,
            "fulltext should reflect deletion after commit_flush"
        );
        // Spot-check the survivors — every hit must be d5..d9.
        for (doc, _, _) in &hits {
            let id = doc.get("_id").and_then(|v| v.as_str()).unwrap_or("");
            assert!(
                id.starts_with("d")
                    && id != "d0"
                    && id != "d1"
                    && id != "d2"
                    && id != "d3"
                    && id != "d4",
                "deleted doc {} reappeared after flush",
                id
            );
        }
    }

    /// Task #12: `batch_update_indexes` no longer issues a redundant
    /// remove for the fulltext index (refactor). Behavioral check:
    /// update changes the indexed field's value once → old text drops
    /// out of search results, new text shows up — no double counting,
    /// no leftover ghost doc with the old text.
    #[test]
    fn task12_update_keeps_index_consistent() {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("update.mlite");
        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        let coll = db.collection("docs").unwrap();
        coll.create_fulltext_index("content".to_string(), "english", None, None)
            .unwrap();
        let mut d = std::collections::HashMap::new();
        d.insert("_id".to_string(), serde_json::json!("d1"));
        d.insert("content".to_string(), serde_json::json!("alpha original"));
        db.insert_one("docs", d).unwrap();

        // Confirm baseline.
        assert_eq!(
            coll.fulltext_search("content", "alpha", Some(10), None, None, None)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            coll.fulltext_search("content", "renamed", Some(10), None, None, None)
                .unwrap()
                .len(),
            0
        );

        // Update the indexed field's content.
        db.update_one(
            "docs",
            &serde_json::json!({"_id": "d1"}),
            &serde_json::json!({"$set": {"content": "renamed entry"}}),
        )
        .unwrap();

        // Old token must be gone (single remove worked); new token must
        // be present (single insert worked).
        assert_eq!(
            coll.fulltext_search("content", "alpha", Some(10), None, None, None)
                .unwrap()
                .len(),
            0,
            "old fulltext token survived update — possible redundant or missing remove"
        );
        assert_eq!(
            coll.fulltext_search("content", "renamed", Some(10), None, None, None)
                .unwrap()
                .len(),
            1
        );
    }

    /// Task #14 (architectural): WAL did not protect non-btree indexes.
    /// Resolution = task #26 watermark + replay infrastructure. This test
    /// stays here as a regression net even though the heavy-lifting tests
    /// live in the wal_replay_tests/edge sections — it verifies the
    /// integrated guarantee in a single line: a fulltext index loses
    /// nothing across a dirty shutdown that was missing this protection
    /// before this week's work landed.
    #[test]
    fn task14_fulltext_survives_dirty_shutdown_with_post_checkpoint_writes() {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("task14.mlite");
        {
            let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
            let coll = db.collection("docs").unwrap();
            coll.create_fulltext_index("content".to_string(), "english", None, None)
                .unwrap();
            for i in 0..6 {
                let mut d = std::collections::HashMap::new();
                d.insert(
                    "content".to_string(),
                    serde_json::json!(format!("alpha doc {}", i)),
                );
                db.insert_one("docs", d).unwrap();
            }
            db.checkpoint().unwrap();
            for i in 6..9 {
                let mut d = std::collections::HashMap::new();
                d.insert(
                    "content".to_string(),
                    serde_json::json!(format!("beta doc {}", i)),
                );
                db.insert_one("docs", d).unwrap();
            }
            db.simulate_crash_for_test();
        }
        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        let coll = db.collection("docs").unwrap();
        // Before this week: beta docs would be lost (WAL didn't protect fulltext).
        // After: every doc is recoverable via replay or rebuild fallback.
        assert_eq!(
            coll.fulltext_search("content", "alpha", Some(20), None, None, None)
                .unwrap()
                .len(),
            6
        );
        assert_eq!(
            coll.fulltext_search("content", "beta", Some(20), None, None, None)
                .unwrap()
                .len(),
            3
        );
    }

    /// Audit #15 finding B — orphaned `.ftidx.tmp` cleanup.
    ///
    /// Mirrors `task11_fzidx_tmp_orphan_cleanup` and `task18_hnsw_tmp_orphan_cleanup`
    /// for the new fulltext atomic flush. Pre-plant a stray `.ftidx.tmp`
    /// in the DB directory, open the DB, assert it is removed.
    #[test]
    fn audit15_finding_b_ftidx_tmp_orphan_cleanup() {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("orphan_ft.mlite");
        DatabaseCore::<StorageEngine>::open(&db_path)
            .unwrap()
            .close()
            .unwrap();

        let orphan = tmp.path().join("foo_content_fts.ftidx.tmp");
        std::fs::write(&orphan, b"crash-leftover").unwrap();
        assert!(orphan.exists());

        let _db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        assert!(!orphan.exists(), ".ftidx.tmp should be removed on open");
    }

    /// Audit #15 finding B — fulltext atomic flush leaves no `.ftidx.tmp` after
    /// success (same shape as `task16_17_fulltext_flush_leaves_no_tmp`, but
    /// runs through the FULL two-phase batch flush so the rename path is
    /// exercised end to end).
    #[test]
    fn audit15_finding_b_serialize_flush_leaves_no_tmp() {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("ft_atomic_full.mlite");
        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        let coll = db.collection("docs").unwrap();
        coll.create_fulltext_index("content".to_string(), "english", None, None)
            .unwrap();

        // First insert + checkpoint to seed disk state, then second batch
        // + checkpoint to exercise the merge-from-disk path inside
        // serialize_flush (lazy_mode=true, snapshot.token_offsets points into
        // the OLD .ftidx file while we write a fresh temp).
        for i in 0..10 {
            let mut d = std::collections::HashMap::new();
            d.insert(
                "content".to_string(),
                serde_json::json!(format!("alpha document {}", i)),
            );
            db.insert_one("docs", d).unwrap();
        }
        db.checkpoint().unwrap();

        for i in 10..20 {
            let mut d = std::collections::HashMap::new();
            d.insert(
                "content".to_string(),
                serde_json::json!(format!("beta document {}", i)),
            );
            db.insert_one("docs", d).unwrap();
        }
        db.checkpoint().unwrap();

        // No .ftidx.tmp must survive a successful flush.
        let leftover: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .flatten()
            .filter(|e| {
                e.path()
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.ends_with(".ftidx.tmp"))
                    .unwrap_or(false)
            })
            .collect();
        assert!(
            leftover.is_empty(),
            "unexpected .ftidx.tmp survived flush: {:?}",
            leftover.iter().map(|e| e.path()).collect::<Vec<_>>()
        );

        // Sanity: after the full atomic two-phase flush the index is still
        // searchable (alpha and beta both findable). Validates that the
        // rename produced a coherent file with both batches' content.
        assert_eq!(
            coll.fulltext_search("content", "alpha", Some(50), None, None, None)
                .unwrap()
                .len(),
            10
        );
        assert_eq!(
            coll.fulltext_search("content", "beta", Some(50), None, None, None)
                .unwrap()
                .len(),
            10
        );
    }

    /// Audit #15 finding A — full DB integration test for the
    /// Phase-2-delete-only sibling bug.
    ///
    /// Reproduces via the public API: insert + checkpoint, then
    /// delete + checkpoint, then close, then reopen. Without the
    /// `has_pending_entries()` extension to check `deleted_doc_ids`,
    /// the second checkpoint would clear the dirty flag too eagerly
    /// (Phase 2 of the second flush was a no-op for `inverted_index`
    /// but added the deleted doc to `deleted_doc_ids`), the close
    /// would skip the fulltext flush, and the reopen would resurrect
    /// the deleted doc.
    ///
    /// The single-thread sequencing here doesn't actually race two
    /// phases together, but the same `commit_flush` path runs and
    /// the same `has_pending_entries` decision fires. Pinning here
    /// guards the public-API surface against regression.
    #[test]
    fn audit15_phase2_delete_survives_close_reopen() {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("audit15.mlite");
        {
            let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
            let coll = db.collection("docs").unwrap();
            coll.create_fulltext_index("content".to_string(), "english", None, None)
                .unwrap();

            // Three docs with distinct content tokens.
            for (i, word) in ["alpha", "beta", "gamma"].iter().enumerate() {
                let mut d = std::collections::HashMap::new();
                d.insert("_id".to_string(), serde_json::json!(format!("d{}", i)));
                d.insert("content".to_string(), serde_json::json!(word.to_string()));
                db.insert_one("docs", d).unwrap();
            }
            db.checkpoint().unwrap();
            assert_eq!(
                coll.fulltext_search("content", "alpha", Some(10), None, None, None)
                    .unwrap()
                    .len(),
                1
            );

            // Delete d0 (alpha) — only delete, no insert. The
            // pre-fix code path: dirty cleared after this checkpoint.
            db.delete_one("docs", &serde_json::json!({"_id": "d0"}))
                .unwrap();
            db.checkpoint().unwrap();

            // Live search must already see the delete.
            assert_eq!(
                coll.fulltext_search("content", "alpha", Some(10), None, None, None)
                    .unwrap()
                    .len(),
                0
            );
            db.close().unwrap();
        }
        // Reopen — the deleted doc must NOT reappear in fulltext search.
        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        let coll = db.collection("docs").unwrap();
        assert_eq!(
            coll.fulltext_search("content", "alpha", Some(10), None, None, None)
                .unwrap()
                .len(),
            0,
            "deleted doc reappeared after close+reopen — Phase-2 delete-only \
             sequence cleared the dirty flag prematurely"
        );
        // Sanity check: the other two docs survive.
        assert_eq!(
            coll.fulltext_search("content", "beta", Some(10), None, None, None)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            coll.fulltext_search("content", "gamma", Some(10), None, None, None)
                .unwrap()
                .len(),
            1
        );
    }
}
