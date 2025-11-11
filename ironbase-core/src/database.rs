// ironbase-core/src/database.rs
// Pure Rust database API - NO PyO3 dependencies

use std::sync::Arc;
use parking_lot::RwLock;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::storage::StorageEngine;
use crate::collection_core::CollectionCore;
use crate::error::Result;
use crate::transaction::{Transaction, TransactionId};

/// Convert transaction::IndexKey to index::IndexKey
fn convert_index_key(tx_key: &crate::transaction::IndexKey) -> crate::index::IndexKey {
    match tx_key {
        crate::transaction::IndexKey::Int(i) => crate::index::IndexKey::Int(*i),
        crate::transaction::IndexKey::String(s) => crate::index::IndexKey::String(s.clone()),
        crate::transaction::IndexKey::Float(f) => crate::index::IndexKey::Float(crate::index::OrderedFloat(f.value())),
        crate::transaction::IndexKey::Bool(b) => crate::index::IndexKey::Bool(*b),
        crate::transaction::IndexKey::Null => crate::index::IndexKey::Null,
    }
}

/// Pure Rust MongoLite Database - language-independent
pub struct DatabaseCore {
    storage: Arc<RwLock<StorageEngine>>,
    db_path: String,
    next_tx_id: AtomicU64,
    active_transactions: Arc<RwLock<std::collections::HashMap<TransactionId, Transaction>>>,
}

impl DatabaseCore {
    /// Open or create database
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path_str = path.as_ref().to_string_lossy().to_string();
        let mut storage = StorageEngine::open(&path_str)?;

        // Recover from WAL (includes both data and index changes)
        let (_wal_entries, recovered_index_changes) = storage.recover_from_wal()?;

        // Create DatabaseCore instance
        let db = DatabaseCore {
            storage: Arc::new(RwLock::new(storage)),
            db_path: path_str,
            next_tx_id: AtomicU64::new(1),
            active_transactions: Arc::new(RwLock::new(std::collections::HashMap::new())),
        };

        // Apply recovered index changes to collections
        // Group index changes by collection name
        use std::collections::HashMap;
        let mut changes_by_collection: HashMap<String, Vec<crate::storage::RecoveredIndexChange>> = HashMap::new();

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

