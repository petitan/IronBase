//! # Durability Modul - Auto-Commit CRUD és Tartósság Kezelés
//!
//! ## Cél
//!
//! Ez a modul felelős az írási műveletek tartósságának (durability) biztosításáért.
//! Implementálja a 3 durability módot és a WAL-first commit stratégiát.
//!
//! ## Durability Módok
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────────────────────┐
//! │                     DURABILITY MODE ÖSSZEHASONLÍTÁS                      │
//! ├──────────────────────────────────────────────────────────────────────────┤
//! │                                                                          │
//! │   SAFE MODE (default)                                                    │
//! │   ┌────────────────────────────────────────────────────────────────┐    │
//! │   │  insert_one()                                                  │    │
//! │   │      │                                                         │    │
//! │   │      ▼                                                         │    │
//! │   │  prepare() → WAL write → WAL fsync → persist() → OK           │    │
//! │   │              ▲                                                 │    │
//! │   │              │                                                 │    │
//! │   │         ATOMIC POINT                                           │    │
//! │   │         (crash után recovery újrajátssza)                      │    │
//! │   │                                                                │    │
//! │   │  Sebesség: ~1,000-5,000 ops/sec                                │    │
//! │   │  Adatvesztés crash-nél: 0 (garantált)                          │    │
//! │   └────────────────────────────────────────────────────────────────┘    │
//! │                                                                          │
//! │   BATCH MODE                                                             │
//! │   ┌────────────────────────────────────────────────────────────────┐    │
//! │   │  insert_one() → buffer ops (N darab)                           │    │
//! │   │      │                                                         │    │
//! │   │      ▼ (buffer full VAGY memory limit)                         │    │
//! │   │  flush_batch():                                                │    │
//! │   │      │                                                         │    │
//! │   │      ├─▶ WAL write ALL ops                                     │    │
//! │   │      ├─▶ WAL fsync ← ATOMIC POINT                              │    │
//! │   │      ├─▶ persist ALL docs                                      │    │
//! │   │      └─▶ sync storage                                          │    │
//! │   │                                                                │    │
//! │   │  Sebesség: ~20,000-50,000 ops/sec                              │    │
//! │   │  Adatvesztés crash-nél: max batch_size ops                     │    │
//! │   └────────────────────────────────────────────────────────────────┘    │
//! │                                                                          │
//! │   UNSAFE MODE                                                            │
//! │   ┌────────────────────────────────────────────────────────────────┐    │
//! │   │  insert_one() → storage write (NO WAL!)                        │    │
//! │   │      │                                                         │    │
//! │   │      ▼ (opcionális auto_checkpoint)                            │    │
//! │   │  checkpoint() → metadata flush                                 │    │
//! │   │                                                                │    │
//! │   │  Sebesség: ~50,000-100,000 ops/sec                             │    │
//! │   │  Adatvesztés crash-nél: minden uncommitted (!)                 │    │
//! │   └────────────────────────────────────────────────────────────────┘    │
//! │                                                                          │
//! └──────────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## WAL-First Commit Stratégia (Safe Mode)
//!
//! ```text
//! ┌───────────────────────────────────────────────────────────────┐
//! │              SAFE MODE: 4-PHASE COMMIT PROTOCOL               │
//! ├───────────────────────────────────────────────────────────────┤
//! │                                                               │
//! │   Phase 1: PREPARE                                            │
//! │   ┌─────────────────────────────────────────────────────┐    │
//! │   │  - Validáció (schema, unique constraint)            │    │
//! │   │  - _id generálás                                    │    │
//! │   │  - WAL doc előkészítés                              │    │
//! │   │  - NEM ír storage-ba!                               │    │
//! │   └─────────────────────────────────────────────────────┘    │
//! │                         │                                     │
//! │                         ▼                                     │
//! │   Phase 2: WAL COMMIT                                         │
//! │   ┌─────────────────────────────────────────────────────┐    │
//! │   │  - BEGIN entry → WAL                                │    │
//! │   │  - OPERATION entry → WAL                            │    │
//! │   │  - COMMIT entry → WAL                               │    │
//! │   │  - fsync() ← ATOMIC POINT (crash-safe innen)        │    │
//! │   └─────────────────────────────────────────────────────┘    │
//! │                         │                                     │
//! │                         ▼                                     │
//! │   Phase 3: PERSIST                                            │
//! │   ┌─────────────────────────────────────────────────────┐    │
//! │   │  - Storage write (append doc)                       │    │
//! │   │  - Index update                                     │    │
//! │   │  - Cache invalidation                               │    │
//! │   └─────────────────────────────────────────────────────┘    │
//! │                         │                                     │
//! │                         ▼                                     │
//! │   Phase 4: CLEANUP                                            │
//! │   ┌─────────────────────────────────────────────────────┐    │
//! │   │  - Metadata flush                                   │    │
//! │   │  - WAL clear                                        │    │
//! │   └─────────────────────────────────────────────────────┘    │
//! │                                                               │
//! └───────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Batch Mode WAL Ordering (Bug Fix)
//!
//! ```text
//! HIBÁS IMPLEMENTÁCIÓ (korábban):
//! ┌───────────────────────────────────────────┐
//! │  1. insert_one_persist() → storage write  │
//! │  2. add_to_batch() → buffer op            │
//! │  3. flush_batch() → WAL write + fsync     │
//! │                                           │
//! │  CRASH WINDOW: 1 és 3 között!             │
//! │  → Doc storage-ban, de NINCS WAL-ban      │
//! │  → Recovery után doc "eltűnik"            │
//! └───────────────────────────────────────────┘
//!
//! HELYES IMPLEMENTÁCIÓ (mostani):
//! ┌───────────────────────────────────────────┐
//! │  1. insert_one_prepare() → validate, ID   │
//! │  2. buffer prepared doc                   │
//! │  3. add_to_batch() → buffer WAL op        │
//! │  4. flush_batch():                        │
//! │     a) WAL write ALL                      │
//! │     b) WAL fsync ← ATOMIC                 │
//! │     c) persist ALL docs                   │
//! │     d) sync storage                       │
//! │                                           │
//! │  NINCS CRASH WINDOW                       │
//! │  → WAL MINDIG előbb, storage utána        │
//! └───────────────────────────────────────────┘
//! ```
//!
//! ## Persist Failure Kezelés (ABORT Entry)
//!
//! Ha a WAL commit után a persist phase sikertelen:
//!
//! ```text
//! 1. WAL commit OK
//! 2. persist() → ERROR
//! 3. abort_committed_transaction(tx_id) → ABORT entry WAL-ba
//! 4. Recovery: látja COMMIT-ot, DE utána ABORT-ot → skip TX
//! ```
//!
//! ## Invariánsok
//!
//! 1. **WAL-First**: Storage write SOHA nem előzi meg a WAL commit-ot (Safe mode)
//! 2. **Atomic Point**: WAL fsync után az adat crash-safe
//! 3. **Read Committed**: Aktív write TX blokkolja az új write-okat
//! 4. **Hybrid Locking**: Collection lock CSAK unique index esetén kell
//! 5. **Memory Limit**: Batch mode automatikusan flush-ol ha túl sok memória
//!
//! ## Teljesítmény Trade-off-ok
//!
//! | Mód    | fsync/op | Sebesség | Crash garancia |
//! |--------|----------|----------|----------------|
//! | Safe   | 1        | Lassú    | 100%           |
//! | Batch  | 1/N      | Gyors    | batch_size ops |
//! | Unsafe | 0        | Leggyorsabb | Nincs       |
//!
//! ## Kapcsolódó Modulok
//!
//! - [`crate::wal`] - Write-Ahead Log implementáció
//! - [`crate::transaction`] - Transaction állapotgép
//! - [`crate::storage`] - Storage engine (append, flush)
//! - [`crate::durability::DurabilityMode`] - Mód enum definíció

