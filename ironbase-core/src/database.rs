// ironbase-core/src/database.rs
// Pure Rust database API - NO PyO3 dependencies

use parking_lot::RwLock;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::collection_core::{CollectionCore, RawOperations};
use crate::document::DocumentId;
use crate::durability::DurabilityMode;
use crate::error::{MongoLiteError, Result};
use crate::index::IndexManager;
use crate::storage::{MemoryStorage, RawStorage, Storage, StorageEngine};
use crate::transaction::{Operation, Transaction, TransactionId};
use serde_json::Value;

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

/// Convert transaction::IndexKey to index::IndexKey
fn convert_index_key(tx_key: &crate::transaction::IndexKey) -> crate::index::IndexKey {
    match tx_key {
        crate::transaction::IndexKey::Int(i) => crate::index::IndexKey::Int(*i),
        crate::transaction::IndexKey::String(s) => crate::index::IndexKey::String(s.clone()),
        crate::transaction::IndexKey::Float(f) => {
            crate::index::IndexKey::Float(crate::index::OrderedFloat(f.value()))
        }
        crate::transaction::IndexKey::Bool(b) => crate::index::IndexKey::Bool(*b),
        crate::transaction::IndexKey::Null => crate::index::IndexKey::Null,
    }
}

/// Extract DocumentId from a JSON Value's _id field
///
/// Handles Int, String, and ObjectId (24-char hex string) formats.
/// Returns error if _id is missing or has invalid type.
fn extract_doc_id(doc: &Value) -> Result<DocumentId> {
    DocumentId::try_from_value(doc).ok_or_else(|| {
        crate::error::MongoLiteError::InvalidQuery("Document missing _id".to_string())
    })
}

/// Pure Rust IronBase Database - language-independent
///
/// Generic over Storage backend:
/// - `DatabaseCore<StorageEngine>` - Production file-based storage (default)
/// - `DatabaseCore<MemoryStorage>` - Fast in-memory storage for testing
pub struct DatabaseCore<S: Storage + RawStorage> {
    storage: Arc<RwLock<S>>,
    db_path: String,
    next_tx_id: AtomicU64,
    active_transactions: Arc<RwLock<std::collections::HashMap<TransactionId, Transaction>>>,

    // NEW: Durability mode (safe by default like SQL databases)
    durability_mode: DurabilityMode,

    // NEW: Batch buffer for Batch mode
    batch_buffer: Arc<RwLock<Vec<Operation>>>,

    // NEW: Operation counter for Unsafe mode auto-checkpoint
    unsafe_op_counter: AtomicU64,

