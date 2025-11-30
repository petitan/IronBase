// Storage compaction tests using public DatabaseCore API
use ironbase_core::{DatabaseCore, StorageEngine};
use serde_json::json;
use std::collections::HashMap;
use tempfile::TempDir;

#[test]
fn test_compaction_removes_tombstones() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("compact_test.mlite");

    let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();

    // Insert 10 documents
    for i in 0..10 {
        let mut doc = HashMap::new();
        doc.insert("id".to_string(), json!(i));
        doc.insert("name".to_string(), json!(format!("User{}", i)));
        db.insert_one("users", doc).unwrap();
    }

    // Delete half (creates tombstones)
    for i in 0..5i64 {
        db.delete_one("users", &json!({"id": i})).unwrap();
    }

    // Compact
    let stats = db.compact().unwrap();

    // Verify stats
    assert_eq!(stats.tombstones_removed, 5);
    assert!(stats.space_saved() > 0);
    assert!(stats.size_after < stats.size_before);
}

#[test]
fn test_compaction_preserves_live_documents() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("compact_preserve.mlite");

    let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();

    // Insert documents
    for i in 0..20 {
        let mut doc = HashMap::new();
        doc.insert("value".to_string(), json!(i * 100));
        db.insert_one("items", doc).unwrap();
    }

    // Compact
    let stats = db.compact().unwrap();

    // All documents should be kept (no tombstones)
    assert_eq!(stats.documents_kept, 20);
    assert_eq!(stats.tombstones_removed, 0);

    // Verify all documents still exist via query
    let coll = db.collection("items").unwrap();
    let docs = coll.find(&json!({})).unwrap();
    assert_eq!(docs.len(), 20);
}

#[test]
fn test_compaction_multi_collection() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("compact_multi.mlite");

    let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();

    // Insert to users collection
    for i in 0..10 {
        let mut doc = HashMap::new();
        doc.insert("name".to_string(), json!(format!("User{}", i)));
        db.insert_one("users", doc).unwrap();
    }

    // Delete some from users (tombstones)
    for i in 0..3i64 {
        db.delete_one("users", &json!({"name": format!("User{}", i)}))
            .unwrap();
    }

    // Insert to posts collection
    for i in 0..10 {
        let mut doc = HashMap::new();
        doc.insert("title".to_string(), json!(format!("Post{}", i)));
        db.insert_one("posts", doc).unwrap();
    }

    // Compact
    let stats = db.compact().unwrap();

    // Should have removed tombstones
    assert_eq!(stats.tombstones_removed, 3);
    // Should keep: 7 users + 10 posts = 17
    assert_eq!(stats.documents_kept, 17);
}

#[test]
fn test_compaction_handles_updates() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("compact_updates.mlite");

    let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();

    // Insert document
    let mut doc = HashMap::new();
    doc.insert("_id".to_string(), json!(1));
    doc.insert("value".to_string(), json!(100));
    db.insert_one("data", doc).unwrap();

    // Update it 5 times (creates old versions)
    for i in 2..=6 {
        db.update_one(
            "data",
            &json!({"_id": 1}),
            &json!({"$set": {"value": i * 100}}),
        )
        .unwrap();
    }

    // Compact - should keep only latest version
    let stats = db.compact().unwrap();

    assert_eq!(stats.documents_kept, 1); // Only latest version
    assert!(stats.size_after < stats.size_before); // Size reduced

    // Verify latest value is preserved
    let coll = db.collection("data").unwrap();
    let doc = coll.find_one(&json!({"_id": 1})).unwrap().unwrap();
    assert_eq!(doc["value"], json!(600)); // Latest value
}

#[test]
fn test_compaction_stats() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("compact_stats.mlite");

    let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();

    // Insert 100 documents
    for i in 0..100 {
        let mut doc = HashMap::new();
        doc.insert("id".to_string(), json!(i));
        doc.insert("data".to_string(), json!(vec![0u8; 100]));
        db.insert_one("test", doc).unwrap();
    }

    // Mark 50 as tombstones (delete)
    for i in 0..50i64 {
        db.delete_one("test", &json!({"id": i})).unwrap();
    }

    // Compact
    let stats = db.compact().unwrap();

    // Verify stats
    assert!(stats.size_before > 0);
    assert!(stats.size_after > 0);
    assert!(stats.size_after < stats.size_before);
    assert_eq!(stats.tombstones_removed, 50);
    assert_eq!(stats.documents_kept, 50);
    assert!(stats.space_saved() > 0);
    assert!(stats.compression_ratio() > 0.0);
    assert!(stats.compression_ratio() < 100.0);
}

#[test]
fn test_compaction_persistence() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("compact_persist.mlite");

    {
        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();

        // Insert documents
        for i in 0..10 {
            let mut doc = HashMap::new();
            doc.insert("id".to_string(), json!(i));
            db.insert_one("items", doc).unwrap();
        }

        // Mark half as deleted
        for i in 0..5i64 {
            db.delete_one("items", &json!({"id": i})).unwrap();
        }

        db.compact().unwrap();
        db.flush().unwrap();
    }

    // Reopen and verify compacted state persisted
    {
        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        let coll = db.collection("items").unwrap();

        // Should only have 5 documents (tombstones removed)
        let docs = coll.find(&json!({})).unwrap();
        assert_eq!(docs.len(), 5);
    }
}
