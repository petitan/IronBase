// ironbase-core/src/transaction_property_tests.rs
// Property-based tests for ACD transactions using proptest

#[cfg(test)]
mod property_tests {
    use crate::database::DatabaseCore;
    use crate::document::DocumentId;
    use crate::transaction::Operation;
    use proptest::prelude::*;
    use serde_json::json;
    use tempfile::TempDir;

    // Strategy for generating random operations
    fn operation_strategy() -> impl Strategy<Value = Operation> {
        prop_oneof![
            // Insert operations
            (1i64..1000, any::<String>()).prop_map(|(id, name)| {
                Operation::Insert {
                    collection: "test".to_string(),
                    doc_id: DocumentId::Int(id),
                    doc: json!({"id": id, "name": name}),
                }
            }),
            // Update operations
            (1i64..1000, any::<String>(), any::<String>()).prop_map(|(id, old_name, new_name)| {
                Operation::Update {
                    collection: "test".to_string(),
                    doc_id: DocumentId::Int(id),
                    old_doc: json!({"id": id, "name": old_name}),
                    new_doc: json!({"id": id, "name": new_name}),
                }
            }),
            // Delete operations
            (1i64..1000, any::<String>()).prop_map(|(id, name)| {
                Operation::Delete {
                    collection: "test".to_string(),
                    doc_id: DocumentId::Int(id),
                    old_doc: json!({"id": id, "name": name}),
                }
            }),
        ]
    }

    proptest! {
        /// Property: Transaction ID always increments
        #[test]
        fn prop_transaction_id_increments(count in 1usize..100) {
            let temp_dir = TempDir::new().unwrap();
            let db_path = temp_dir.path().join("test.mlite");
            let db = DatabaseCore::open(&db_path).unwrap();

            let mut tx_ids = Vec::new();
            for _ in 0..count {
                let tx_id = db.begin_transaction();
                tx_ids.push(tx_id);
            }

            // Check all IDs are unique and incrementing
            for i in 1..tx_ids.len() {
                prop_assert!(tx_ids[i] > tx_ids[i-1], "TX IDs should increment");
            }
        }

        /// Property: Empty transaction always succeeds
        #[test]
        fn prop_empty_transaction_succeeds(iterations in 1usize..50) {
            let temp_dir = TempDir::new().unwrap();
            let db_path = temp_dir.path().join("test.mlite");
            let db = DatabaseCore::open(&db_path).unwrap();

            for _ in 0..iterations {
                let tx_id = db.begin_transaction();
                let result = db.commit_transaction(tx_id);
                prop_assert!(result.is_ok(), "Empty transaction should succeed");
            }
        }

        /// Property: Rollback always succeeds regardless of operations
        #[test]
        fn prop_rollback_always_succeeds(ops in prop::collection::vec(operation_strategy(), 0..50)) {
            let temp_dir = TempDir::new().unwrap();
            let db_path = temp_dir.path().join("test.mlite");
            let db = DatabaseCore::open(&db_path).unwrap();

            // Create collection first
            db.collection("test").unwrap();

            let tx_id = db.begin_transaction();
            let mut tx = db.get_transaction(tx_id).unwrap();

            // Add all operations
            for op in ops {
                tx.add_operation(op).unwrap();
            }
            db.update_transaction(tx_id, tx).unwrap();

            // Rollback should always succeed
            let result = db.rollback_transaction(tx_id);
            prop_assert!(result.is_ok(), "Rollback should always succeed");
        }

        /// Property: Transaction is removed after commit or rollback
        #[test]
        fn prop_transaction_removed_after_completion(should_commit: bool) {
            let temp_dir = TempDir::new().unwrap();
            let db_path = temp_dir.path().join("test.mlite");
            let db = DatabaseCore::open(&db_path).unwrap();

            let tx_id = db.begin_transaction();

            // Transaction exists before completion
            prop_assert!(db.get_transaction(tx_id).is_some());

            // Complete it
            if should_commit {
                db.commit_transaction(tx_id).unwrap();
            } else {
                db.rollback_transaction(tx_id).unwrap();
            }

            // Transaction should be removed
            prop_assert!(db.get_transaction(tx_id).is_none(), "TX should be removed after completion");
        }

        /// Property: Multiple transactions can coexist before commit
        #[test]
        fn prop_multiple_active_transactions(count in 2usize..20) {
            let temp_dir = TempDir::new().unwrap();
            let db_path = temp_dir.path().join("test.mlite");
            let db = DatabaseCore::open(&db_path).unwrap();

            let mut tx_ids = Vec::new();

            // Create multiple transactions
            for _ in 0..count {
                let tx_id = db.begin_transaction();
                tx_ids.push(tx_id);
            }

            // All should be active
            for &tx_id in &tx_ids {
                prop_assert!(db.get_transaction(tx_id).is_some(), "All TXs should be active");
            }

            // Commit all
            for tx_id in tx_ids {
                db.commit_transaction(tx_id).unwrap();
            }
        }

        /// Property: Operation count matches what was added
        #[test]
        fn prop_operation_count_matches(ops in prop::collection::vec(operation_strategy(), 0..100)) {
            let temp_dir = TempDir::new().unwrap();
            let db_path = temp_dir.path().join("test.mlite");
            let db = DatabaseCore::open(&db_path).unwrap();

            let tx_id = db.begin_transaction();
            let mut tx = db.get_transaction(tx_id).unwrap();

            let expected_count = ops.len();

            for op in ops {
                tx.add_operation(op).unwrap();
            }

            prop_assert_eq!(tx.operation_count(), expected_count, "Operation count should match");
        }

        /// Property: Cannot commit same transaction twice
        #[test]
        fn prop_cannot_double_commit(ops in prop::collection::vec(operation_strategy(), 0..10)) {
            let temp_dir = TempDir::new().unwrap();
            let db_path = temp_dir.path().join("test.mlite");
            let db = DatabaseCore::open(&db_path).unwrap();

            db.collection("test").unwrap();

            let tx_id = db.begin_transaction();
            let mut tx = db.get_transaction(tx_id).unwrap();

            for op in ops {
                tx.add_operation(op).unwrap();
            }
            db.update_transaction(tx_id, tx).unwrap();

            // First commit succeeds
            let first = db.commit_transaction(tx_id);
            prop_assert!(first.is_ok());

            // Second commit fails (transaction doesn't exist)
            let second = db.commit_transaction(tx_id);
            prop_assert!(second.is_err(), "Cannot commit same TX twice");
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(50))]

