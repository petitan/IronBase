// Storage compaction tests
use ironbase_core::storage::RawStorage;
use ironbase_core::{Document, DocumentId, StorageEngine};
use serde_json::json;
use std::collections::HashMap;
use tempfile::TempDir;

#[test]
fn test_compaction_removes_tombstones() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("compact_test.mlite");
    let mut storage = StorageEngine::open(&db_path).unwrap();
    storage.create_collection("users").unwrap();

    // Insert 10 documents
    for i in 0..10 {
        let mut fields = HashMap::new();
        fields.insert("id".to_string(), json!(i));
        fields.insert("name".to_string(), json!(format!("User{}", i)));
        let doc = Document::new(DocumentId::Int(i as i64), fields);
        let doc_json = doc.to_json().unwrap();
        storage
            .write_document_raw("users", &DocumentId::Int(i as i64), doc_json.as_bytes())
            .unwrap();
    }

    // Mark half as tombstones (simulate deletes)
    for i in 0..5 {
        let mut fields = HashMap::new();
        fields.insert("id".to_string(), json!(i));
        fields.insert("_tombstone".to_string(), json!(true));
        fields.insert("_collection".to_string(), json!("users"));
        let doc = Document::new(DocumentId::Int(i as i64), fields);
        let doc_json = doc.to_json().unwrap();
        storage
            .write_document_raw("users", &DocumentId::Int(i as i64), doc_json.as_bytes())
            .unwrap();
    }

    storage.flush().unwrap();
    let size_before = storage.file_len().unwrap();

    // Compact
    let stats = storage.compact().unwrap();

    // Verify stats
    assert_eq!(stats.tombstones_removed, 5);
    assert!(stats.space_saved() > 0);
    assert!(stats.size_after < size_before);

    // Verify file size decreased
    let size_after = storage.file_len().unwrap();
    assert!(size_after < size_before);
}

#[test]
fn test_compaction_preserves_live_documents() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("compact_preserve.mlite");
    let mut storage = StorageEngine::open(&db_path).unwrap();
    storage.create_collection("items").unwrap();

    // Insert documents
    let mut expected_ids = vec![];
    for i in 0..20 {
        let mut fields = HashMap::new();
        fields.insert("value".to_string(), json!(i * 100));
        fields.insert("_collection".to_string(), json!("items"));
        let doc = Document::new(DocumentId::Int(i as i64), fields);
        let doc_json = doc.to_json().unwrap();
        storage
            .write_document_raw("items", &DocumentId::Int(i as i64), doc_json.as_bytes())
            .unwrap();
        expected_ids.push(i);
    }

    storage.flush().unwrap();

    // Compact
    let stats = storage.compact().unwrap();

    // All documents should be kept (no tombstones)
    assert_eq!(stats.documents_kept, 20);
    assert_eq!(stats.tombstones_removed, 0);

    // Verify all documents still exist by reading exactly document_count documents
    let meta = storage.get_collection_meta("items").unwrap();
    let mut current_offset = meta.data_offset;
    let mut found_ids = vec![];

    // Read exactly document_count documents from this collection
    for _ in 0..meta.document_count {
        match storage.read_data(current_offset) {
            Ok(doc_bytes) => {
                let doc_str = String::from_utf8(doc_bytes.clone()).unwrap();
                let doc: Document = Document::from_json(&doc_str).unwrap();
                if let DocumentId::Int(id) = doc.id {
                    found_ids.push(id);
                }
                current_offset += 4 + doc_bytes.len() as u64;
            }
            Err(_) => {
                break;
            }
        }
    }

    found_ids.sort();
    assert_eq!(found_ids, expected_ids);
}

#[test]
fn test_compaction_multi_collection() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("compact_multi.mlite");
    let mut storage = StorageEngine::open(&db_path).unwrap();
    storage.create_collection("users").unwrap();
    storage.create_collection("posts").unwrap();

    // Insert documents to both collections
    for i in 0..10 {
        // Users
        let mut fields = HashMap::new();
        fields.insert("name".to_string(), json!(format!("User{}", i)));
        fields.insert("_collection".to_string(), json!("users"));
        let doc = Document::new(DocumentId::Int(i as i64), fields);
        let doc_json = doc.to_json().unwrap();
        storage
            .write_document_raw("users", &DocumentId::Int(i as i64), doc_json.as_bytes())
            .unwrap();

        // Posts
        let mut fields = HashMap::new();
        fields.insert("title".to_string(), json!(format!("Post{}", i)));
        fields.insert("_collection".to_string(), json!("posts"));
        let doc = Document::new(DocumentId::Int(i as i64), fields);
        let doc_json = doc.to_json().unwrap();
        storage
            .write_document_raw("posts", &DocumentId::Int(i as i64), doc_json.as_bytes())
            .unwrap();
    }

    // Delete some from users (tombstones)
    for i in 0..3 {
        let mut fields = HashMap::new();
        fields.insert("_tombstone".to_string(), json!(true));
        fields.insert("_collection".to_string(), json!("users"));
        let doc = Document::new(DocumentId::Int(i as i64), fields);
        let doc_json = doc.to_json().unwrap();
        storage
            .write_document_raw("users", &DocumentId::Int(i as i64), doc_json.as_bytes())
            .unwrap();
    }

    storage.flush().unwrap();

    // Compact
    let stats = storage.compact().unwrap();

    // Should have removed tombstones
    assert_eq!(stats.tombstones_removed, 3);
    // Should keep: 7 users + 10 posts = 17
    assert_eq!(stats.documents_kept, 17);
}

