//! Auto-commit mode tests
//!
//! Tests that verify durability guarantees for different modes.

#[cfg(test)]
mod tests {
    use crate::storage::StorageEngine;
    use crate::{DatabaseCore, DurabilityMode};
    use serde_json::json;
    use std::collections::HashMap;

    #[test]
    fn test_safe_mode_default() {
        // Test that Safe mode is the default
        let db_path = "test_safe_default.mlite";
        let wal_path = "test_safe_default.wal";

        // Cleanup
        let _ = std::fs::remove_file(db_path);
        let _ = std::fs::remove_file(wal_path);

        let db = DatabaseCore::<StorageEngine>::open(db_path).unwrap();

        assert_eq!(db.durability_mode(), DurabilityMode::Safe);

        // Cleanup
        std::fs::remove_file(db_path).unwrap();
        let _ = std::fs::remove_file(wal_path);
    }

    #[test]
    fn test_insert_one_safe_mode() {
        let db_path = "test_insert_safe.mlite";
        let wal_path = "test_insert_safe.wal";

        // Cleanup
        let _ = std::fs::remove_file(db_path);
        let _ = std::fs::remove_file(wal_path);

        // Open database in Safe mode (default)
        let db = DatabaseCore::<StorageEngine>::open(db_path).unwrap();

        // Insert one document using safe insert
        let doc = HashMap::from([
            ("name".to_string(), json!("Alice")),
            ("age".to_string(), json!(30)),
        ]);

        let doc_id = db.insert_one("users", doc).unwrap();
        println!("Inserted document with ID: {:?}", doc_id);

        // Verify the document was written
        let collection = db.collection("users").unwrap();
        let count = collection.count_documents(&json!({})).unwrap();
        assert_eq!(count, 1);

        // Cleanup
        std::fs::remove_file(db_path).unwrap();
        let _ = std::fs::remove_file(wal_path);
    }

    #[test]
    fn test_batch_mode() {
        let db_path = "test_insert_batch.mlite";
        let wal_path = "test_insert_batch.wal";

        // Cleanup
        let _ = std::fs::remove_file(db_path);
        let _ = std::fs::remove_file(wal_path);

        // Open database in Batch mode
        let db = DatabaseCore::<StorageEngine>::open_with_durability(
            db_path,
            DurabilityMode::Batch { batch_size: 3 },
        )
        .unwrap();

        assert_eq!(
            db.durability_mode(),
            DurabilityMode::Batch { batch_size: 3 }
        );

        // Insert 5 documents (should trigger 1 flush at 3, leaving 2 in buffer)
        for i in 0..5 {
            let doc = HashMap::from([("value".to_string(), json!(i))]);
            db.insert_one("test", doc).unwrap();
        }

        // WAL-FIRST DESIGN: Before flush, only the first batch (3 docs) is persisted.
        // The remaining 2 are buffered and not yet visible in queries.
        let collection = db.collection("test").unwrap();
        let count_before_flush = collection.count_documents(&json!({})).unwrap();
        assert_eq!(
            count_before_flush, 3,
            "Only first batch should be persisted before flush"
        );

        // Manual flush to commit remaining batch
        db.flush_batch().unwrap();

        // After flush, all 5 documents should be visible
        let count_after_flush = collection.count_documents(&json!({})).unwrap();
        assert_eq!(
            count_after_flush, 5,
            "All documents should be visible after flush"
        );

        // Cleanup
        std::fs::remove_file(db_path).unwrap();
        let _ = std::fs::remove_file(wal_path);
    }

    #[test]
    fn test_unsafe_mode() {
        let db_path = "test_insert_unsafe.mlite";
        let wal_path = "test_insert_unsafe.wal";

        // Cleanup
        let _ = std::fs::remove_file(db_path);
        let _ = std::fs::remove_file(wal_path);

        // Open database in Unsafe mode (manual checkpoint)
        let db = DatabaseCore::<StorageEngine>::open_with_durability(
            db_path,
            DurabilityMode::unsafe_manual(),
        )
        .unwrap();

        assert_eq!(db.durability_mode(), DurabilityMode::unsafe_manual());

        // Insert document (fast path, no WAL)
        let doc = HashMap::from([("name".to_string(), json!("Bob"))]);

        db.insert_one("users", doc).unwrap();

        // Verify the document was written
        let collection = db.collection("users").unwrap();
        let count = collection.count_documents(&json!({})).unwrap();
        assert_eq!(count, 1);

        // Cleanup
        std::fs::remove_file(db_path).unwrap();
        let _ = std::fs::remove_file(wal_path);
    }
}