    // Shared IndexManagers per collection (fixes stale index problem)
    // Each collection shares its IndexManager across all CollectionCore instances
    index_managers: Arc<RwLock<HashMap<String, Arc<RwLock<IndexManager>>>>>,
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
    /// ```rust
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
    /// # Ok::<(), ironbase_core::MongoLiteError>(())
    /// ```
    pub fn open_with_durability<P: AsRef<Path>>(path: P, mode: DurabilityMode) -> Result<Self> {
        let path_str = path.as_ref().to_string_lossy().to_string();
        let mut storage = StorageEngine::open(&path_str)?;

        // Recover from WAL (includes both data and index changes)
        let (_wal_entries, recovered_index_changes) = storage.recover_from_wal()?;

        // NOTE: WAL recovery now uses write_document() which updates the catalog.
        // The document_catalog is loaded from metadata by StorageEngine::open(),
        // and recover_from_wal() properly updates it for any recovered operations.

        // Create DatabaseCore instance with specified mode
        let db = DatabaseCore {
            storage: Arc::new(RwLock::new(storage)),
            db_path: path_str,
            next_tx_id: AtomicU64::new(1),
            active_transactions: Arc::new(RwLock::new(std::collections::HashMap::new())),
            durability_mode: mode,
            batch_buffer: Arc::new(RwLock::new(Vec::new())),
            unsafe_op_counter: AtomicU64::new(0),
            index_managers: Arc::new(RwLock::new(HashMap::new())),
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
                        // Convert transaction::IndexKey to index::IndexKey
                        let index_key = convert_index_key(&change.key);

                        match change.operation {
                            crate::transaction::IndexOperation::Insert => {
                                btree_index.insert(index_key, change.doc_id)?;
                            }
                            crate::transaction::IndexOperation::Delete => {
                                btree_index.delete(&index_key, &change.doc_id)?;
                            }
                        }
                    }
                }
            }
        }

        Ok(db)
    }

    /// Get database statistics as JSON (StorageEngine-specific)
    pub fn stats(&self) -> serde_json::Value {
        let storage = self.storage.read();
        storage.stats()
    }

    /// Storage compaction - removes tombstones and old document versions (StorageEngine-specific)
    pub fn compact(&self) -> Result<crate::storage::CompactionStats> {
        let mut storage = self.storage.write();
        storage.compact()
    }

    /// Commit a transaction (applies all buffered operations atomically) - StorageEngine-specific
    pub fn commit_transaction(&self, tx_id: TransactionId) -> Result<()> {
        // Remove transaction from active list
        let mut transaction = {
            let mut active = self.active_transactions.write();
            active.remove(&tx_id).ok_or_else(|| {
                crate::error::MongoLiteError::TransactionAborted(format!(
                    "Transaction {} not found",
                    tx_id
                ))
            })?
        };

        // Commit through storage engine
        let mut storage = self.storage.write();
        storage.commit_transaction(&mut transaction)?;

        Ok(())
    }

    /// Rollback a transaction (discard all buffered operations) - StorageEngine-specific
    pub fn rollback_transaction(&self, tx_id: TransactionId) -> Result<()> {
        // Remove transaction from active list
        let mut transaction = {
            let mut active = self.active_transactions.write();
            active.remove(&tx_id).ok_or_else(|| {
                crate::error::MongoLiteError::TransactionAborted(format!(
                    "Transaction {} not found",
                    tx_id
                ))
            })?
        };

        // Rollback through storage engine
        let mut storage = self.storage.write();
        storage.rollback_transaction(&mut transaction)?;

        Ok(())
    }

    /// Commit transaction with index operations - StorageEngine-specific
    pub fn commit_transaction_with_indexes(&self, tx_id: TransactionId) -> Result<()> {
        // Remove transaction from active list
        let mut transaction = {
            let mut active = self.active_transactions.write();
            active.remove(&tx_id).ok_or_else(|| {
                crate::error::MongoLiteError::TransactionAborted(format!(
                    "Transaction {} not found",
                    tx_id
                ))
            })?
        };

        // Commit through storage engine with index operations
        let mut storage = self.storage.write();
        storage.commit_transaction(&mut transaction)?;

        Ok(())
    }

    // ========== Auto-Commit Transaction Helpers (StorageEngine-specific, INTERNAL) ==========

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

        // Commit (WAL + fsync)
        self.commit_auto_transaction(auto_tx)?;

        // Clear batch
        batch.clear();

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
        match self.durability_mode {
            DurabilityMode::Safe => {
                // Safe mode: Auto-commit every operation
                let collection = self.collection(collection_name)?;

                // 1. Begin auto-transaction
                let mut auto_tx = self.begin_auto_transaction();

                // 2. Execute insert
                let doc_id = collection.insert_one_raw(document.clone())?;

                // 3. Add operation to transaction
                // IMPORTANT: WAL must contain the FULL document with _id and _collection
                // so that recovery can rebuild the catalog correctly
                let mut doc_with_metadata = document.clone();
                doc_with_metadata.insert(
                    "_id".to_string(),
                    serde_json::to_value(&doc_id)
                        .map_err(|e| crate::error::MongoLiteError::Serialization(e.to_string()))?,
                );
                doc_with_metadata.insert(
                    "_collection".to_string(),
                    Value::String(collection_name.to_string()),
                );
                let doc_value = serde_json::to_value(&doc_with_metadata)
                    .map_err(|e| crate::error::MongoLiteError::Serialization(e.to_string()))?;
                auto_tx.add_operation(Operation::Insert {
                    collection: collection_name.to_string(),
                    doc_id: doc_id.clone(),
                    doc: doc_value,
                })?;
                // The insert has already been applied; mark to avoid double-apply
                auto_tx.mark_operations_applied();

                // 4. Auto-commit (WAL write + fsync)
                self.commit_auto_transaction(auto_tx)?;

                Ok(doc_id)
            }

            DurabilityMode::Batch { .. } => {
                // Batch mode: Add to batch, flush when full
                let collection = self.collection(collection_name)?;

                // 1. Execute insert
                let doc_id = collection.insert_one_raw(document.clone())?;

                // 2. Add to batch buffer
                // IMPORTANT: WAL must contain the FULL document with _id and _collection
                let mut doc_with_metadata = document.clone();
                doc_with_metadata.insert(
                    "_id".to_string(),
                    serde_json::to_value(&doc_id)
                        .map_err(|e| crate::error::MongoLiteError::Serialization(e.to_string()))?,
                );
                doc_with_metadata.insert(
                    "_collection".to_string(),
                    Value::String(collection_name.to_string()),
                );
                let doc_value = serde_json::to_value(&doc_with_metadata)
                    .map_err(|e| crate::error::MongoLiteError::Serialization(e.to_string()))?;
                let should_flush = self.add_to_batch(Operation::Insert {
                    collection: collection_name.to_string(),
                    doc_id: doc_id.clone(),
                    doc: doc_value,
                })?;

                // 3. Flush if batch is full
                if should_flush {
                    self.flush_batch()?;
                }

                Ok(doc_id)
            }

            DurabilityMode::Unsafe {
                auto_checkpoint_ops,
            } => {
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
        match self.durability_mode {
            DurabilityMode::Safe => {
                let collection = self.collection(collection_name)?;

                // 1. Find the document BEFORE update (for WAL old_doc)
                let old_doc = collection.find_one(query)?;
                if old_doc.is_none() {
                    return Ok((0, 0)); // No match, nothing to update
                }
                let old_doc = old_doc.unwrap();

                let doc_id = extract_doc_id(&old_doc)?;

                // 2. Begin auto-transaction
                let mut auto_tx = self.begin_auto_transaction();

                // 3. Execute update
                let (matched, modified) = collection.update_one_raw(query, update)?;

                // 4. If modified, get new state and add to WAL
                if modified > 0 {
                    // Find the updated document
                    let new_doc = collection
                        .find_one(&serde_json::json!({"_id": &doc_id}))?
                        .unwrap_or(old_doc.clone());

                    auto_tx.add_operation(Operation::Update {
                        collection: collection_name.to_string(),
                        doc_id: doc_id.clone(),
                        old_doc,
                        new_doc,
                    })?;
                    auto_tx.mark_operations_applied();

                    // 5. Auto-commit (WAL write + fsync)
                    self.commit_auto_transaction(auto_tx)?;
                }

                Ok((matched, modified))
            }

            DurabilityMode::Batch { .. } => {
                let collection = self.collection(collection_name)?;
                let old_doc = collection.find_one(query)?;
                if old_doc.is_none() {
                    return Ok((0, 0));
                }
                let old_doc = old_doc.unwrap();

                let doc_id = extract_doc_id(&old_doc)?;

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
                let collection = self.collection(collection_name)?;
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
        match self.durability_mode {
            DurabilityMode::Safe => {
                let collection = self.collection(collection_name)?;

                // 1. Find the document BEFORE delete (for WAL old_doc)
                let old_doc = collection.find_one(query)?;
                if old_doc.is_none() {
                    return Ok(0); // No match, nothing to delete
                }
                let old_doc = old_doc.unwrap();

                // Extract doc_id
                let doc_id = extract_doc_id(&old_doc)?;

                // 2. Begin auto-transaction
                let mut auto_tx = self.begin_auto_transaction();

                // 3. Execute delete
                let deleted = collection.delete_one_raw(query)?;

                // 4. If deleted, add to WAL
                if deleted > 0 {
                    auto_tx.add_operation(Operation::Delete {
                        collection: collection_name.to_string(),
                        doc_id,
                        old_doc,
                    })?;
                    auto_tx.mark_operations_applied();

                    // 5. Auto-commit (WAL write + fsync)
                    self.commit_auto_transaction(auto_tx)?;
                }

                Ok(deleted)
            }

            DurabilityMode::Batch { .. } => {
                let collection = self.collection(collection_name)?;
                let old_doc = collection.find_one(query)?;
                if old_doc.is_none() {
                    return Ok(0);
                }
                let old_doc = old_doc.unwrap();

                let doc_id = extract_doc_id(&old_doc)?;

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
                let collection = self.collection(collection_name)?;
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
        match self.durability_mode {
            DurabilityMode::Safe => {
                let collection = self.collection(collection_name)?;

                // 🔒 FIX #17: Pre-validate batch for duplicates BEFORE any insert
                // This ensures atomic failure - either all documents insert or none.
                collection.validate_batch_constraints(&documents)?;

                let mut auto_tx = self.begin_auto_transaction();
                let mut inserted_ids = Vec::with_capacity(documents.len());

                for document in documents {
                    let doc_id = collection.insert_one_raw(document.clone())?;

                    // Add full document to WAL
                    let mut doc_with_metadata = document.clone();
                    doc_with_metadata.insert(
                        "_id".to_string(),
                        serde_json::to_value(&doc_id).map_err(|e| {
                            crate::error::MongoLiteError::Serialization(e.to_string())
                        })?,
                    );
                    doc_with_metadata.insert(
                        "_collection".to_string(),
                        Value::String(collection_name.to_string()),
                    );
                    let doc_value = serde_json::to_value(&doc_with_metadata)
                        .map_err(|e| crate::error::MongoLiteError::Serialization(e.to_string()))?;

                    auto_tx.add_operation(Operation::Insert {
                        collection: collection_name.to_string(),
                        doc_id: doc_id.clone(),
                        doc: doc_value,
                    })?;

                    inserted_ids.push(doc_id);
                }

                auto_tx.mark_operations_applied();
                self.commit_auto_transaction(auto_tx)?;

                Ok(inserted_ids)
            }

            DurabilityMode::Batch { .. } => {
                let collection = self.collection(collection_name)?;

                // 🔒 FIX #17: Pre-validate batch for duplicates BEFORE any insert
                collection.validate_batch_constraints(&documents)?;

                let mut inserted_ids = Vec::with_capacity(documents.len());

                for document in documents {
                    let doc_id = collection.insert_one_raw(document.clone())?;

                    let mut doc_with_metadata = document.clone();
                    doc_with_metadata.insert(
                        "_id".to_string(),
                        serde_json::to_value(&doc_id).map_err(|e| {
                            crate::error::MongoLiteError::Serialization(e.to_string())
                        })?,
                    );
                    doc_with_metadata.insert(
                        "_collection".to_string(),
                        Value::String(collection_name.to_string()),
                    );
                    let doc_value = serde_json::to_value(&doc_with_metadata)
                        .map_err(|e| crate::error::MongoLiteError::Serialization(e.to_string()))?;

                    let should_flush = self.add_to_batch(Operation::Insert {
                        collection: collection_name.to_string(),
                        doc_id: doc_id.clone(),
                        doc: doc_value,
                    })?;

                    if should_flush {
                        self.flush_batch()?;
                    }

                    inserted_ids.push(doc_id);
                }

                Ok(inserted_ids)
            }

            DurabilityMode::Unsafe {
                auto_checkpoint_ops,
            } => {
                let collection = self.collection(collection_name)?;

                // 🔒 FIX #17: Pre-validate batch for duplicates BEFORE any insert
                collection.validate_batch_constraints(&documents)?;

                let mut inserted_ids = Vec::with_capacity(documents.len());

                for document in documents {
                    let doc_id = collection.insert_one_raw(document)?;
                    inserted_ids.push(doc_id);
                }

                if let Some(threshold) = auto_checkpoint_ops {
                    let count = self
                        .unsafe_op_counter
                        .fetch_add(inserted_ids.len() as u64, Ordering::Relaxed)
                        + inserted_ids.len() as u64;
                    if count >= threshold as u64 {
                        self.unsafe_op_counter.store(0, Ordering::Relaxed);
                        self.checkpoint()?;
                    }
                }

                Ok(inserted_ids)
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
        match self.durability_mode {
            DurabilityMode::Safe => {
                let collection = self.collection(collection_name)?;

                // 1. Find all matching documents BEFORE update
                let old_docs = collection.find(query)?;
                if old_docs.is_empty() {
                    return Ok((0, 0));
                }

                // 2. Begin auto-transaction
                let mut auto_tx = self.begin_auto_transaction();

                // 3. Execute update_many
                let (matched, modified) = collection.update_many_raw(query, update)?;

                // 4. For each modified document, add WAL entry
                if modified > 0 {
                    for old_doc in old_docs.iter() {
                        let Some(doc_id) = DocumentId::try_from_value(old_doc) else {
                            continue; // Skip docs without valid _id
                        };

                        // Find the updated document
                        if let Ok(Some(new_doc)) =
                            collection.find_one(&serde_json::json!({"_id": &doc_id}))
                        {
                            auto_tx.add_operation(Operation::Update {
                                collection: collection_name.to_string(),
                                doc_id,
                                old_doc: old_doc.clone(),
                                new_doc,
                            })?;
                        }
                    }
                    auto_tx.mark_operations_applied();
                    self.commit_auto_transaction(auto_tx)?;
                }

                Ok((matched, modified))
            }

            DurabilityMode::Batch { .. } => {
                let collection = self.collection(collection_name)?;
                let old_docs = collection.find(query)?;
                if old_docs.is_empty() {
                    return Ok((0, 0));
                }

                let (matched, modified) = collection.update_many_raw(query, update)?;

                if modified > 0 {
                    for old_doc in old_docs.iter() {
                        let doc_id = match old_doc.get("_id") {
                            Some(Value::Number(n)) => DocumentId::Int(n.as_i64().unwrap_or(0)),
                            Some(Value::String(s)) => {
                                if s.len() == 24 && s.chars().all(|c| c.is_ascii_hexdigit()) {
                                    DocumentId::ObjectId(s.clone())
                                } else {
                                    DocumentId::String(s.clone())
                                }
                            }
                            _ => continue,
                        };

                        if let Ok(Some(new_doc)) =
                            collection.find_one(&serde_json::json!({"_id": &doc_id}))
                        {
                            let should_flush = self.add_to_batch(Operation::Update {
                                collection: collection_name.to_string(),
                                doc_id,
                                old_doc: old_doc.clone(),
                                new_doc,
                            })?;

                            if should_flush {
                                self.flush_batch()?;
                            }
                        }
                    }
                }

                Ok((matched, modified))
            }

            DurabilityMode::Unsafe {
                auto_checkpoint_ops,
            } => {
                let collection = self.collection(collection_name)?;
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
        match self.durability_mode {
            DurabilityMode::Safe => {
                let collection = self.collection(collection_name)?;

                // 1. Find all matching documents BEFORE delete
                let old_docs = collection.find(query)?;
                if old_docs.is_empty() {
                    return Ok(0);
                }

                // 2. Begin auto-transaction
                let mut auto_tx = self.begin_auto_transaction();

                // 3. Execute delete_many
                let deleted = collection.delete_many_raw(query)?;

                // 4. For each deleted document, add WAL entry
                if deleted > 0 {
                    for old_doc in old_docs {
                        let doc_id = match old_doc.get("_id") {
                            Some(Value::Number(n)) => DocumentId::Int(n.as_i64().unwrap_or(0)),
                            Some(Value::String(s)) => {
                                if s.len() == 24 && s.chars().all(|c| c.is_ascii_hexdigit()) {
                                    DocumentId::ObjectId(s.clone())
                                } else {
                                    DocumentId::String(s.clone())
                                }
                            }
                            _ => continue,
                        };

                        auto_tx.add_operation(Operation::Delete {
                            collection: collection_name.to_string(),
                            doc_id,
                            old_doc,
                        })?;
                    }
                    auto_tx.mark_operations_applied();
                    self.commit_auto_transaction(auto_tx)?;
                }

                Ok(deleted)
            }

            DurabilityMode::Batch { .. } => {
                let collection = self.collection(collection_name)?;
                let old_docs = collection.find(query)?;
                if old_docs.is_empty() {
                    return Ok(0);
                }

                let deleted = collection.delete_many_raw(query)?;

                if deleted > 0 {
                    for old_doc in old_docs {
                        let doc_id = match old_doc.get("_id") {
                            Some(Value::Number(n)) => DocumentId::Int(n.as_i64().unwrap_or(0)),
                            Some(Value::String(s)) => {
                                if s.len() == 24 && s.chars().all(|c| c.is_ascii_hexdigit()) {
                                    DocumentId::ObjectId(s.clone())
                                } else {
                                    DocumentId::String(s.clone())
                                }
                            }
                            _ => continue,
                        };

                        let should_flush = self.add_to_batch(Operation::Delete {
                            collection: collection_name.to_string(),
                            doc_id,
                            old_doc,
                        })?;

                        if should_flush {
                            self.flush_batch()?;
                        }
                    }
                }

                Ok(deleted)
            }

            DurabilityMode::Unsafe {
                auto_checkpoint_ops,
            } => {
                let collection = self.collection(collection_name)?;
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

    // ========== Two-Phase Commit Helper Methods (StorageEngine-specific) ==========

    /// Construct index file path for a collection's index
    /// Format: {db_path_without_ext}.{index_name}.idx
    ///
    /// Example: "/data/myapp.mlite" + "users_age" → "/data/myapp.users_age.idx"
    #[cfg(test)]
    fn get_index_file_path(&self, _collection_name: &str, index_name: &str) -> std::path::PathBuf {
        use std::path::PathBuf;

        let mut path = PathBuf::from(&self.db_path);

        // Remove .mlite extension if present
        if path.extension().map(|e| e == "mlite").unwrap_or(false) {
            path.set_extension("");
        }

        // Append index name and .idx extension
        let index_file = format!("{}.{}.idx", path.display(), index_name);
        PathBuf::from(index_file)
    }

    /// Extract collection name from transaction's first operation
    #[cfg(test)]
    fn get_collection_from_transaction(transaction: &Transaction) -> Option<String> {
        transaction.operations().first().map(|op| match op {
            crate::transaction::Operation::Insert { collection, .. } => collection.clone(),
            crate::transaction::Operation::Update { collection, .. } => collection.clone(),
            crate::transaction::Operation::Delete { collection, .. } => collection.clone(),
        })
    }
}

// ============================================================================
// MEMORYSTORAGE-SPECIFIC IMPLEMENTATION (in-memory, no WAL)
// ============================================================================

impl BatchFlush for DatabaseCore<MemoryStorage> {
    fn flush_pending_batch(&self) -> Result<()> {
        // No-op for MemoryStorage (no persistence)
        Ok(())
    }
}

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
    /// # Ok::<(), ironbase_core::MongoLiteError>(())
    /// ```
    pub fn open_memory() -> Result<Self> {
        let storage = MemoryStorage::new();

        Ok(DatabaseCore {
            storage: Arc::new(RwLock::new(storage)),
            db_path: String::new(), // No file path for memory storage
            next_tx_id: AtomicU64::new(1),
            active_transactions: Arc::new(RwLock::new(std::collections::HashMap::new())),
            durability_mode: DurabilityMode::default(),
            batch_buffer: Arc::new(RwLock::new(Vec::new())),
            unsafe_op_counter: AtomicU64::new(0),
            index_managers: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    // ========== CRUD Operations (MemoryStorage - no WAL) ==========

    /// Insert one document (MemoryStorage version - no WAL/durability)
    ///
    /// For in-memory databases, this is a simple fast-path insert without
    /// WAL logging since data doesn't need to survive restarts.
    pub fn insert_one(
        &self,
        collection_name: &str,
        document: HashMap<String, Value>,
    ) -> Result<DocumentId> {
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
        let collection = self.collection(collection_name)?;
        collection.update_one_raw(query, update)
    }

    /// Delete one document (MemoryStorage version - no WAL/durability)
    ///
    /// Returns deleted_count
    pub fn delete_one(&self, collection_name: &str, query: &Value) -> Result<u64> {
        let collection = self.collection(collection_name)?;
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
        let collection = self.collection(collection_name)?;
        collection.update_many_raw(query, update)
    }

    /// Delete many documents (MemoryStorage version - no WAL/durability)
    ///
    /// Returns deleted_count
    pub fn delete_many(&self, collection_name: &str, query: &Value) -> Result<u64> {
        let collection = self.collection(collection_name)?;
        collection.delete_many_raw(query)
    }
}

// ============================================================================
// GENERIC IMPLEMENTATION (all storage backends)
// ============================================================================

impl<S: Storage + RawStorage> DatabaseCore<S> {
    /// Get or create a shared IndexManager for a collection
    ///
    /// This method uses double-checked locking to ensure thread-safe creation
    /// of IndexManagers while minimizing lock contention.
    fn get_or_create_index_manager(&self, name: &str) -> Result<Arc<RwLock<IndexManager>>> {
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
        use crate::index::IndexKey;
        use crate::log_warn;

        let mut rebuilt_count = 0u64;

        // Pre-collect fuzzy index info to avoid repeated lookups in the loop
        let fuzzy_info: Vec<_> = index_manager
            .list_fuzzy_indexes()
            .iter()
            .map(|idx| (idx.metadata.name.clone(), idx.metadata.field.clone()))
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
                let _ = id_index.insert(index_key, doc_id.clone());
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
                    let _ = index.insert(key, doc_id.clone());
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
        index_manager.create_btree_index(id_index_name.clone(), "_id".to_string(), true)?;

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
            .ok_or_else(|| crate::error::MongoLiteError::CollectionNotFound(name.to_string()))?;

        let catalog = meta.document_catalog.clone();
        let persisted_indexes = meta.indexes.clone();
        let persisted_fuzzy_indexes = meta.fuzzy_indexes.clone();

        log_debug!(
            "Collection '{}' - catalog size: {}, persisted indexes: {}, fuzzy indexes: {}",
            name,
            catalog.len(),
            persisted_indexes.len(),
            persisted_fuzzy_indexes.len()
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
    /// Uses shared IndexManager to fix stale index problem.
    pub fn collection(&self, name: &str) -> Result<CollectionCore<S>> {
        let shared_indexes = self.get_or_create_index_manager(name)?;
        CollectionCore::with_shared_indexes(
            name.to_string(),
            Arc::clone(&self.storage),
            shared_indexes,
        )
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
                    return Err(MongoLiteError::OperationNotAllowed(format!(
                        "Cannot drop protected collection '{}'",
                        name
                    )));
                }
            }
        }

        // Remove shared IndexManager first
        self.index_managers.write().remove(name);

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
            .ok_or_else(|| MongoLiteError::CollectionNotFound(name.to_string()))?;
        meta.flags = flags;
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
            hidden: false, // System collections visible by default (for debugging)
        };
        self.set_collection_flags(name, flags)
    }

    /// Flush all changes to disk
    pub fn flush(&self) -> Result<()>
    where
        DatabaseCore<S>: BatchFlush,
    {
        // Ensure any pending batch operations are flushed before metadata sync
        self.flush_pending_batch()?;

        let mut storage = self.storage.write();
        storage.flush()
    }

    /// Checkpoint - Clear WAL without flushing metadata
    /// Use this in long-running processes to prevent WAL file growth
    pub fn checkpoint(&self) -> Result<()> {
        let mut storage = self.storage.write();
        storage.checkpoint()
    }

    /// Get database path
    pub fn path(&self) -> &str {
        &self.db_path
    }

    // ========== ACD Transaction API ==========

    /// Begin a new transaction
    /// Returns the transaction ID
    pub fn begin_transaction(&self) -> TransactionId {
        let tx_id = self.next_tx_id.fetch_add(1, Ordering::SeqCst);
        let transaction = Transaction::new(tx_id);

        let mut active = self.active_transactions.write();
        active.insert(tx_id, transaction);

        tx_id
    }

    /// Get a reference to an active transaction (for adding operations)
    pub fn get_transaction(&self, tx_id: TransactionId) -> Option<Transaction> {
        let active = self.active_transactions.read();
        active.get(&tx_id).cloned()
    }

    /// Update a transaction (after adding operations)
    pub fn update_transaction(&self, tx_id: TransactionId, transaction: Transaction) -> Result<()> {
        let mut active = self.active_transactions.write();
        active.insert(tx_id, transaction);
        Ok(())
    }

    /// Execute a closure with mutable access to a transaction
    /// This is more efficient than get + modify + update
    pub fn with_transaction<F, R>(&self, tx_id: TransactionId, f: F) -> Result<R>
    where
        F: FnOnce(&mut Transaction) -> Result<R>,
    {
        let mut active = self.active_transactions.write();
        let transaction = active.get_mut(&tx_id).ok_or_else(|| {
            crate::error::MongoLiteError::TransactionAborted(format!(
                "Transaction {} not found",
                tx_id
            ))
        })?;

        f(transaction)
    }

    // ========== Transaction Convenience Methods ==========

    /// Insert one document within a transaction (convenience method)
    ///
    /// This is a helper that combines collection lookup and transaction execution.
    /// Equivalent to: db.collection(name).insert_one_tx(doc, tx)
    pub fn insert_one_tx(
        &self,
        collection_name: &str,
        document: HashMap<String, Value>,
        tx_id: TransactionId,
    ) -> Result<DocumentId> {
        let collection = self.collection(collection_name)?;

        self.with_transaction(tx_id, |transaction| {
            collection.insert_one_tx(document, transaction)
        })
    }

    /// Update one document within a transaction (convenience method)
    ///
    /// Returns (matched_count, modified_count)
    pub fn update_one_tx(
        &self,
        collection_name: &str,
        query: &Value,
        update: Value,
        tx_id: TransactionId,
    ) -> Result<(u64, u64)> {
        let collection = self.collection(collection_name)?;

        self.with_transaction(tx_id, |transaction| {
            collection.update_one_tx(query, update, transaction)
        })
    }

    /// Delete one document within a transaction (convenience method)
    ///
    /// Returns deleted_count
    pub fn delete_one_tx(
        &self,
        collection_name: &str,
        query: &Value,
        tx_id: TransactionId,
    ) -> Result<u64> {
        let collection = self.collection(collection_name)?;

        self.with_transaction(tx_id, |transaction| {
            collection.delete_one_tx(query, transaction)
        })
    }

    /// Get current durability mode
    pub fn durability_mode(&self) -> DurabilityMode {
        self.durability_mode
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        collection.create_index("age".to_string(), false).unwrap();

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
                    key: crate::transaction::IndexKey::Int(30),
                    doc_id: DocumentId::Int(1),
                },
            )?;

            Ok(())
        })
        .unwrap();

        // Commit with indexes
        let result = db.commit_transaction_with_indexes(tx_id);
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
        let result = db.commit_transaction_with_indexes(tx_id);
        assert!(result.is_ok());
    }

    #[test]
    fn test_commit_with_indexes_nonexistent_transaction() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");
        let db = DatabaseCore::open(&db_path).unwrap();

        // Try to commit non-existent transaction
        let result = db.commit_transaction_with_indexes(999);
        assert!(result.is_err());

        // Should be TransactionAborted error
        match result {
            Err(crate::error::MongoLiteError::TransactionAborted(_)) => {}
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
        let index_name = coll.create_index("age".to_string(), false).unwrap();
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
    fn test_system_collection_visible_by_default() {
        let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();

        // Create system collection (hidden = false by default)
        db.create_system_collection("_system.scripts").unwrap();

        // System collections are visible in list_collections by default
        let visible = db.list_collections();
        assert!(visible.contains(&"_system.scripts".to_string()));
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
}
