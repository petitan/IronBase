// ironbase-core/src/transaction_integration_tests.rs
// Integration tests for ACD transactions

#[cfg(test)]
mod integration_tests {
    use crate::database::DatabaseCore;
    use crate::transaction::Operation;
    use crate::document::DocumentId;
    use serde_json::json;
    use tempfile::TempDir;
    
    use std::thread;

    #[test]
    fn test_multi_collection_transaction() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");
        let db = DatabaseCore::open(&db_path).unwrap();

        // Create multiple collections
        db.collection("users").unwrap();
        db.collection("posts").unwrap();
        db.collection("comments").unwrap();

        // Begin transaction spanning multiple collections
        let tx_id = db.begin_transaction();
        let mut tx = db.get_transaction(tx_id).unwrap();

        // Add operations for different collections
        tx.add_operation(Operation::Insert {
            collection: "users".to_string(),
            doc_id: DocumentId::Int(1),
            doc: json!({"name": "Alice", "email": "alice@example.com"}),
        }).unwrap();

        tx.add_operation(Operation::Insert {
            collection: "posts".to_string(),
            doc_id: DocumentId::Int(1),
            doc: json!({"user_id": 1, "title": "First Post"}),
        }).unwrap();

        tx.add_operation(Operation::Insert {
            collection: "comments".to_string(),
            doc_id: DocumentId::Int(1),
            doc: json!({"post_id": 1, "text": "Nice post!"}),
        }).unwrap();

        db.update_transaction(tx_id, tx).unwrap();

        // Commit should apply all changes atomically
        let result = db.commit_transaction(tx_id);
        assert!(result.is_ok());