    /// Get collection (creates if doesn't exist)
    pub fn collection(&self, name: &str) -> Result<CollectionCore> {
        CollectionCore::new(name.to_string(), Arc::clone(&self.storage))
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
    pub fn flush(&self) -> Result<()> {
        let mut storage = self.storage.write();
        storage.flush()
    }

    /// Get database statistics as JSON
    pub fn stats(&self) -> serde_json::Value {
        let storage = self.storage.read();
        storage.stats()
    }

    /// Storage compaction - removes tombstones and old document versions
    pub fn compact(&self) -> Result<crate::storage::CompactionStats> {
        let mut storage = self.storage.write();
        storage.compact()
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

    /// Commit a transaction (applies all buffered operations atomically)
    pub fn commit_transaction(&self, tx_id: TransactionId) -> Result<()> {
        // Remove transaction from active list
        let mut transaction = {
            let mut active = self.active_transactions.write();
            active.remove(&tx_id)
                .ok_or_else(|| crate::error::MongoLiteError::TransactionAborted(
                    format!("Transaction {} not found", tx_id)
                ))?
        };

        // Commit through storage engine
        let mut storage = self.storage.write();
        storage.commit_transaction(&mut transaction)?;

        Ok(())
    }

    /// Rollback a transaction (discard all buffered operations)
    pub fn rollback_transaction(&self, tx_id: TransactionId) -> Result<()> {
        // Remove transaction from active list
        let mut transaction = {
            let mut active = self.active_transactions.write();
            active.remove(&tx_id)
                .ok_or_else(|| crate::error::MongoLiteError::TransactionAborted(
                    format!("Transaction {} not found", tx_id)
                ))?
        };

        // Rollback through storage engine
        let mut storage = self.storage.write();
        storage.rollback_transaction(&mut transaction)?;

        Ok(())
    }

    /// Commit transaction with atomic index updates (two-phase commit)
    ///
    /// # Two-Phase Commit Protocol
    /// 1. PREPARE: Apply index changes to in-memory IndexManager
    /// 2. PREPARE: Create temp index files (.idx.tmp) via prepare_changes()
    /// 3. COMMIT: Delegate to StorageEngine::commit_transaction() (WAL + data)
    /// 4. FINALIZE: Atomic rename .idx.tmp → .idx via commit_prepared_changes()
    ///
    /// # Crash Recovery
    /// - If crash before COMMIT: WAL rollback cleans up temp files
    /// - If crash after COMMIT: WAL recovery replays index changes from WAL
    ///
    /// # Arguments
    /// * `tx_id` - Transaction ID to commit
    ///
    /// # Returns
    /// * `Ok(())` on successful commit
    /// * `Err(MongoLiteError)` if commit fails (transaction rolled back)
    pub fn commit_transaction_with_indexes(&self, tx_id: TransactionId) -> Result<()> {
        use std::collections::HashMap;
        use std::path::PathBuf;

        // ========== PHASE 0: EXTRACT TRANSACTION ==========

        // 1. Extract transaction from active list
        let mut transaction = {
            let mut active = self.active_transactions.write();
            active.remove(&tx_id)
                .ok_or_else(|| crate::error::MongoLiteError::TransactionAborted(
                    format!("Transaction {} not found", tx_id)
                ))?
        };

        // 2. If transaction has no index changes, delegate to simple commit
        if transaction.index_changes().is_empty() {
            let mut storage = self.storage.write();
            return storage.commit_transaction(&mut transaction);
        }

        // 3. Extract collection name from first operation
        let collection_name = Self::get_collection_from_transaction(&transaction)
            .ok_or_else(|| crate::error::MongoLiteError::TransactionAborted(
                format!("Transaction {} has no operations", tx_id)
            ))?;

        // ========== PHASE 1: PREPARE INDEXES ==========

        // Track all temp files for atomic rename
        let mut prepared_indexes: Vec<(PathBuf, PathBuf)> = Vec::new();

        // Get collection (creates if doesn't exist)
        let collection = self.collection(&collection_name)?;

        // Group index changes by index name
        let mut changes_by_index: HashMap<String, Vec<crate::transaction::IndexChange>> = HashMap::new();
        for (index_name, changes) in transaction.index_changes() {
            changes_by_index.insert(index_name.clone(), changes.clone());
        }

        // Apply changes to in-memory indexes and prepare temp files
        for (index_name, changes) in changes_by_index {
            let mut indexes = collection.indexes.write();

            if let Some(index) = indexes.get_btree_index_mut(&index_name) {
                // Apply all changes to in-memory index
                for change in &changes {
                    let result = match change.operation {
                        crate::transaction::IndexOperation::Insert => {
                            let key = convert_index_key(&change.key);
                            index.insert(key, change.doc_id.clone())
                        }
                        crate::transaction::IndexOperation::Delete => {
                            let key = convert_index_key(&change.key);
                            index.delete(&key, &change.doc_id)
                        }
                    };

                    // If index modification fails, cleanup temp files and restore transaction
                    if let Err(e) = result {
                        // Cleanup all prepared temp files
                        for (temp_path, _) in &prepared_indexes {
                            let _ = crate::index::BPlusTree::rollback_prepared_changes(temp_path);
                        }

                        // Re-insert transaction into active list for potential rollback
                        let mut active = self.active_transactions.write();
                        active.insert(tx_id, transaction);

                        return Err(e);
                    }
                }

                // Prepare temp file with updated index
                let base_path = self.get_index_file_path(&collection_name, &index_name);
                match index.prepare_changes(&base_path) {
                    Ok(temp_path) => {
                        prepared_indexes.push((temp_path, base_path));
                    }
                    Err(e) => {
                        // Cleanup all prepared temp files
                        for (temp_path, _) in &prepared_indexes {
                            let _ = crate::index::BPlusTree::rollback_prepared_changes(temp_path);
                        }

                        // Re-insert transaction into active list for potential rollback
                        let mut active = self.active_transactions.write();
                        active.insert(tx_id, transaction);

                        return Err(e);
                    }
                }
            }

            // Release indexes write lock before next iteration
            drop(indexes);
        }

        // ========== PHASE 2: COMMIT DATA + WAL ==========

        // Delegate to existing StorageEngine commit
        // This handles:
        // - Writing WAL entries (Operations + IndexChanges)
        // - Fsync WAL
        // - Applying operations to data
        // - Fsync data
        // - Marking transaction committed
        let commit_result = {
            let mut storage = self.storage.write();
            storage.commit_transaction(&mut transaction)
        };

        // If commit fails, cleanup temp files (transaction not committed)
        if let Err(e) = commit_result {
            for (temp_path, _) in &prepared_indexes {
                let _ = crate::index::BPlusTree::rollback_prepared_changes(temp_path);
            }
            return Err(e);
        }

        // ========== PHASE 3: FINALIZE INDEXES ==========

        // Atomic rename all temp files to final paths
        // NOTE: If finalize fails, transaction is already committed (durable in WAL)
        // Temp files will be cleaned up on next startup, indexes rebuilt from WAL
        for (temp_path, final_path) in prepared_indexes {
            if let Err(e) = crate::index::BPlusTree::commit_prepared_changes(&temp_path, &final_path) {
                // Log error but DON'T fail transaction (already committed)
                eprintln!("WARN: Index finalize failed for {:?}: {:?}", final_path, e);
                eprintln!("WARN: Index will be rebuilt from WAL on next open()");
                // Continue with next index
            }
        }

        Ok(())
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
        let transaction = active.get_mut(&tx_id)
            .ok_or_else(|| crate::error::MongoLiteError::TransactionAborted(
                format!("Transaction {} not found", tx_id)
            ))?;

        f(transaction)
    }

    // ========== Two-Phase Commit Helper Methods ==========

    /// Construct index file path for a collection's index
    /// Format: {db_path_without_ext}.{index_name}.idx
    ///
    /// Example: "/data/myapp.mlite" + "users_age" → "/data/myapp.users_age.idx"
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
    fn get_collection_from_transaction(transaction: &Transaction) -> Option<String> {
        transaction.operations()
            .first()
            .map(|op| match op {
                crate::transaction::Operation::Insert { collection, .. } => collection.clone(),
                crate::transaction::Operation::Update { collection, .. } => collection.clone(),
                crate::transaction::Operation::Delete { collection, .. } => collection.clone(),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use crate::transaction::Operation;
    use serde_json::json;
    use crate::document::DocumentId;

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
        }).unwrap();
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
        }).unwrap();
        tx.add_operation(Operation::Insert {
            collection: "users".to_string(),
            doc_id: DocumentId::Int(2),
            doc: json!({"name": "Bob", "age": 25}),
        }).unwrap();
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
                }
            )?;

            Ok(())
        }).unwrap();

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
        }).unwrap();

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
            Err(crate::error::MongoLiteError::TransactionAborted(_)) => {},
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
        transaction.add_operation(Operation::Insert {
            collection: "users".to_string(),
            doc_id: DocumentId::Int(1),
            doc: json!({"name": "Alice"}),
        }).unwrap();

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
}