use std::collections::HashMap;
use std::sync::atomic::Ordering;

use serde_json::Value;

use crate::collection_core::RawOperations;
use crate::document::DocumentId;
use crate::durability::DurabilityMode;
use crate::error::{IronBaseError, Result};
use crate::storage::{MemoryStorage, StorageEngine};
use crate::transaction::{Operation, Transaction};

use super::DatabaseCore;

/// Internal trait to flush any pending batch buffers before metadata sync
pub trait BatchFlush {
    fn flush_pending_batch(&self) -> Result<()>;
}

impl BatchFlush for DatabaseCore<StorageEngine> {
    fn flush_pending_batch(&self) -> Result<()> {
        if matches!(self.durability_mode, DurabilityMode::Batch { .. }) {
            self.flush_batch()?;
        }
        Ok(())
    }
}

impl BatchFlush for DatabaseCore<MemoryStorage> {
    fn flush_pending_batch(&self) -> Result<()> {
        // No-op for MemoryStorage (no persistence)
        Ok(())
    }
}

// ============================================================================
// STORAGEENGINE-SPECIFIC AUTO-COMMIT HELPERS
// ============================================================================

impl DatabaseCore<StorageEngine> {
    /// Begin an auto-transaction (internal use only for auto-commit mode)
    ///
    /// This is used internally by insert_one/update_one/delete_one when
    /// durability_mode is Safe or Batch. Not exposed to external users.
    pub(crate) fn begin_auto_transaction(&self) -> Transaction {
        let tx_id = self.next_tx_id.fetch_add(1, Ordering::SeqCst);
        Transaction::new(tx_id)
    }

    /// Commit auto-transaction with WAL and fsync
    ///
    /// This is the critical path for Safe mode:
    /// 1. Write to WAL (BEGIN + OPERATIONS + COMMIT)
    /// 2. WAL fsync
    /// 3. Metadata flush
    /// 4. WAL clear
    pub(crate) fn commit_auto_transaction(&self, mut transaction: Transaction) -> Result<()> {
        let mut storage = self.storage.write();

        // Write to WAL and commit
        storage.commit_transaction(&mut transaction)?;

        // WAL is automatically flushed in commit_transaction()
        // This ensures durability even on power failure

        Ok(())
    }

    /// Commit auto-transaction for batch mode (skip file sync)
    /// WAL is synced for durability, but file sync is deferred to batch end.
    pub(crate) fn commit_auto_transaction_batch(&self, mut transaction: Transaction) -> Result<()> {
        let mut storage = self.storage.write();
        storage.commit_transaction_batch(&mut transaction)?;
        Ok(())
    }

    /// Sync the database file to disk (for batch mode)
    pub(crate) fn sync_storage_file(&self) -> Result<()> {
        let mut storage = self.storage.write();
        storage.sync_file()?;
        Ok(())
    }

