// Long key index tests - verify B+ tree handles long string keys properly
// Tests for issue: "Node size exceeds page size" with long keys like email message_id

use ironbase_core::index::NODE_PAGE_SIZE;
use ironbase_core::DatabaseCore;
use serde_json::json;
use tempfile::TempDir;

#[test]
fn test_page_size_is_16kb() {
    // Verify the page size constant was updated
    assert_eq!(
        NODE_PAGE_SIZE, 16384,
        "NODE_PAGE_SIZE should be 16KB for long key support"
    );
}

#[test]
fn test_index_with_long_string_keys() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    let db = DatabaseCore::open(&db_path).unwrap();
    let collection = db.collection("emails").unwrap();

    // Create index on message_id field (typically long email message IDs)
    collection
        .create_index("message_id".to_string(), false, false)
        .unwrap();

    // Insert documents with long message_id values (500+ bytes)
    for i in 0..10 {
        let long_message_id = format!("<{}.{}@example.com>", "x".repeat(400), i);
        assert!(
            long_message_id.len() > 400,
            "Message ID should be over 400 chars"
        );

        let mut fields = std::collections::HashMap::new();
        fields.insert("message_id".to_string(), json!(long_message_id));
        fields.insert("subject".to_string(), json!(format!("Test email {}", i)));

        db.insert_one("emails", fields).unwrap();
    }

    // Verify all documents were inserted
    let all_docs = collection.find(&json!({})).unwrap();
    assert_eq!(all_docs.len(), 10);
}

#[test]
fn test_many_long_keys_triggers_split() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    let db = DatabaseCore::open(&db_path).unwrap();
    let collection = db.collection("documents").unwrap();

    // Create index on a field that will have 200-byte keys
    collection
        .create_index("long_field".to_string(), false, false)
        .unwrap();

    // Insert 100 documents with 200-byte keys - this should trigger multiple splits
    for i in 0..100 {
        let long_value = format!("{:0>200}", i); // 200 chars, padded with zeros
        assert_eq!(long_value.len(), 200);

        let mut fields = std::collections::HashMap::new();
        fields.insert("long_field".to_string(), json!(long_value));
        fields.insert("seq".to_string(), json!(i));

        db.insert_one("documents", fields).unwrap();
    }

    // Verify all documents were inserted
    let all_docs = collection.find(&json!({})).unwrap();
    assert_eq!(all_docs.len(), 100);

    // Verify index-based query works
    let search_key = format!("{:0>200}", 50);
    let found = collection.find(&json!({"long_field": search_key})).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0]["seq"], 50);
}

#[test]
fn test_very_long_keys_1kb() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    let db = DatabaseCore::open(&db_path).unwrap();
    let collection = db.collection("big_keys").unwrap();

    // Create index on a field with 1KB keys
    collection
        .create_index("huge_key".to_string(), false, false)
        .unwrap();

    // Insert 20 documents with ~1KB keys - this should trigger size-based early split
    for i in 0..20 {
        let huge_value = format!("{:0>1000}", i); // 1000 chars
        assert_eq!(huge_value.len(), 1000);

        let mut fields = std::collections::HashMap::new();
        fields.insert("huge_key".to_string(), json!(huge_value));
        fields.insert("id".to_string(), json!(i));

        db.insert_one("big_keys", fields).unwrap();
    }

    // Verify all documents were inserted successfully
    let all_docs = collection.find(&json!({})).unwrap();
    assert_eq!(all_docs.len(), 20);
}

#[test]
fn test_unique_index_with_long_keys() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    let db = DatabaseCore::open(&db_path).unwrap();
    let collection = db.collection("unique_emails").unwrap();

    // Create UNIQUE index on message_id
    collection
        .create_index("message_id".to_string(), true, false)
        .unwrap();

    // Insert documents with unique long message_ids
    for i in 0..5 {
        let long_message_id = format!("<unique.{}.{}@test.example.com>", "y".repeat(300), i);

        let mut fields = std::collections::HashMap::new();
        fields.insert("message_id".to_string(), json!(long_message_id));

        db.insert_one("unique_emails", fields).unwrap();
    }

    // Verify unique constraint works - try to insert duplicate
    let duplicate_id = format!("<unique.{}.{}@test.example.com>", "y".repeat(300), 0);
    let mut dup_fields = std::collections::HashMap::new();
    dup_fields.insert("message_id".to_string(), json!(duplicate_id));

    let result = db.insert_one("unique_emails", dup_fields);
    assert!(result.is_err(), "Duplicate key should be rejected");
}

#[test]
fn test_index_persistence_with_long_keys() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // First session: create index and insert data
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let collection = db.collection("persistent").unwrap();

        collection
            .create_index("long_key".to_string(), false, false)
            .unwrap();

        for i in 0..10 {
            let long_value = format!("{:0>500}", i);
            let mut fields = std::collections::HashMap::new();
            fields.insert("long_key".to_string(), json!(long_value));
            fields.insert("val".to_string(), json!(i));
            db.insert_one("persistent", fields).unwrap();
        }
    }

    // Second session: verify data persisted and index works
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let collection = db.collection("persistent").unwrap();

        let all_docs = collection.find(&json!({})).unwrap();
        assert_eq!(all_docs.len(), 10);

        // Verify index is used for lookup
        let search_key = format!("{:0>500}", 5);
        let found = collection.find(&json!({"long_key": search_key})).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0]["val"], 5);
    }
}
