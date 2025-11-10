// Integration tests for MongoLite Core
use ironbase_core::{StorageEngine, Document, DocumentId};
use std::collections::HashMap;
use serde_json::json;
use tempfile::TempDir;

// Helper to create test storage
fn create_test_storage() -> (TempDir, StorageEngine) {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let storage = StorageEngine::open(&db_path).unwrap();
    (temp_dir, storage)
}

// Helper to create test document
fn create_doc(id: i64, name: &str, age: i64) -> Document {
    let mut fields = HashMap::new();
    fields.insert("name".to_string(), json!(name));
    fields.insert("age".to_string(), json!(age));
    Document::new(DocumentId::Int(id), fields)
}

#[test]
fn test_insert_and_read_document() {
    let (_temp, mut storage) = create_test_storage();
    storage.create_collection("users").unwrap();

    // Insert document
    let doc = create_doc(1, "Alice", 30);
    let doc_json = doc.to_json().unwrap();
    let offset = storage.write_data(doc_json.as_bytes()).unwrap();

    // Read back
    let data = storage.read_data(offset).unwrap();
    let json_str = String::from_utf8(data).unwrap();
    let restored = Document::from_json(&json_str).unwrap();

    assert_eq!(restored.id, DocumentId::Int(1));
    assert_eq!(restored.get("name").unwrap(), &json!("Alice"));
    assert_eq!(restored.get("age").unwrap(), &json!(30));
}

#[test]
fn test_multiple_documents_in_collection() {
    let (_temp, mut storage) = create_test_storage();
    storage.create_collection("users").unwrap();

    // Insert multiple documents
    let docs = vec![
        create_doc(1, "Alice", 30),
        create_doc(2, "Bob", 25),
        create_doc(3, "Carol", 35),
    ];

    let mut offsets = Vec::new();
    for doc in docs {
        let doc_json = doc.to_json().unwrap();
        let offset = storage.write_data(doc_json.as_bytes()).unwrap();
        offsets.push(offset);
    }

    // Read all back
    assert_eq!(offsets.len(), 3);

    for (i, offset) in offsets.iter().enumerate() {
        let data = storage.read_data(*offset).unwrap();
        let json_str = String::from_utf8(data).unwrap();
        let doc = Document::from_json(&json_str).unwrap();
        assert_eq!(doc.id, DocumentId::Int((i + 1) as i64));
    }
}

#[test]
fn test_collection_isolation() {
    let (_temp, mut storage) = create_test_storage();
    storage.create_collection("users").unwrap();
    storage.create_collection("posts").unwrap();

    // Get metadata
    let users_meta = storage.get_collection_meta("users").unwrap();
    let posts_meta = storage.get_collection_meta("posts").unwrap();

    // Both should have different metadata but same data_offset (since both empty)
    assert_eq!(users_meta.name, "users");
    assert_eq!(posts_meta.name, "posts");
    assert_eq!(users_meta.document_count, 0);
    assert_eq!(posts_meta.document_count, 0);
}

#[test]
fn test_document_count_tracking() {
    let (_temp, mut storage) = create_test_storage();
    storage.create_collection("users").unwrap();

    // Update document count manually (simulating collection operations)
    {
        let meta = storage.get_collection_meta_mut("users").unwrap();
        meta.document_count = 5;
    }

    // Verify
    let meta = storage.get_collection_meta("users").unwrap();
    assert_eq!(meta.document_count, 5);
}

#[test]
fn test_last_id_increment() {
    let (_temp, mut storage) = create_test_storage();
    storage.create_collection("users").unwrap();

    // Simulate ID generation
    let id1 = {
        let meta = storage.get_collection_meta_mut("users").unwrap();
        let id = DocumentId::new_auto(meta.last_id);
        meta.last_id += 1;
        id
    };

    let id2 = {
        let meta = storage.get_collection_meta_mut("users").unwrap();
        let id = DocumentId::new_auto(meta.last_id);
        meta.last_id += 1;
        id
    };

    assert_eq!(id1, DocumentId::Int(1));
    assert_eq!(id2, DocumentId::Int(2));
}

#[test]
fn test_persistence_across_reopens() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // First session - create and write
    {
        let mut storage = StorageEngine::open(&db_path).unwrap();
        storage.create_collection("users").unwrap();

        let doc = create_doc(1, "Alice", 30);
        let doc_json = doc.to_json().unwrap();
        storage.write_data(doc_json.as_bytes()).unwrap();

        // Update metadata
        {
            let meta = storage.get_collection_meta_mut("users").unwrap();
            meta.document_count = 1;
            meta.last_id = 1;
        }

        storage.flush().unwrap();
    }

    // Second session - reopen and verify
    {
        let storage = StorageEngine::open(&db_path).unwrap();
        let meta = storage.get_collection_meta("users").unwrap();

        assert_eq!(meta.name, "users");
        assert_eq!(meta.document_count, 1);
        assert_eq!(meta.last_id, 1);
    }
}