    /// Write ABORT entry for a previously committed transaction
    ///
    /// This is called when the persist phase fails after WAL commit.
    /// The ABORT entry ensures recovery will discard the committed transaction.
    ///
    /// # Arguments
    /// * `tx_id` - The transaction ID that was committed but persist failed
    pub(crate) fn abort_committed_transaction(&self, tx_id: u64) -> Result<()> {
        let mut storage = self.storage.write();
        storage.write_abort_entry(tx_id)
    }

    /// Flush batch operations to WAL with proper WAL-first ordering
    ///
    /// Used by Batch mode when batch_buffer reaches batch_size.
    /// Creates a single transaction with all buffered operations.
    ///
    /// ## WAL-FIRST ORDERING (FIX for durability bug)
    ///
    /// BEFORE (incorrect):
    /// 1. insert_one_persist() → storage write
    /// 2. add_to_batch() → buffer operation
    /// 3. flush_batch() → WAL write + fsync
    /// CRASH WINDOW: If crash between 1 and 3, doc in storage but NOT in WAL!
    ///
    /// AFTER (correct):
    /// 1. insert_one_prepare() → validate, generate ID (NO storage write)
    /// 2. buffer prepared doc in batch_doc_buffer
    /// 3. add_to_batch() → buffer WAL operation
    /// 4. flush_batch():
    ///    a) WAL write all ops
    ///    b) WAL fsync ← ATOMIC POINT
    ///    c) Persist all buffered docs to storage
    ///    d) Clear buffers
    ///    e) Sync storage file
    pub(crate) fn flush_batch(&self) -> Result<()> {
        let mut batch = self.batch_buffer.write();
        let mut doc_buffer = self.batch_doc_buffer.write();

        // Nothing to flush
        if batch.is_empty() && doc_buffer.is_empty() {
            return Ok(());
        }

        // 1. Create auto-transaction with all WAL operations
        let mut auto_tx = self.begin_auto_transaction();
        let tx_id = auto_tx.id;

        for op in batch.iter() {
            auto_tx.add_operation(op.clone())?;
        }

        // Mark as already_applied - we'll persist ourselves after WAL commit
        // This prevents commit_transaction from calling apply_operations()
        auto_tx.mark_operations_applied();

        // 2. WAL COMMIT FIRST (atomic point)
        // This writes BEGIN + OPERATIONS + COMMIT to WAL and fsyncs
        self.commit_auto_transaction_batch(auto_tx)?;

        // 3. PERSIST all buffered operations to storage
        // At this point, if we crash, recovery will replay from WAL
        if let Err(e) = self.persist_buffered_operations(&mut doc_buffer) {
            // Persist failed → write ABORT to WAL
            // This ensures recovery will skip this transaction
            let _ = self.abort_committed_transaction(tx_id);
            return Err(e);
        }

        // 4. Clear buffers
        batch.clear();
        let previous_bytes = doc_buffer.memory_bytes;
        doc_buffer.clear_and_shrink_if_large(previous_bytes);

        // 5. Sync storage file (one fsync per batch - key optimization)
        drop(batch);
        drop(doc_buffer);
        self.sync_storage_file()?;

        Ok(())
    }

    /// Persist all buffered operations to storage (WAL ORDERING FIX)
    ///
    /// Called after WAL commit in flush_batch() to actually write:
    /// - Inserts: new documents
    /// - Updates: modified documents
    /// - Deletes: tombstones
    fn persist_buffered_operations(&self, doc_buffer: &mut super::BatchDocBuffer) -> Result<()> {
        // 1. Persist inserts
        for (collection_name, prepared_docs) in doc_buffer.inserts.drain() {
            let collection = self.collection(&collection_name)?;
            for prepared in prepared_docs {
                collection.insert_one_persist(prepared)?;
            }
        }

        // 2. Persist updates (WAL ORDERING FIX)
        for (collection_name, prepared_updates) in doc_buffer.updates.drain() {
            let collection = self.collection(&collection_name)?;
            for prepared in prepared_updates {
                collection.update_one_persist_batch(prepared)?;
            }
        }

        // 3. Persist deletes (WAL ORDERING FIX)
        for (collection_name, prepared_deletes) in doc_buffer.deletes.drain() {
            let collection = self.collection(&collection_name)?;
            for prepared in prepared_deletes {
                collection.delete_one_persist_batch(prepared)?;
            }
        }

        Ok(())
    }

    /// Add operation to batch buffer (for Batch mode)
    ///
    /// Returns true if batch is full and needs flushing
    pub(crate) fn add_to_batch(&self, operation: Operation) -> Result<bool> {
        let mut batch = self.batch_buffer.write();
        batch.push(operation);

        if let Some(batch_size) = self.durability_mode.batch_size() {
            Ok(batch.len() >= batch_size)
        } else {
            Ok(false)
        }
    }

    // ========== Auto-Commit CRUD Operations (StorageEngine-specific, PUBLIC API) ==========

