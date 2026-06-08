// ironbase-core/src/transaction_integration_tests.rs
// Integration tests for ACD transactions

#[cfg(test)]
mod integration_tests {
    use crate::database::DatabaseCore;
    use crate::document::DocumentId;
    use crate::transaction::Operation;
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
            doc: std::sync::Arc::new(json!({"name": "Alice", "email": "alice@example.com"})),
        })
        .unwrap();

        tx.add_operation(Operation::Insert {
            collection: "posts".to_string(),
            doc_id: DocumentId::Int(1),
            doc: std::sync::Arc::new(json!({"user_id": 1, "title": "First Post"})),
        })
        .unwrap();

        tx.add_operation(Operation::Insert {
            collection: "comments".to_string(),
            doc_id: DocumentId::Int(1),
            doc: std::sync::Arc::new(json!({"post_id": 1, "text": "Nice post!"})),
        })
        .unwrap();

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
                doc: std::sync::Arc::new(json!({"id": i, "value": format!("item_{}", i)})),
            })
            .unwrap();
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
                doc: std::sync::Arc::new(json!({"id": i, "data": i * 2})),
            })
            .unwrap();
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
            doc: std::sync::Arc::new(json!({"name": "Item 1"})),
        })
        .unwrap();

        tx.add_operation(Operation::Update {
            collection: "mixed".to_string(),
            doc_id: DocumentId::Int(1),
            old_doc: std::sync::Arc::new(json!({"name": "Item 1"})),
            new_doc: std::sync::Arc::new(json!({"name": "Updated Item 1"})),
        })
        .unwrap();

        tx.add_operation(Operation::Insert {
            collection: "mixed".to_string(),
            doc_id: DocumentId::Int(2),
            doc: std::sync::Arc::new(json!({"name": "Item 2"})),
        })
        .unwrap();

        tx.add_operation(Operation::Delete {
            collection: "mixed".to_string(),
            doc_id: DocumentId::Int(2),
            old_doc: std::sync::Arc::new(json!({"name": "Item 2"})),
        })
        .unwrap();

        assert_eq!(tx.operation_count(), 4);

        db.update_transaction(tx_id, tx).unwrap();

        let result = db.commit_transaction(tx_id);
        assert!(result.is_ok());
    }

    #[test]
    fn test_concurrent_readers_during_transaction() {
        use std::sync::Arc;

        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");

        // Create shared database instance (correct pattern with file locking)
        let db = Arc::new(DatabaseCore::open(&db_path).unwrap());
        db.collection("concurrent_test").unwrap();

        let db_reader = Arc::clone(&db);

        // Spawn reader thread - uses SAME database instance
        let reader_handle = thread::spawn(move || {
            // Readers can access the shared database during active transactions
            let collections = db_reader.list_collections();
            assert!(collections.contains(&"concurrent_test".to_string()));
        });

        // Main thread: Start transaction but don't commit yet
        {
            let tx_id = db.begin_transaction();
            let mut tx = db.get_transaction(tx_id).unwrap();

            tx.add_operation(Operation::Insert {
                collection: "concurrent_test".to_string(),
                doc_id: DocumentId::Int(1),
                doc: std::sync::Arc::new(json!({"data": "test"})),
            })
            .unwrap();

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
        tx1_obj
            .add_operation(Operation::Insert {
                collection: "isolation_test".to_string(),
                doc_id: DocumentId::Int(1),
                doc: std::sync::Arc::new(json!({"value": 100})),
            })
            .unwrap();
        db.update_transaction(tx1, tx1_obj).unwrap();
        db.commit_transaction(tx1).unwrap();

        // Transaction 2: Update
        let tx2 = db.begin_transaction();
        let mut tx2_obj = db.get_transaction(tx2).unwrap();
        tx2_obj
            .add_operation(Operation::Update {
                collection: "isolation_test".to_string(),
                doc_id: DocumentId::Int(1),
                old_doc: std::sync::Arc::new(json!({"value": 100})),
                new_doc: std::sync::Arc::new(json!({"value": 200})),
            })
            .unwrap();
        db.update_transaction(tx2, tx2_obj).unwrap();
        db.commit_transaction(tx2).unwrap();

        // Transaction 3: Delete
        let tx3 = db.begin_transaction();
        let mut tx3_obj = db.get_transaction(tx3).unwrap();
        tx3_obj
            .add_operation(Operation::Delete {
                collection: "isolation_test".to_string(),
                doc_id: DocumentId::Int(1),
                old_doc: std::sync::Arc::new(json!({"value": 200})),
            })
            .unwrap();
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
                doc: std::sync::Arc::new(json!({"collection": i})),
            })
            .unwrap();
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
                doc: std::sync::Arc::new(json!({"id": i})),
            })
            .unwrap();
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
            tx1_obj
                .add_operation(Operation::Insert {
                    collection: "recovery_test".to_string(),
                    doc_id: DocumentId::Int(1),
                    doc: std::sync::Arc::new(json!({"tx": 1})),
                })
                .unwrap();
            db.update_transaction(tx1, tx1_obj).unwrap();
            db.commit_transaction(tx1).unwrap();

            // TX 2
            let tx2 = db.begin_transaction();
            let mut tx2_obj = db.get_transaction(tx2).unwrap();
            tx2_obj
                .add_operation(Operation::Insert {
                    collection: "recovery_test".to_string(),
                    doc_id: DocumentId::Int(2),
                    doc: std::sync::Arc::new(json!({"tx": 2})),
                })
                .unwrap();
            db.update_transaction(tx2, tx2_obj).unwrap();
            db.commit_transaction(tx2).unwrap();

            // TX 3
            let tx3 = db.begin_transaction();
            let mut tx3_obj = db.get_transaction(tx3).unwrap();
            tx3_obj
                .add_operation(Operation::Insert {
                    collection: "recovery_test".to_string(),
                    doc_id: DocumentId::Int(3),
                    doc: std::sync::Arc::new(json!({"tx": 3})),
                })
                .unwrap();
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

    #[test]
    fn test_file_lock_prevents_double_open() {
        use crate::error::IronBaseError;

        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");

        // First open succeeds
        let _db1 = DatabaseCore::open(&db_path).unwrap();

        // Second open on SAME file should fail with DatabaseLocked
        let result = DatabaseCore::open(&db_path);

        match result {
            Err(IronBaseError::DatabaseLocked(path)) => {
                assert!(path.contains("test.mlite"));
            }
            Ok(_) => panic!("Expected DatabaseLocked error, got Ok"),
            Err(e) => panic!("Expected DatabaseLocked error, got: {:?}", e),
        }
    }

    #[test]
    fn test_file_lock_released_on_drop() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");

        // Open and close database
        {
            let db = DatabaseCore::open(&db_path).unwrap();
            db.collection("test").unwrap();
        } // db is dropped here, lock should be released

        // Now we should be able to open it again
        let db2 = DatabaseCore::open(&db_path).unwrap();
        let collections = db2.list_collections();
        assert!(collections.contains(&"test".to_string()));
    }

    #[test]
    fn test_file_lock_released_on_close() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");

        // Open database and create a collection
        let db1 = DatabaseCore::open(&db_path).unwrap();
        db1.collection("test").unwrap();

        // Explicitly close - should release lock
        db1.close().unwrap();

        // Now we should be able to open it again (db1 still exists but lock is released)
        let db2 = DatabaseCore::open(&db_path).unwrap();
        let collections = db2.list_collections();
        assert!(collections.contains(&"test".to_string()));
    }

    #[test]
    fn test_dead_pid_in_lockfile_does_not_bypass_live_holder() {
        // Audit 2026-06-08 P0-1 (empirically reproduced): a `.lock` file recording
        // a PID that looks DEAD must NOT let a second opener bypass a LIVE holder.
        // The old PID-based stale-detection unlinked the live holder's lock inode
        // and re-locked a fresh one → two live writers on one .mlite → corruption.
        // The fix trusts the OS flock (auto-released only on holder death), so the
        // PID written in the file is irrelevant to exclusivity.
        use crate::error::IronBaseError;

        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");
        let lock_path = temp_dir.path().join("test.mlite.lock");

        // db1 holds the exclusive fs2 lock.
        let _db1 = DatabaseCore::open(&db_path).unwrap();

        // Overwrite the .lock content with a PID guaranteed not to be alive
        // (> any pid_max → kill(pid,0) → ESRCH). The flock held by db1 is unaffected.
        std::fs::write(&lock_path, "2147483646\n").unwrap();

        // A second open MUST still fail with DatabaseLocked — db1 holds the flock,
        // regardless of the (dead-looking) PID written in the file.
        match DatabaseCore::open(&db_path) {
            Err(IronBaseError::DatabaseLocked(_)) => {}
            Ok(_) => panic!("dead PID in .lock bypassed the live flock: second open succeeded"),
            Err(e) => panic!("expected DatabaseLocked, got: {:?}", e),
        }
    }

    #[test]
    fn test_stale_lockfile_from_crashed_holder_allows_reopen() {
        // The other direction of P0-1: a crashed process leaves the `.lock` FILE
        // (with its now-dead PID) but the OS released its flock on death. A fresh
        // open MUST succeed — the file's existence is not a lock, only the flock
        // is. This pins the foundational assumption (flock auto-release) the fix
        // rests on, so a future lock-scheme change that doesn't auto-release on
        // crash would fail here instead of silently locking the DB out forever.
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");
        let lock_path = temp_dir.path().join("test.mlite.lock");

        // Leftover stale lock file, NO flock held (simulates post-crash state).
        std::fs::write(&lock_path, "2147483646\n").unwrap();

        // Nothing holds the flock → open must succeed and overwrite the stale PID.
        let _db = DatabaseCore::open(&db_path)
            .expect("reopen after crash (stale .lock file, no flock) must succeed");
    }

    #[test]
    fn test_distinct_db_files_do_not_share_a_lock() {
        // P0-1 path fix: `foo` and `foo.mlite` are DIFFERENT databases and must
        // not block each other. The old `with_extension("mlite.lock")` mapped BOTH
        // to `foo.mlite.lock`; the fix appends `.lock` → `foo.lock` vs
        // `foo.mlite.lock`. Both must open concurrently.
        let temp_dir = TempDir::new().unwrap();
        let p1 = temp_dir.path().join("foo");
        let p2 = temp_dir.path().join("foo.mlite");

        let _db1 = DatabaseCore::open(&p1).unwrap();
        let _db2 = DatabaseCore::open(&p2)
            .expect("distinct db files (foo vs foo.mlite) must not collide on one lock");
    }
}