        // Verify all collections exist
        let collections = db.list_collections();
        assert!(collections.contains(&"users".to_string()));
        assert!(collections.contains(&"posts".to_string()));
        assert!(collections.contains(&"comments".to_string()));
    }

    #[test]
    fn test_large_transaction_1000_operations() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");
        let db = DatabaseCore::open(&db_path).unwrap();

        db.collection("large_test").unwrap();

        let tx_id = db.begin_transaction();
        let mut tx = db.get_transaction(tx_id).unwrap();

        // Add 1000 insert operations
        for i in 0..1000 {
            tx.add_operation(Operation::Insert {
                collection: "large_test".to_string(),
                doc_id: DocumentId::Int(i),
                doc: json!({"id": i, "value": format!("item_{}", i)}),
            }).unwrap();
        }

        assert_eq!(tx.operation_count(), 1000);

        db.update_transaction(tx_id, tx).unwrap();

        // Commit should handle 1000 operations
        let result = db.commit_transaction(tx_id);
        assert!(result.is_ok(), "Large transaction should succeed");
    }

    #[test]
    fn test_very_large_transaction_10000_operations() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");
        let db = DatabaseCore::open(&db_path).unwrap();

        db.collection("very_large_test").unwrap();

        let tx_id = db.begin_transaction();
        let mut tx = db.get_transaction(tx_id).unwrap();

        // Add 10,000 insert operations
        for i in 0..10000 {
            tx.add_operation(Operation::Insert {
                collection: "very_large_test".to_string(),
                doc_id: DocumentId::Int(i),
                doc: json!({"id": i, "data": i * 2}),
            }).unwrap();
        }

        assert_eq!(tx.operation_count(), 10000);

        db.update_transaction(tx_id, tx).unwrap();

        // This should succeed but might be slow
        let result = db.commit_transaction(tx_id);
        assert!(result.is_ok(), "Very large transaction should succeed");
    }

    #[test]
    fn test_mixed_operations_transaction() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");
        let db = DatabaseCore::open(&db_path).unwrap();

        db.collection("mixed").unwrap();

        let tx_id = db.begin_transaction();
        let mut tx = db.get_transaction(tx_id).unwrap();

        // Mix of Insert, Update, Delete operations
        tx.add_operation(Operation::Insert {
            collection: "mixed".to_string(),
            doc_id: DocumentId::Int(1),
            doc: json!({"name": "Item 1"}),
        }).unwrap();

        tx.add_operation(Operation::Update {
            collection: "mixed".to_string(),
            doc_id: DocumentId::Int(1),
            old_doc: json!({"name": "Item 1"}),
            new_doc: json!({"name": "Updated Item 1"}),
        }).unwrap();

        tx.add_operation(Operation::Insert {
            collection: "mixed".to_string(),
            doc_id: DocumentId::Int(2),
            doc: json!({"name": "Item 2"}),
        }).unwrap();

        tx.add_operation(Operation::Delete {
            collection: "mixed".to_string(),
            doc_id: DocumentId::Int(2),
            old_doc: json!({"name": "Item 2"}),
        }).unwrap();

        assert_eq!(tx.operation_count(), 4);

        db.update_transaction(tx_id, tx).unwrap();

        let result = db.commit_transaction(tx_id);
        assert!(result.is_ok());
    }

    #[test]
    fn test_concurrent_readers_during_transaction() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");

        // Setup: Create database and collection
        {
            let db = DatabaseCore::open(&db_path).unwrap();
            db.collection("concurrent_test").unwrap();
        }

        let db_path_clone = db_path.clone();

        // Spawn reader thread
        let reader_handle = thread::spawn(move || {
            let db = DatabaseCore::open(&db_path_clone).unwrap();

            // Readers should be able to open database
            // even if there are active (uncommitted) transactions
            let collections = db.list_collections();
            assert!(collections.contains(&"concurrent_test".to_string()));
        });

        // Main thread: Start transaction but don't commit yet
        {
            let db = DatabaseCore::open(&db_path).unwrap();
            let tx_id = db.begin_transaction();
            let mut tx = db.get_transaction(tx_id).unwrap();

            tx.add_operation(Operation::Insert {
                collection: "concurrent_test".to_string(),
                doc_id: DocumentId::Int(1),
                doc: json!({"data": "test"}),
            }).unwrap();

            db.update_transaction(tx_id, tx).unwrap();

            // Don't commit yet - reader should still work
            thread::sleep(std::time::Duration::from_millis(10));

            db.commit_transaction(tx_id).unwrap();
        }

        // Wait for reader
        reader_handle.join().unwrap();
    }

    #[test]
    fn test_sequential_transactions_isolation() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");
        let db = DatabaseCore::open(&db_path).unwrap();

        db.collection("isolation_test").unwrap();

        // Transaction 1: Insert
        let tx1 = db.begin_transaction();
        let mut tx1_obj = db.get_transaction(tx1).unwrap();
        tx1_obj.add_operation(Operation::Insert {
            collection: "isolation_test".to_string(),
            doc_id: DocumentId::Int(1),
            doc: json!({"value": 100}),
        }).unwrap();
        db.update_transaction(tx1, tx1_obj).unwrap();
        db.commit_transaction(tx1).unwrap();

        // Transaction 2: Update
        let tx2 = db.begin_transaction();
        let mut tx2_obj = db.get_transaction(tx2).unwrap();
        tx2_obj.add_operation(Operation::Update {
            collection: "isolation_test".to_string(),
            doc_id: DocumentId::Int(1),
            old_doc: json!({"value": 100}),
            new_doc: json!({"value": 200}),
        }).unwrap();
        db.update_transaction(tx2, tx2_obj).unwrap();
        db.commit_transaction(tx2).unwrap();

        // Transaction 3: Delete
        let tx3 = db.begin_transaction();
        let mut tx3_obj = db.get_transaction(tx3).unwrap();
        tx3_obj.add_operation(Operation::Delete {
            collection: "isolation_test".to_string(),
            doc_id: DocumentId::Int(1),
            old_doc: json!({"value": 200}),
        }).unwrap();
        db.update_transaction(tx3, tx3_obj).unwrap();
        db.commit_transaction(tx3).unwrap();

        // All should succeed sequentially
    }

    #[test]
    fn test_transaction_with_many_collections() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");
        let db = DatabaseCore::open(&db_path).unwrap();

        // Create 50 collections
        for i in 0..50 {
            db.collection(&format!("col_{}", i)).unwrap();
        }

        let tx_id = db.begin_transaction();
        let mut tx = db.get_transaction(tx_id).unwrap();

        // Add operations across all collections
        for i in 0..50 {
            tx.add_operation(Operation::Insert {
                collection: format!("col_{}", i),
                doc_id: DocumentId::Int(1),
                doc: json!({"collection": i}),
            }).unwrap();
        }

        assert_eq!(tx.operation_count(), 50);

        db.update_transaction(tx_id, tx).unwrap();

        let result = db.commit_transaction(tx_id);
        assert!(result.is_ok());

        // Verify all collections exist
        let collections = db.list_collections();
        assert_eq!(collections.len(), 50);
    }

    #[test]
    fn test_rollback_after_many_operations() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");
        let db = DatabaseCore::open(&db_path).unwrap();

        db.collection("rollback_test").unwrap();

        let tx_id = db.begin_transaction();
        let mut tx = db.get_transaction(tx_id).unwrap();

        // Add 500 operations
        for i in 0..500 {
            tx.add_operation(Operation::Insert {
                collection: "rollback_test".to_string(),
                doc_id: DocumentId::Int(i),
                doc: json!({"id": i}),
            }).unwrap();
        }

        assert_eq!(tx.operation_count(), 500);

        db.update_transaction(tx_id, tx.clone()).unwrap();

        // Rollback instead of commit
        let result = db.rollback_transaction(tx_id);
        assert!(result.is_ok());

        // Transaction should be gone
        assert!(db.get_transaction(tx_id).is_none());
    }

    #[test]
    fn test_crash_recovery_with_multiple_transactions() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");

        // Phase 1: Create and commit 3 transactions
        {
            let db = DatabaseCore::open(&db_path).unwrap();
            db.collection("recovery_test").unwrap();

            // TX 1
            let tx1 = db.begin_transaction();
            let mut tx1_obj = db.get_transaction(tx1).unwrap();
            tx1_obj.add_operation(Operation::Insert {
                collection: "recovery_test".to_string(),
                doc_id: DocumentId::Int(1),
                doc: json!({"tx": 1}),
            }).unwrap();
            db.update_transaction(tx1, tx1_obj).unwrap();
            db.commit_transaction(tx1).unwrap();

            // TX 2
            let tx2 = db.begin_transaction();
            let mut tx2_obj = db.get_transaction(tx2).unwrap();
            tx2_obj.add_operation(Operation::Insert {
                collection: "recovery_test".to_string(),
                doc_id: DocumentId::Int(2),
                doc: json!({"tx": 2}),
            }).unwrap();
            db.update_transaction(tx2, tx2_obj).unwrap();
            db.commit_transaction(tx2).unwrap();

            // TX 3
            let tx3 = db.begin_transaction();
            let mut tx3_obj = db.get_transaction(tx3).unwrap();
            tx3_obj.add_operation(Operation::Insert {
                collection: "recovery_test".to_string(),
                doc_id: DocumentId::Int(3),
                doc: json!({"tx": 3}),
            }).unwrap();
            db.update_transaction(tx3, tx3_obj).unwrap();
            db.commit_transaction(tx3).unwrap();

            // Simulate crash (drop db without cleanup)
        }

        // Phase 2: Reopen and verify recovery
        {
            let db = DatabaseCore::open(&db_path).unwrap();

            // Database should recover successfully
            let collections = db.list_collections();
            assert!(collections.contains(&"recovery_test".to_string()));

            // New transactions should work after recovery
            let tx = db.begin_transaction();
            db.commit_transaction(tx).unwrap();
        }
    }
}