    /// Insert one document with auto-commit (respects durability mode)
    ///
    /// This is the SAFE insert_one that respects the database's durability mode:
    /// - **Safe mode**: Auto-commits immediately (like SQL)
    /// - **Batch mode**: Batches and commits periodically
    /// - **Unsafe mode**: No auto-commit (fast path)
    ///
    /// # Example
    /// ```rust
    /// use ironbase_core::{DatabaseCore, DurabilityMode};
    /// use ironbase_core::storage::StorageEngine;
    /// use std::collections::HashMap;
    /// use serde_json::json;
    ///
    /// let db = DatabaseCore::<StorageEngine>::open("app.mlite")?; // Safe by default
    /// let doc_id = db.insert_one("users", HashMap::from([
    ///     ("name".to_string(), json!("Alice")),
    ///     ("age".to_string(), json!(30)),
    /// ]))?;
    /// # Ok::<(), ironbase_core::IronBaseError>(())
    /// ```
    pub fn insert_one(
        &self,
        collection_name: &str,
        document: HashMap<String, Value>,
    ) -> Result<DocumentId> {
        self.check_not_closed()?;
        match self.durability_mode {
            DurabilityMode::Safe => {
                // Wait for any active write transaction to complete (blocking with timeout)
                self.wait_for_write_lock_release()?;

                // HYBRID LOCKING: Only acquire collection lock if there are unique indexes
                // Collections without unique indexes don't need the lock (no constraint races)
                let write_lock = self.get_collection_write_lock(collection_name);
                let _guard = if self.collection_has_unique_index(collection_name) {
                    Some(write_lock.lock())
                } else {
                    None
                };

                // Safe mode: Auto-commit every operation
                let collection = self.collection(collection_name)?;

                // 1. PREPARE phase: validate, generate ID, build WAL doc
                // No storage writes happen here - just preparation
                let prepared = collection.insert_one_prepare(document)?;

                // 2. Begin auto-transaction and add operation
                // prepared.wal_doc already contains _id and _collection
                let mut auto_tx = self.begin_auto_transaction();
                let tx_id = auto_tx.id; // Save tx_id before commit consumes transaction
                auto_tx.add_operation(Operation::Insert {
                    collection: collection_name.to_string(),
                    doc_id: prepared.doc_id.clone(),
                    doc: prepared.wal_doc.clone(),
                })?;

                // 3. Mark as applied (storage write follows WAL commit)
                auto_tx.mark_operations_applied();

                // 4. Auto-commit (WAL write + fsync)
                self.commit_auto_transaction(auto_tx)?;

                // 5. PERSIST phase: write to storage after WAL is committed
                // If persist fails, write ABORT to WAL to prevent recovery replaying this tx
                match collection.insert_one_persist(prepared) {
                    Ok(doc_id) => Ok(doc_id),
                    Err(e) => {
                        // Persist failed - write ABORT to invalidate the committed WAL entry
                        // This ensures recovery will skip this transaction
                        let _ = self.abort_committed_transaction(tx_id);
                        Err(e)
                    }
                }
                // _guard dropped here - lock released
            }

            DurabilityMode::Batch { .. } => {
                // Wait for active write transaction to complete (Read Committed isolation)
                self.wait_for_write_lock_release()?;

                // WAL-FIRST BATCH MODE: Guaranteed crash safety
                //
                // Documents are buffered and only persisted AFTER WAL commit.
                // This means documents are NOT visible until flush_batch() is called.
                //
                // Trade-off: Delayed visibility for guaranteed durability.
                // Use Safe mode if you need immediate read-after-write.

                let collection = self.collection(collection_name)?;

                // 1. PREPARE phase: validate, generate ID, build WAL doc (NO storage write!)
                let prepared = collection.insert_one_prepare(document)?;

                // 2. Extract data before moving prepared into buffer
                let doc_id = prepared.doc_id.clone();
                let wal_doc = prepared.wal_doc.clone();

                // 3. BUFFER phase: store prepared doc for later persist
                {
                    let mut doc_buffer = self.batch_doc_buffer.write();
                    doc_buffer.add_insert(collection_name.to_string(), prepared);
                }

                // 4. Add operation to WAL batch buffer
                let should_flush = self.add_to_batch(Operation::Insert {
                    collection: collection_name.to_string(),
                    doc_id: doc_id.clone(),
                    doc: wal_doc,
                })?;

                // 5. Check if memory limit exceeded (early flush trigger)
                let memory_exceeded = {
                    let doc_buffer = self.batch_doc_buffer.read();
                    doc_buffer.memory_limit_exceeded()
                };

                // 6. Flush if batch is full OR memory limit exceeded
                if should_flush || memory_exceeded {
                    self.flush_batch()?;
                }

                Ok(doc_id)
            }

            DurabilityMode::Unsafe {
                auto_checkpoint_ops,
            } => {
                // Wait for active write transaction to complete (Read Committed isolation)
                self.wait_for_write_lock_release()?;

                // Unsafe mode: Fast path, optional auto-checkpoint
                let collection = self.collection(collection_name)?;
                let doc_id = collection.insert_one_raw(document)?;

                // Auto checkpoint if configured
                if let Some(threshold) = auto_checkpoint_ops {
                    let count = self.unsafe_op_counter.fetch_add(1, Ordering::Relaxed) + 1;
                    if count >= threshold as u64 {
                        self.unsafe_op_counter.store(0, Ordering::Relaxed);
                        self.checkpoint()?;
                    }
                }

                Ok(doc_id)
            }
        }
    }

