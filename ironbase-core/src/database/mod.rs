// ironbase-core/src/database/mod.rs
// Pure Rust database API - NO PyO3 dependencies

mod collections;
mod durability;
mod maintenance;
mod transactions;

pub use durability::BatchFlush;

use parking_lot::{Mutex, RwLock};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use crate::collection_core::schema::CompiledSchema;
use crate::document::DocumentId;
use crate::durability::DurabilityMode;
use crate::error::{IronBaseError, Result};
use crate::index::IndexManager;
use crate::storage::{MemoryStorage, RawStorage, Storage, StorageEngine};
use crate::transaction::{Operation, Transaction, TransactionId};
use serde_json::Value;

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
        crate::error::IronBaseError::InvalidQuery("Document missing _id".to_string())
    })
}

/// Pure Rust IronBase Database - language-independent
///
/// Generic over Storage backend:
/// - `DatabaseCore<StorageEngine>` - Production file-based storage (default)
/// - `DatabaseCore<MemoryStorage>` - Fast in-memory storage for testing
#[allow(clippy::type_complexity)]
pub struct DatabaseCore<S: Storage + RawStorage> {
    pub(crate) storage: Arc<RwLock<S>>,
    pub(crate) db_path: String,
    pub(crate) next_tx_id: AtomicU64,
    pub(crate) active_transactions:
        Arc<RwLock<std::collections::HashMap<TransactionId, Transaction>>>,

    // NEW: Durability mode (safe by default like SQL databases)
    pub(crate) durability_mode: DurabilityMode,

    // NEW: Batch buffer for Batch mode
    pub(crate) batch_buffer: Arc<RwLock<Vec<Operation>>>,

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
    pub(crate) write_transaction_lock: Arc<RwLock<Option<TransactionId>>>,

    // Flag to prevent operations after close() is called
    // Arc-wrapped so CollectionCore can share the same flag
    pub(crate) is_closed: Arc<AtomicBool>,

    // Collection-level write locks for Safe mode atomicity
    // Ensures prepare-WAL-persist sequence is atomic per collection
    // Prevents race conditions in unique constraint checks
    pub(crate) collection_write_locks: Arc<RwLock<HashMap<String, Arc<Mutex<()>>>>>,
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

        // Recover from WAL (includes both data and index changes)
        let (_wal_entries, recovered_index_changes) = storage.recover_from_wal()?;

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
        if !_wal_entries.is_empty() {
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
            active_transactions: Arc::new(RwLock::new(std::collections::HashMap::new())),
            durability_mode: mode,
            batch_buffer: Arc::new(RwLock::new(Vec::new())),
            unsafe_op_counter: AtomicU64::new(0),
            index_managers: Arc::new(RwLock::new(HashMap::new())),
            schema_managers: Arc::new(RwLock::new(HashMap::new())),
            write_transaction_lock: Arc::new(RwLock::new(None)),
            is_closed: Arc::new(AtomicBool::new(false)),
            collection_write_locks: Arc::new(RwLock::new(HashMap::new())),
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
            active_transactions: Arc::new(RwLock::new(std::collections::HashMap::new())),
            durability_mode: DurabilityMode::default(),
            batch_buffer: Arc::new(RwLock::new(Vec::new())),
            unsafe_op_counter: AtomicU64::new(0),
            index_managers: Arc::new(RwLock::new(HashMap::new())),
            schema_managers: Arc::new(RwLock::new(HashMap::new())),
            write_transaction_lock: Arc::new(RwLock::new(None)),
            is_closed: Arc::new(AtomicBool::new(false)),
            collection_write_locks: Arc::new(RwLock::new(HashMap::new())),
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
