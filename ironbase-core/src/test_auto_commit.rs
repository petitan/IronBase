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

        let doc_id = db.insert_one_safe("users", doc).unwrap();
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
            db.insert_one_safe("test", doc).unwrap();
        }

        // Verify all documents were written
        let collection = db.collection("test").unwrap();
        let count = collection.count_documents(&json!({})).unwrap();
        assert_eq!(count, 5);

        // Manual flush to commit remaining batch
        db.flush_batch().unwrap();

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

        // Open database in Unsafe mode
        let db =
            DatabaseCore::<StorageEngine>::open_with_durability(db_path, DurabilityMode::Unsafe)
                .unwrap();

        assert_eq!(db.durability_mode(), DurabilityMode::Unsafe);

        // Insert document (fast path, no WAL)
        let doc = HashMap::from([("name".to_string(), json!("Bob"))]);

        db.insert_one_safe("users", doc).unwrap();

        // Verify the document was written
        let collection = db.collection("users").unwrap();
        let count = collection.count_documents(&json!({})).unwrap();
        assert_eq!(count, 1);

        // Cleanup
        std::fs::remove_file(db_path).unwrap();
        let _ = std::fs::remove_file(wal_path);
    }
}