    /// Update one document with WAL durability
    ///
    /// This method wraps update_one with proper WAL logging for crash recovery.
    /// The document's old and new state are both logged to enable undo/redo.
    ///
    /// Returns (matched_count, modified_count)
    pub fn update_one(
        &self,
        collection_name: &str,
        query: &Value,
        update: &Value,
    ) -> Result<(u64, u64)> {
        self.check_not_closed()?;
        match self.durability_mode {
            DurabilityMode::Safe => {
                // Wait for active write transaction to complete (Read Committed isolation)
                self.wait_for_write_lock_release()?;

                // Use get_collection - no implicit creation for update operations
                let collection = self.get_collection(collection_name)?;

                // Phase 6: Use prepare/persist pattern
                // PREPARE: Find doc, apply update, write to storage (atomic under lock)
                let prepared = collection.update_one_prepare(query, update)?;

                if prepared.matched == 0 {
                    return Ok((0, 0)); // No match, nothing to update
                }

                // If modified, add to WAL
                if prepared.modified > 0 {
                    let mut auto_tx = self.begin_auto_transaction();

                    // Extract doc_id - invariant: doc_id is always Some when modified > 0
                    let doc_id = prepared.doc_id.clone().ok_or_else(|| {
                        IronBaseError::Corruption(
                            "update_one_prepare: modified > 0 but doc_id is None".into(),
                        )
                    })?;

                    auto_tx.add_operation(Operation::Update {
                        collection: collection_name.to_string(),
                        doc_id,
                        old_doc: prepared.old_doc.clone().unwrap_or(Value::Null),
                        new_doc: prepared.new_doc.clone().unwrap_or(Value::Null),
                    })?;
                    auto_tx.mark_operations_applied();

                    // Auto-commit (WAL write + fsync)
                    self.commit_auto_transaction(auto_tx)?;
                }

                // PERSIST: Cache invalidation only (storage already written in prepare)
                collection.update_one_persist(prepared)
            }

            DurabilityMode::Batch { .. } => {
                // Wait for active write transaction to complete (Read Committed isolation)
                self.wait_for_write_lock_release()?;

                // WAL ORDERING FIX: Use prepare_batch pattern (NO storage write in prepare)
                // 1. PREPARE: Find doc, compute update, NO storage write
                let collection = self.get_collection(collection_name)?;
                let prepared = match collection.update_one_prepare_batch(query, update)? {
                    Some(p) => p,
                    None => return Ok((0, 0)), // No match
                };

                // 2. BUFFER: Store prepared data for later persist
                let doc_id = prepared.doc_id.clone();
                let old_doc = prepared.old_doc.clone();
                let new_doc = prepared.new_doc.clone();

                {
                    let mut doc_buffer = self.batch_doc_buffer.write();
                    doc_buffer.add_update(collection_name.to_string(), prepared);
                }

                // 3. Add operation to WAL batch buffer
                let should_flush = self.add_to_batch(Operation::Update {
                    collection: collection_name.to_string(),
                    doc_id,
                    old_doc,
                    new_doc,
                })?;

                // 4. Check if memory limit exceeded (early flush trigger)
                let memory_exceeded = {
                    let doc_buffer = self.batch_doc_buffer.read();
                    doc_buffer.memory_limit_exceeded()
                };

                // 5. Flush if batch is full OR memory limit exceeded
                if should_flush || memory_exceeded {
                    self.flush_batch()?;
                }

                Ok((1, 1))
            }

            DurabilityMode::Unsafe {
                auto_checkpoint_ops,
            } => {
                // Wait for active write transaction to complete (Read Committed isolation)
                self.wait_for_write_lock_release()?;

                // Use get_collection - no implicit creation for update operations
                let collection = self.get_collection(collection_name)?;
                let result = collection.update_one_raw(query, update)?;

                if let Some(threshold) = auto_checkpoint_ops {
                    let count = self.unsafe_op_counter.fetch_add(1, Ordering::Relaxed) + 1;
                    if count >= threshold as u64 {
                        self.unsafe_op_counter.store(0, Ordering::Relaxed);
                        self.checkpoint()?;
                    }
                }

                Ok(result)
            }
        }
    }

    /// Delete one document with WAL durability
    ///
    /// This method wraps delete_one with proper WAL logging for crash recovery.
    /// The deleted document is logged for potential rollback.
    ///
    /// Returns deleted_count
    pub fn delete_one(&self, collection_name: &str, query: &Value) -> Result<u64> {
        self.check_not_closed()?;
        match self.durability_mode {
            DurabilityMode::Safe => {
                // Wait for active write transaction to complete (Read Committed isolation)
                self.wait_for_write_lock_release()?;

                // Use get_collection - no implicit creation for delete operations
                let collection = self.get_collection(collection_name)?;

                // Phase 6: Use prepare/persist pattern
                // PREPARE: Find doc, write tombstone (atomic under lock)
                let prepared = collection.delete_one_prepare(query)?;

                if prepared.deleted == 0 {
                    return Ok(0); // No match, nothing to delete
                }

                // If deleted, add to WAL
                let mut auto_tx = self.begin_auto_transaction();

                // Extract doc_id - invariant: doc_id is always Some when deleted > 0
                let doc_id = prepared.doc_id.clone().ok_or_else(|| {
                    IronBaseError::Corruption(
                        "delete_one_prepare: deleted > 0 but doc_id is None".into(),
                    )
                })?;

                auto_tx.add_operation(Operation::Delete {
                    collection: collection_name.to_string(),
                    doc_id,
                    old_doc: prepared.old_doc.clone().unwrap_or(Value::Null),
                })?;
                auto_tx.mark_operations_applied();

                // Auto-commit (WAL write + fsync)
                self.commit_auto_transaction(auto_tx)?;

                // PERSIST: Cache invalidation only (storage already written in prepare)
                collection.delete_one_persist(prepared)
            }

            DurabilityMode::Batch { .. } => {
                // Wait for active write transaction to complete (Read Committed isolation)
                self.wait_for_write_lock_release()?;

                // WAL ORDERING FIX: Use prepare_batch pattern (NO tombstone write in prepare)
                // 1. PREPARE: Find doc, NO tombstone write
                let collection = self.get_collection(collection_name)?;
                let prepared = match collection.delete_one_prepare_batch(query)? {
                    Some(p) => p,
                    None => return Ok(0), // No match
                };

                // 2. BUFFER: Store prepared data for later persist
                let doc_id = prepared.doc_id.clone();
                let old_doc = prepared.old_doc.clone();

                {
                    let mut doc_buffer = self.batch_doc_buffer.write();
                    doc_buffer.add_delete(collection_name.to_string(), prepared);
                }

                // 3. Add operation to WAL batch buffer
                let should_flush = self.add_to_batch(Operation::Delete {
                    collection: collection_name.to_string(),
                    doc_id,
                    old_doc,
                })?;

                // 4. Check if memory limit exceeded (early flush trigger)
                let memory_exceeded = {
                    let doc_buffer = self.batch_doc_buffer.read();
                    doc_buffer.memory_limit_exceeded()
                };

                // 5. Flush if batch is full OR memory limit exceeded
                if should_flush || memory_exceeded {
                    self.flush_batch()?;
                }

                Ok(1)
            }

            DurabilityMode::Unsafe {
                auto_checkpoint_ops,
            } => {
                // Wait for active write transaction to complete (Read Committed isolation)
                self.wait_for_write_lock_release()?;

                // Use get_collection - no implicit creation for delete operations
                let collection = self.get_collection(collection_name)?;
                let deleted = collection.delete_one_raw(query)?;

                if let Some(threshold) = auto_checkpoint_ops {
                    let count = self.unsafe_op_counter.fetch_add(1, Ordering::Relaxed) + 1;
                    if count >= threshold as u64 {
                        self.unsafe_op_counter.store(0, Ordering::Relaxed);
                        self.checkpoint()?;
                    }
                }

                Ok(deleted)
            }
        }
    }