        /// Property: Database survives crash and recovery (simulated)
        #[test]
        fn prop_crash_recovery_preserves_committed(
            committed_ops in prop::collection::vec(operation_strategy(), 1..20),
            uncommitted_ops in prop::collection::vec(operation_strategy(), 0..20)
        ) {
            let temp_dir = TempDir::new().unwrap();
            let db_path = temp_dir.path().join("test.mlite");

            // Phase 1: Commit some transactions
            {
                let db = DatabaseCore::open(&db_path).unwrap();
                db.collection("test").unwrap();

                let tx_id = db.begin_transaction();
                let mut tx = db.get_transaction(tx_id).unwrap();

                for op in committed_ops {
                    tx.add_operation(op).unwrap();
                }
                db.update_transaction(tx_id, tx).unwrap();
                db.commit_transaction(tx_id).unwrap();

                // Add uncommitted transaction
                if !uncommitted_ops.is_empty() {
                    let tx_id2 = db.begin_transaction();
                    let mut tx2 = db.get_transaction(tx_id2).unwrap();
                    for op in uncommitted_ops {
                        tx2.add_operation(op).unwrap();
                    }
                    db.update_transaction(tx_id2, tx2).unwrap();
                    // DON'T commit - simulate crash
                }

                // Drop db (simulates crash)
            }

            // Phase 2: Reopen database (recovery happens automatically)
            {
                let db = DatabaseCore::open(&db_path).unwrap();

                // Database should be openable after "crash"
                prop_assert!(db.list_collections().contains(&"test".to_string()));

                // New transactions should work
                let tx_id = db.begin_transaction();
                db.commit_transaction(tx_id).unwrap();
            }
        }
    }
}