#[test]
fn test_compaction_handles_updates() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("compact_updates.mlite");
    let mut storage = StorageEngine::open(&db_path).unwrap();
    storage.create_collection("data").unwrap();

    // Insert document
    let mut fields = HashMap::new();
    fields.insert("value".to_string(), json!(100));
    fields.insert("_collection".to_string(), json!("data"));
    let doc = Document::new(DocumentId::Int(1), fields);
    let doc_json = doc.to_json().unwrap();
    storage
        .write_document_raw("data", &DocumentId::Int(1), doc_json.as_bytes())
        .unwrap();

    // Update it 5 times (creates old versions)
    for i in 2..=6 {
        let mut fields = HashMap::new();
        fields.insert("value".to_string(), json!(i * 100));
        fields.insert("_collection".to_string(), json!("data"));
        let doc = Document::new(DocumentId::Int(1), fields);
        let doc_json = doc.to_json().unwrap();
        storage
            .write_document_raw("data", &DocumentId::Int(1), doc_json.as_bytes())
            .unwrap();
    }

    storage.flush().unwrap();
    let size_before = storage.file_len().unwrap();

    // Compact - should keep only latest version
    let stats = storage.compact().unwrap();

    assert_eq!(stats.documents_kept, 1); // Only latest version
    assert!(stats.size_after < size_before); // Size reduced

    // Verify latest value is preserved
    let meta = storage.get_collection_meta("data").unwrap();
    let doc_bytes = storage.read_data(meta.data_offset).unwrap();
    let doc_str = String::from_utf8(doc_bytes).unwrap();
    let doc: Document = Document::from_json(&doc_str).unwrap();

    assert_eq!(doc.get("value").unwrap(), &json!(600)); // Latest value
}

#[test]
fn test_compaction_stats() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("compact_stats.mlite");
    let mut storage = StorageEngine::open(&db_path).unwrap();
    storage.create_collection("test").unwrap();

    // Insert 100 documents
    for i in 0..100 {
        let mut fields = HashMap::new();
        fields.insert("data".to_string(), json!(vec![0u8; 100])); // 100 bytes each
        fields.insert("_collection".to_string(), json!("test"));
        let doc = Document::new(DocumentId::Int(i), fields);
        let doc_json = doc.to_json().unwrap();
        storage
            .write_document_raw("test", &DocumentId::Int(i), doc_json.as_bytes())
            .unwrap();
    }

    // Mark 50 as tombstones
    for i in 0..50 {
        let mut fields = HashMap::new();
        fields.insert("_tombstone".to_string(), json!(true));
        fields.insert("_collection".to_string(), json!("test"));
        let doc = Document::new(DocumentId::Int(i), fields);
        let doc_json = doc.to_json().unwrap();
        storage
            .write_document_raw("test", &DocumentId::Int(i), doc_json.as_bytes())
            .unwrap();
    }

    storage.flush().unwrap();

    // Compact
    let stats = storage.compact().unwrap();

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
        let mut storage = StorageEngine::open(&db_path).unwrap();
        storage.create_collection("items").unwrap();

        // Insert and delete
        for i in 0..10 {
            let mut fields = HashMap::new();
            fields.insert("id".to_string(), json!(i));
            fields.insert("_collection".to_string(), json!("items"));
            let doc = Document::new(DocumentId::Int(i), fields);
            let doc_json = doc.to_json().unwrap();
            storage
                .write_document_raw("items", &DocumentId::Int(i), doc_json.as_bytes())
                .unwrap();
        }

        // Mark half as deleted
        for i in 0..5 {
            let mut fields = HashMap::new();
            fields.insert("_tombstone".to_string(), json!(true));
            fields.insert("_collection".to_string(), json!("items"));
            let doc = Document::new(DocumentId::Int(i), fields);
            let doc_json = doc.to_json().unwrap();
            storage
                .write_document_raw("items", &DocumentId::Int(i), doc_json.as_bytes())
                .unwrap();
        }

        storage.compact().unwrap();
        storage.flush().unwrap();
    }

    // Reopen and verify compacted state persisted
    {
        let mut storage = StorageEngine::open(&db_path).unwrap();
        let meta = storage.get_collection_meta("items").unwrap();

        // Should only have 5 documents (tombstones removed)
        // Verify by checking document_count in metadata
        assert_eq!(meta.document_count, 5);

        // Also verify we can read all 5 documents
        let mut current_offset = meta.data_offset;
        let mut count = 0;

        for _ in 0..meta.document_count {
            if let Ok(doc_bytes) = storage.read_data(current_offset) {
                count += 1;
                current_offset += 4 + doc_bytes.len() as u64;
            } else {
                break;
            }
        }

        assert_eq!(count, 5);
    }
}