    /// Insert multiple documents with WAL durability
    ///
    /// Each document is logged individually to the WAL for crash recovery.
    ///
    /// Returns vector of inserted document IDs
    pub fn insert_many(
        &self,
        collection_name: &str,
        documents: Vec<HashMap<String, Value>>,
    ) -> Result<Vec<DocumentId>> {
        // Wait for active write transaction to complete (Read Committed isolation)
        self.wait_for_write_lock_release()?;

        match self.durability_mode {
            DurabilityMode::Safe => {
                // HYBRID LOCKING: Only acquire collection lock if there are unique indexes
                // Collections without unique indexes don't need the lock (no constraint races)
                let write_lock = self.get_collection_write_lock(collection_name);
                let _guard = if self.collection_has_unique_index(collection_name) {
                    Some(write_lock.lock())
                } else {
                    None
                };

                let collection = self.collection(collection_name)?;

                // 1. PREPARE phase: validate all documents, generate IDs, build WAL docs
                // This does all validation and constraint checking atomically
                let prepared = collection.insert_many_prepare(documents)?;

                if prepared.prepared_docs.is_empty() {
                    return Ok(Vec::new());
                }

                // 2. Begin auto-transaction and add all operations
                // Each prepared_doc.wal_doc already contains _id and _collection
                let mut auto_tx = self.begin_auto_transaction();
                let tx_id = auto_tx.id; // Save tx_id before commit consumes transaction
                for prep in &prepared.prepared_docs {
                    auto_tx.add_operation(Operation::Insert {
                        collection: collection_name.to_string(),
                        doc_id: prep.doc_id.clone(),
                        doc: prep.wal_doc.clone(),
                    })?;
                }

                // 3. Mark as applied (storage write follows WAL commit)
                auto_tx.mark_operations_applied();

                // 4. Auto-commit (WAL write + fsync)
                self.commit_auto_transaction(auto_tx)?;

                // 5. PERSIST phase: write all documents to storage after WAL is committed
                // If persist fails, write ABORT to WAL to prevent recovery replaying this tx
                match collection.insert_many_persist(prepared) {
                    Ok(inserted_ids) => Ok(inserted_ids),
                    Err(e) => {
                        // Persist failed - write ABORT to invalidate the committed WAL entry
                        let _ = self.abort_committed_transaction(tx_id);
                        Err(e)
                    }
                }
                // _guard dropped here - lock released
            }

            DurabilityMode::Batch { .. } => {
                let collection = self.collection(collection_name)?;

                // 1. PREPARE phase: validate all documents, generate IDs, build WAL docs
                let prepared = collection.insert_many_prepare(documents)?;

                if prepared.prepared_docs.is_empty() {
                    return Ok(Vec::new());
                }

                // 2. Collect WAL data before persist consumes prepared
                let wal_entries: Vec<_> = prepared
                    .prepared_docs
                    .iter()
                    .map(|p| (p.doc_id.clone(), p.wal_doc.clone()))
                    .collect();

                // 3. PERSIST phase: write all documents to storage
                // (Batch mode writes storage first, WAL later)
                let inserted_ids = collection.insert_many_persist(prepared)?;

                // 4. Add to WAL batch (wal_doc has _id and _collection)
                for (doc_id, wal_doc) in wal_entries {
                    let should_flush = self.add_to_batch(Operation::Insert {
                        collection: collection_name.to_string(),
                        doc_id,
                        doc: wal_doc,
                    })?;

                    if should_flush {
                        self.flush_batch()?;
                    }
                }

                Ok(inserted_ids)
            }

            DurabilityMode::Unsafe {
                auto_checkpoint_ops,
            } => {
                // Unsafe mode: Fast path, no WAL
                let collection = self.collection(collection_name)?;

                // insert_many_raw handles all validation and storage writes
                let result = collection.insert_many_raw(documents)?;

                // Auto checkpoint if configured
                if let Some(threshold) = auto_checkpoint_ops {
                    let count = self
                        .unsafe_op_counter
                        .fetch_add(result.inserted_count as u64, Ordering::Relaxed)
                        + result.inserted_count as u64;
                    if count >= threshold as u64 {
                        self.unsafe_op_counter.store(0, Ordering::Relaxed);
                        self.checkpoint()?;
                    }
                }

                Ok(result.inserted_ids)
            }
        }
    }

