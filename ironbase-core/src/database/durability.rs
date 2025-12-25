// src/database/durability.rs
// Auto-commit CRUD operations with durability mode handling

use std::collections::HashMap;
use std::sync::atomic::Ordering;

use serde_json::Value;

use crate::collection_core::RawOperations;
use crate::document::DocumentId;
use crate::durability::DurabilityMode;
use crate::error::Result;
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

    /// Flush batch operations to WAL
    ///
    /// Used by Batch mode when batch_buffer reaches batch_size.
    /// Creates a single transaction with all buffered operations.
    pub(crate) fn flush_batch(&self) -> Result<()> {
        let mut batch = self.batch_buffer.write();

        if batch.is_empty() {
            return Ok(());
        }

        // Create auto-transaction with all operations
        let mut auto_tx = self.begin_auto_transaction();

        for op in batch.iter() {
            auto_tx.add_operation(op.clone())?;
        }
        // Operations in batch were already applied when enqueued
        auto_tx.mark_operations_applied();

        // Commit with batch mode (WAL sync only, skip file sync)
        self.commit_auto_transaction_batch(auto_tx)?;

        // Clear batch
        batch.clear();

        // Now sync the file once for the entire batch
        // This is the key optimization: one fsync per batch instead of per transaction
        drop(batch); // Release the batch lock before sync
        self.sync_storage_file()?;

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
    /// # Ok::<(), ironbase_core::MongoLiteError>(())
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

                // Batch mode: Add to batch, flush when full
                let collection = self.collection(collection_name)?;

                // 1. PREPARE phase: validate, generate ID, build WAL doc
                let prepared = collection.insert_one_prepare(document)?;

                // 2. Extract WAL data before persist consumes prepared
                let wal_doc = prepared.wal_doc.clone();
                let doc_id_for_batch = prepared.doc_id.clone();

                // 3. PERSIST phase: write to storage immediately
                // (Batch mode writes storage first, WAL later)
                let doc_id = collection.insert_one_persist(prepared)?;

                // 4. Add to batch buffer (wal_doc has _id and _collection)
                let should_flush = self.add_to_batch(Operation::Insert {
                    collection: collection_name.to_string(),
                    doc_id: doc_id_for_batch,
                    doc: wal_doc,
                })?;

                // 5. Flush if batch is full
                if should_flush {
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

                    // Extract doc_id - prepared.doc_id is always Some when matched > 0
                    let doc_id = prepared.doc_id.clone().unwrap();

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

                // Use get_collection - no implicit creation for update operations
                let collection = self.get_collection(collection_name)?;
                let old_doc = collection.find_one(query)?;
                if old_doc.is_none() {
                    return Ok((0, 0));
                }
                let old_doc = old_doc.unwrap();

                let doc_id = super::extract_doc_id(&old_doc)?;

                let (matched, modified) = collection.update_one_raw(query, update)?;

                if modified > 0 {
                    let new_doc = collection
                        .find_one(&serde_json::json!({"_id": &doc_id}))?
                        .unwrap_or(old_doc.clone());

                    let should_flush = self.add_to_batch(Operation::Update {
                        collection: collection_name.to_string(),
                        doc_id,
                        old_doc,
                        new_doc,
                    })?;

                    if should_flush {
                        self.flush_batch()?;
                    }
                }

                Ok((matched, modified))
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

                // Extract doc_id - prepared.doc_id is always Some when deleted > 0
                let doc_id = prepared.doc_id.clone().unwrap();

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

                // Use get_collection - no implicit creation for delete operations
                let collection = self.get_collection(collection_name)?;
                let old_doc = collection.find_one(query)?;
                if old_doc.is_none() {
                    return Ok(0);
                }
                let old_doc = old_doc.unwrap();

                let doc_id = super::extract_doc_id(&old_doc)?;

                let deleted = collection.delete_one_raw(query)?;

                if deleted > 0 {
                    let should_flush = self.add_to_batch(Operation::Delete {
                        collection: collection_name.to_string(),
                        doc_id,
                        old_doc,
                    })?;

                    if should_flush {
                        self.flush_batch()?;
                    }
                }

                Ok(deleted)
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
