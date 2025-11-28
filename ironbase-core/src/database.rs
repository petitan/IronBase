// ironbase-core/src/database.rs
// Pure Rust database API - NO PyO3 dependencies

use parking_lot::RwLock;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::collection_core::CollectionCore;
use crate::document::DocumentId;
use crate::durability::DurabilityMode;
use crate::error::Result;
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

/// Pure Rust IronBase Database - language-independent
///
/// Generic over Storage backend:
/// - `DatabaseCore<StorageEngine>` - Production file-based storage (default)
/// - `DatabaseCore<MemoryStorage>` - Fast in-memory storage for testing
///
/// # Future TODO
/// - FileStorage needs full refactor for better trait compliance
/// - WAL recovery currently StorageEngine-specific
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
}

// ============================================================================
// STORAGEENGINE-SPECIFIC IMPLEMENTATION (WAL recovery)
// ============================================================================

impl DatabaseCore<StorageEngine> {
    /// Open or create database with StorageEngine (production)
    ///
    /// This method is StorageEngine-specific because it handles WAL recovery.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path_str = path.as_ref().to_string_lossy().to_string();
        let mut storage = StorageEngine::open(&path_str)?;

        // Recover from WAL (includes both data and index changes)
        let (_wal_entries, recovered_index_changes) = storage.recover_from_wal()?;

        // Rebuild document catalog from file after WAL recovery
        // This ensures all documents are properly indexed in memory
        storage.rebuild_catalog_from_file()?;

        // Create DatabaseCore instance with default Safe mode
        let db = DatabaseCore {
            storage: Arc::new(RwLock::new(storage)),
            db_path: path_str,
            next_tx_id: AtomicU64::new(1),
            active_transactions: Arc::new(RwLock::new(std::collections::HashMap::new())),
            durability_mode: DurabilityMode::default(), // Safe mode by default
            batch_buffer: Arc::new(RwLock::new(Vec::new())),
            unsafe_op_counter: AtomicU64::new(0),
        };

        // Apply recovered index changes to collections
        // Group index changes by collection name
        let mut changes_by_collection: HashMap<String, Vec<crate::storage::RecoveredIndexChange>> =
            HashMap::new();

        for change in recovered_index_changes {
            // Group by collection name (now properly included in RecoveredIndexChange)
            changes_by_collection
                .entry(change.collection.clone())
                .or_insert_with(Vec::new)
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

        // Rebuild document catalog from file after WAL recovery
        // This ensures all documents are properly indexed in memory
        storage.rebuild_catalog_from_file()?;

        // Create DatabaseCore instance with specified mode
        let db = DatabaseCore {
            storage: Arc::new(RwLock::new(storage)),
            db_path: path_str,
            next_tx_id: AtomicU64::new(1),
            active_transactions: Arc::new(RwLock::new(std::collections::HashMap::new())),
            durability_mode: mode,
            batch_buffer: Arc::new(RwLock::new(Vec::new())),
            unsafe_op_counter: AtomicU64::new(0),
        };

        // Apply recovered index changes to collections
        // Group index changes by collection name
        let mut changes_by_collection: HashMap<String, Vec<crate::storage::RecoveredIndexChange>> =
            HashMap::new();

        for change in recovered_index_changes {
            // Group by collection name (now properly included in RecoveredIndexChange)
            changes_by_collection
                .entry(change.collection.clone())
                .or_insert_with(Vec::new)
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
    /// let doc_id = db.insert_one_safe("users", HashMap::from([
    ///     ("name".to_string(), json!("Alice")),
    ///     ("age".to_string(), json!(30)),
    /// ]))?;
    /// # Ok::<(), ironbase_core::MongoLiteError>(())
    /// ```
    pub fn insert_one_safe(
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
                let doc_id = collection.insert_one(document.clone())?;

                // 3. Add operation to transaction
                // IMPORTANT: WAL must contain the FULL document with _id and _collection
                // so that recovery can rebuild the catalog correctly
                let mut doc_with_metadata = document.clone();
                doc_with_metadata.insert("_id".to_string(), serde_json::to_value(&doc_id).unwrap());
                doc_with_metadata.insert("_collection".to_string(), Value::String(collection_name.to_string()));
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
                let doc_id = collection.insert_one(document.clone())?;

                // 2. Add to batch buffer
                // IMPORTANT: WAL must contain the FULL document with _id and _collection
                let mut doc_with_metadata = document.clone();
                doc_with_metadata.insert("_id".to_string(), serde_json::to_value(&doc_id).unwrap());
                doc_with_metadata.insert("_collection".to_string(), Value::String(collection_name.to_string()));
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

            DurabilityMode::Unsafe { auto_checkpoint_ops } => {
                // Unsafe mode: Fast path, optional auto-checkpoint
                let collection = self.collection(collection_name)?;
                let doc_id = collection.insert_one(document)?;

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
    /// let users = db.collection("users")?;
    ///
    /// // Use like normal - all CRUD operations work
    /// users.insert_one(std::collections::HashMap::from([
    ///     ("name".to_string(), serde_json::json!("Alice")),
    /// ]))?;
    ///
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
        })
    }
}

// ============================================================================
// GENERIC IMPLEMENTATION (all storage backends)
// ============================================================================

impl<S: Storage + RawStorage> DatabaseCore<S> {
    /// Get collection (creates if doesn't exist)
    pub fn collection(&self, name: &str) -> Result<CollectionCore<S>> {
        CollectionCore::new(name.to_string(), Arc::clone(&self.storage))
    }

    /// Set or clear JSON schema for a collection
    pub fn set_collection_schema(&self, name: &str, schema: Option<Value>) -> Result<()> {
        let collection = self.collection(name)?;
        collection.set_schema(schema)
    }

    /// List all collection names
    pub fn list_collections(&self) -> Vec<String> {
        let storage = self.storage.read();
        storage.list_collections()
    }

    /// Drop collection
    pub fn drop_collection(&self, name: &str) -> Result<()> {
        let mut storage = self.storage.write();
        storage.drop_collection(name)
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
        let id = coll.insert_one(doc).unwrap();
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
        let id = coll.insert_one(doc).unwrap();

        // Find
        let found = coll.find_one(&json!({"_id": id})).unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap()["name"], "Alice");

        // Update
        coll.update_one(&json!({"_id": id}), &json!({"$set": {"age": 31}}))
            .unwrap();
        let updated = coll.find_one(&json!({"_id": id})).unwrap().unwrap();
        assert_eq!(updated["age"], 31);

        // Delete
        coll.delete_one(&json!({"_id": id})).unwrap();
        let count = coll.count_documents(&json!({})).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_memory_multiple_collections() {
        let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();

        let users = db.collection("users").unwrap();
        let posts = db.collection("posts").unwrap();

        users
            .insert_one(std::collections::HashMap::from([(
                "name".to_string(),
                json!("Alice"),
            )]))
            .unwrap();
        posts
            .insert_one(std::collections::HashMap::from([(
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
            coll.insert_one(std::collections::HashMap::from([
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
            coll.insert_one(std::collections::HashMap::from([(
                "age".to_string(),
                json!(i * 10),
            )]))
            .unwrap();
        }

        // Query using index
        let results = coll.find(&json!({"age": {"$gte": 50}})).unwrap();
        assert_eq!(results.len(), 5);
    }
}