    /// Update multiple documents with WAL durability
    ///
    /// Each document update is logged to the WAL for crash recovery.
    /// All updates are committed in a single transaction.
    ///
    /// Returns (matched_count, modified_count)
    pub fn update_many(
        &self,
        collection_name: &str,
        query: &Value,
        update: &Value,
    ) -> Result<(u64, u64)> {
        // Wait for active write transaction to complete (Read Committed isolation)
        self.wait_for_write_lock_release()?;

        match self.durability_mode {
            DurabilityMode::Safe => {
                // Use get_collection - no implicit creation for update operations
                let collection = self.get_collection(collection_name)?;

                // BUG #1 FIX: Use prepare/persist pattern for correct WAL ordering
                // PHASE 1: PREPARE - compute updates in memory (NO storage writes!)
                let prepared = collection.update_many_prepare(query, update)?;

                // Save counts before moving prepared into persist
                let matched = prepared.matched;
                let modified = prepared.modified;

                if modified > 0 {
                    // PHASE 2: BUILD WAL from prepared results
                    let mut auto_tx = self.begin_auto_transaction();
                    let tx_id = auto_tx.id; // Save tx_id before commit consumes transaction
                    for (doc_id, old_doc, new_doc) in &prepared.wal_entries {
                        auto_tx.add_operation(Operation::Update {
                            collection: collection_name.to_string(),
                            doc_id: doc_id.clone(),
                            old_doc: old_doc.clone(),
                            new_doc: new_doc.clone(),
                        })?;
                    }

                    // PHASE 3: COMMIT WAL (fsync!) ← ATOMIC POINT
                    auto_tx.mark_operations_applied();
                    self.commit_auto_transaction(auto_tx)?;

                    // PHASE 4: PERSIST to storage (WAL is safe now)
                    // If persist fails, write ABORT to WAL to prevent recovery replaying this tx
                    if let Err(e) = collection.update_many_persist(prepared) {
                        let _ = self.abort_committed_transaction(tx_id);
                        return Err(e);
                    }
                }

                Ok((matched, modified))
            }

            DurabilityMode::Batch { .. } => {
                // Use get_collection - no implicit creation for update operations
                let collection = self.get_collection(collection_name)?;

                // BUG #1 FIX: Use prepare/persist pattern for correct WAL ordering
                // PHASE 1: PREPARE - compute updates in memory (NO storage writes!)
                let prepared = collection.update_many_prepare(query, update)?;

                // Save counts before moving prepared into persist
                let matched = prepared.matched;
                let modified = prepared.modified;

                if modified > 0 {
                    // PHASE 2: BUILD batch WAL from prepared results
                    for (doc_id, old_doc, new_doc) in &prepared.wal_entries {
                        let should_flush = self.add_to_batch(Operation::Update {
                            collection: collection_name.to_string(),
                            doc_id: doc_id.clone(),
                            old_doc: old_doc.clone(),
                            new_doc: new_doc.clone(),
                        })?;

                        if should_flush {
                            self.flush_batch()?;
                        }
                    }

                    // PHASE 3: PERSIST to storage (batch WAL handles durability)
                    collection.update_many_persist(prepared)?;
                }

                Ok((matched, modified))
            }

            DurabilityMode::Unsafe {
                auto_checkpoint_ops,
            } => {
                // Use get_collection - no implicit creation for update operations
                let collection = self.get_collection(collection_name)?;
                let result = collection.update_many_raw(query, update)?;

                if let Some(threshold) = auto_checkpoint_ops {
                    let count = self
                        .unsafe_op_counter
                        .fetch_add(result.1, Ordering::Relaxed)
                        + result.1;
                    if count >= threshold as u64 {
                        self.unsafe_op_counter.store(0, Ordering::Relaxed);
                        self.checkpoint()?;
                    }
                }

                Ok(result)
            }
        }
    }

