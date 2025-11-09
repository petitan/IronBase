// mongolite-core/src/database.rs
// Pure Rust database API - NO PyO3 dependencies

use std::sync::Arc;
use parking_lot::RwLock;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::storage::StorageEngine;
use crate::collection_core::CollectionCore;
use crate::error::Result;
use crate::transaction::{Transaction, TransactionId};

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
        let storage = StorageEngine::open(&path_str)?;

        Ok(DatabaseCore {
            storage: Arc::new(RwLock::new(storage)),
            db_path: path_str,
            next_tx_id: AtomicU64::new(1),
            active_transactions: Arc::new(RwLock::new(std::collections::HashMap::new())),
        })
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
}