#[test]
fn test_document_with_collection_field() {
    let (_temp, mut storage) = create_test_storage();
    storage.create_collection("users").unwrap();

    // Create document with _collection field (for isolation)
    let mut doc = create_doc(1, "Alice", 30);
    doc.set("_collection".to_string(), json!("users"));

    let doc_json = doc.to_json().unwrap();
    let offset = storage.write_data(doc_json.as_bytes()).unwrap();

    // Read back and verify _collection field
    let data = storage.read_data(offset).unwrap();
    let json_str = String::from_utf8(data).unwrap();
    let restored = Document::from_json(&json_str).unwrap();

    assert_eq!(restored.get("_collection").unwrap(), &json!("users"));
}

#[test]
fn test_tombstone_pattern() {
    let (_temp, mut storage) = create_test_storage();
    storage.create_collection("users").unwrap();

    // Create document
    let doc = create_doc(1, "Alice", 30);
    let doc_json = doc.to_json().unwrap();
    storage.write_data(doc_json.as_bytes()).unwrap();

    // Create tombstone version
    let mut tombstone = create_doc(1, "Alice", 30);
    tombstone.set("_tombstone".to_string(), json!(true));

    let tombstone_json = tombstone.to_json().unwrap();
    let tombstone_offset = storage.write_data(tombstone_json.as_bytes()).unwrap();

    // Read tombstone
    let data = storage.read_data(tombstone_offset).unwrap();
    let json_str = String::from_utf8(data).unwrap();
    let restored = Document::from_json(&json_str).unwrap();

    assert_eq!(restored.get("_tombstone").unwrap(), &json!(true));
}

#[test]
fn test_update_pattern() {
    let (_temp, mut storage) = create_test_storage();
    storage.create_collection("users").unwrap();

    // Original document
    let doc = create_doc(1, "Alice", 30);
    let doc_json = doc.to_json().unwrap();
    storage.write_data(doc_json.as_bytes()).unwrap();

    // Updated version
    let mut updated = create_doc(1, "Alice", 31); // Age changed
    updated.set("updated".to_string(), json!(true));

    let updated_json = updated.to_json().unwrap();
    let updated_offset = storage.write_data(updated_json.as_bytes()).unwrap();

    // Read updated version
    let data = storage.read_data(updated_offset).unwrap();
    let json_str = String::from_utf8(data).unwrap();
    let restored = Document::from_json(&json_str).unwrap();

    assert_eq!(restored.get("age").unwrap(), &json!(31));
    assert_eq!(restored.get("updated").unwrap(), &json!(true));
}

#[test]
fn test_large_number_of_documents() {
    let (_temp, mut storage) = create_test_storage();
    storage.create_collection("users").unwrap();

    // Insert 100 documents
    let mut offsets = Vec::new();
    for i in 0..100 {
        let doc = create_doc(i, &format!("User{}", i), 20 + (i % 50));
        let doc_json = doc.to_json().unwrap();
        let offset = storage.write_data(doc_json.as_bytes()).unwrap();
        offsets.push(offset);
    }

    assert_eq!(offsets.len(), 100);

    // Verify first and last
    let first_data = storage.read_data(offsets[0]).unwrap();
    let first_doc = Document::from_json(&String::from_utf8(first_data).unwrap()).unwrap();
    assert_eq!(first_doc.id, DocumentId::Int(0));

    let last_data = storage.read_data(offsets[99]).unwrap();
    let last_doc = Document::from_json(&String::from_utf8(last_data).unwrap()).unwrap();
    assert_eq!(last_doc.id, DocumentId::Int(99));
}

#[test]
fn test_stats_with_collections() {
    let (_temp, mut storage) = create_test_storage();
    storage.create_collection("users").unwrap();
    storage.create_collection("posts").unwrap();

    let stats = storage.stats();

    assert_eq!(stats["collection_count"], 2);

    let collections = stats["collections"].as_array().unwrap();
    assert_eq!(collections.len(), 2);

    // Check collection names
    let names: Vec<String> = collections.iter()
        .map(|c| c["name"].as_str().unwrap().to_string())
        .collect();

    assert!(names.contains(&"users".to_string()));
    assert!(names.contains(&"posts".to_string()));
}