    /// Delete multiple documents with WAL durability
    ///
    /// Each deleted document is logged to the WAL for crash recovery.
    /// All deletes are committed in a single transaction.
    ///
    /// Returns deleted_count
    pub fn delete_many(&self, collection_name: &str, query: &Value) -> Result<u64> {
        // Wait for active write transaction to complete (Read Committed isolation)
        self.wait_for_write_lock_release()?;

        match self.durability_mode {
            DurabilityMode::Safe => {
                // Use get_collection - no implicit creation for delete operations
                let collection = self.get_collection(collection_name)?;

                // BUG #1 FIX: Use prepare/persist pattern for correct WAL ordering
                // PHASE 1: PREPARE - identify deletions in memory (NO storage writes!)
                let prepared = collection.delete_many_prepare(query)?;

                // Save count before moving prepared into persist
                let deleted = prepared.deleted;

                if deleted > 0 {
                    // PHASE 2: BUILD WAL from prepared results
                    let mut auto_tx = self.begin_auto_transaction();
                    let tx_id = auto_tx.id; // Save tx_id before commit consumes transaction
                    for (doc_id, old_doc) in &prepared.wal_entries {
                        auto_tx.add_operation(Operation::Delete {
                            collection: collection_name.to_string(),
                            doc_id: doc_id.clone(),
                            old_doc: old_doc.clone(),
                        })?;
                    }

                    // PHASE 3: COMMIT WAL (fsync!) ← ATOMIC POINT
                    auto_tx.mark_operations_applied();
                    self.commit_auto_transaction(auto_tx)?;

                    // PHASE 4: PERSIST tombstones to storage (WAL is safe now)
                    // If persist fails, write ABORT to WAL to prevent recovery replaying this tx
                    if let Err(e) = collection.delete_many_persist(prepared) {
                        let _ = self.abort_committed_transaction(tx_id);
                        return Err(e);
                    }
                }

                Ok(deleted)
            }

            DurabilityMode::Batch { .. } => {
                // Use get_collection - no implicit creation for delete operations
                let collection = self.get_collection(collection_name)?;

                // BUG #1 FIX: Use prepare/persist pattern for correct WAL ordering
                // PHASE 1: PREPARE - identify deletions in memory (NO storage writes!)
                let prepared = collection.delete_many_prepare(query)?;

                // Save count before moving prepared into persist
                let deleted = prepared.deleted;

                if deleted > 0 {
                    // PHASE 2: BUILD batch WAL from prepared results
                    for (doc_id, old_doc) in &prepared.wal_entries {
                        let should_flush = self.add_to_batch(Operation::Delete {
                            collection: collection_name.to_string(),
                            doc_id: doc_id.clone(),
                            old_doc: old_doc.clone(),
                        })?;

                        if should_flush {
                            self.flush_batch()?;
                        }
                    }

                    // PHASE 3: PERSIST tombstones to storage (batch WAL handles durability)
                    collection.delete_many_persist(prepared)?;
                }

                Ok(deleted)
            }

            DurabilityMode::Unsafe {
                auto_checkpoint_ops,
            } => {
                // Use get_collection - no implicit creation for delete operations
                let collection = self.get_collection(collection_name)?;
                let deleted = collection.delete_many_raw(query)?;

                if let Some(threshold) = auto_checkpoint_ops {
                    let count =
                        self.unsafe_op_counter.fetch_add(deleted, Ordering::Relaxed) + deleted;
                    if count >= threshold as u64 {
                        self.unsafe_op_counter.store(0, Ordering::Relaxed);
                        self.checkpoint()?;
                    }
                }

                Ok(deleted)
            }
        }
    }
}

// ============================================================================
// MEMORYSTORAGE-SPECIFIC CRUD (no WAL)
// ============================================================================

impl DatabaseCore<MemoryStorage> {
    /// Insert one document (MemoryStorage version - no WAL/durability)
    ///
    /// For in-memory databases, this is a simple fast-path insert without
    /// WAL logging since data doesn't need to survive restarts.
    pub fn insert_one(
        &self,
        collection_name: &str,
        document: HashMap<String, Value>,
    ) -> Result<DocumentId> {
        self.check_not_closed()?;
        let collection = self.collection(collection_name)?;
        collection.insert_one_raw(document)
    }

    /// Update one document (MemoryStorage version - no WAL/durability)
    ///
    /// Returns (matched_count, modified_count)
    pub fn update_one(
        &self,
        collection_name: &str,
        query: &Value,
        update: &Value,
    ) -> Result<(u64, u64)> {
        self.check_not_closed()?;
        // Use get_collection - no implicit creation for update operations
        let collection = self.get_collection(collection_name)?;
        collection.update_one_raw(query, update)
    }

    /// Delete one document (MemoryStorage version - no WAL/durability)
    ///
    /// Returns deleted_count
    pub fn delete_one(&self, collection_name: &str, query: &Value) -> Result<u64> {
        self.check_not_closed()?;
        // Use get_collection - no implicit creation for delete operations
        let collection = self.get_collection(collection_name)?;
        collection.delete_one_raw(query)
    }

    /// Insert many documents (MemoryStorage version - no WAL/durability)
    ///
    /// Returns vector of inserted document IDs
    pub fn insert_many(
        &self,
        collection_name: &str,
        documents: Vec<HashMap<String, Value>>,
    ) -> Result<Vec<DocumentId>> {
        self.check_not_closed()?;
        let collection = self.collection(collection_name)?;
        let result = collection.insert_many_raw(documents)?;
        Ok(result.inserted_ids)
    }

    /// Update many documents (MemoryStorage version - no WAL/durability)
    ///
    /// Returns (matched_count, modified_count)
    pub fn update_many(
        &self,
        collection_name: &str,
        query: &Value,
        update: &Value,
    ) -> Result<(u64, u64)> {
        self.check_not_closed()?;
        // Use get_collection - no implicit creation for update operations
        let collection = self.get_collection(collection_name)?;
        collection.update_many_raw(query, update)
    }

    /// Delete many documents (MemoryStorage version - no WAL/durability)
    ///
    /// Returns deleted_count
    pub fn delete_many(&self, collection_name: &str, query: &Value) -> Result<u64> {
        self.check_not_closed()?;
        // Use get_collection - no implicit creation for delete operations
        let collection = self.get_collection(collection_name)?;
        collection.delete_many_raw(query)
    }
}
