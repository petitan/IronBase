// recovery_integration_tests.rs
// Integration tests for WAL recovery, crash scenarios, and data persistence

use ironbase_core::DatabaseCore;
use serde_json::json;
use std::collections::HashMap;
use std::fs;
use tempfile::TempDir;

/// Helper to create a document with an ID
fn doc_with_id(id: i64, name: &str) -> HashMap<String, serde_json::Value> {
    let mut doc = HashMap::new();
    doc.insert("_id".to_string(), json!(id));
    doc.insert("name".to_string(), json!(name));
    doc
}

/// Helper to create a document with additional fields
fn doc_with_fields(
    id: i64,
    name: &str,
    age: i64,
    city: &str,
) -> HashMap<String, serde_json::Value> {
    let mut doc = HashMap::new();
    doc.insert("_id".to_string(), json!(id));
    doc.insert("name".to_string(), json!(name));
    doc.insert("age".to_string(), json!(age));
    doc.insert("city".to_string(), json!(city));
    doc
}

// ============================================================================
// DATABASE LIFECYCLE TESTS
// ============================================================================

#[test]
fn test_database_open_close_reopen_empty() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // Open and close empty database
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        db.close().unwrap();
    }

    // Reopen and verify it's still empty
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let collections = db.list_collections();
        assert!(collections.is_empty());
        db.close().unwrap();
    }
}

#[test]
fn test_database_persist_single_document() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // Insert a document
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        db.insert_one("users", doc_with_id(1, "Alice")).unwrap();
        db.close().unwrap();
    }

    // Reopen and verify document persists
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let coll = db.collection("users").unwrap();
        let results = coll.find(&json!({"_id": 1})).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["name"], "Alice");
        db.close().unwrap();
    }
}

#[test]
fn test_database_persist_multiple_documents() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // Insert multiple documents
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        for i in 1..=100 {
            db.insert_one("items", doc_with_id(i, &format!("Item {}", i)))
                .unwrap();
        }
        db.close().unwrap();
    }

    // Reopen and verify all documents persist
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let coll = db.collection("items").unwrap();
        let count = coll.count_documents(&json!({})).unwrap();
        assert_eq!(count, 100);

        // Verify specific documents
        let results = coll.find(&json!({"_id": 50})).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["name"], "Item 50");
        db.close().unwrap();
    }
}

#[test]
fn test_database_persist_multiple_collections() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // Create multiple collections with documents
    {
        let db = DatabaseCore::open(&db_path).unwrap();

        db.insert_one("users", doc_with_id(1, "Alice")).unwrap();
        db.insert_one("users", doc_with_id(2, "Bob")).unwrap();

        db.insert_one("products", doc_with_id(100, "Laptop"))
            .unwrap();
        db.insert_one("products", doc_with_id(101, "Phone"))
            .unwrap();
        db.insert_one("products", doc_with_id(102, "Tablet"))
            .unwrap();

        db.insert_one("orders", doc_with_id(1000, "Order1"))
            .unwrap();

        db.close().unwrap();
    }

    // Reopen and verify all collections and documents
    {
        let db = DatabaseCore::open(&db_path).unwrap();

        let collections = db.list_collections();
        assert!(collections.contains(&"users".to_string()));
        assert!(collections.contains(&"products".to_string()));
        assert!(collections.contains(&"orders".to_string()));

        let users = db.collection("users").unwrap();
        assert_eq!(users.count_documents(&json!({})).unwrap(), 2);

        let products = db.collection("products").unwrap();
        assert_eq!(products.count_documents(&json!({})).unwrap(), 3);

        let orders = db.collection("orders").unwrap();
        assert_eq!(orders.count_documents(&json!({})).unwrap(), 1);

        db.close().unwrap();
    }
}

#[test]
fn test_database_persist_updates() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // Insert and update documents
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        db.insert_one("users", doc_with_fields(1, "Alice", 25, "NYC"))
            .unwrap();
        db.insert_one("users", doc_with_fields(2, "Bob", 30, "LA"))
            .unwrap();

        // Update Alice's age
        db.update_one("users", &json!({"_id": 1}), &json!({"$set": {"age": 26}}))
            .unwrap();

        db.close().unwrap();
    }

    // Reopen and verify update persisted
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let coll = db.collection("users").unwrap();

        let results = coll.find(&json!({"_id": 1})).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["age"], 26);
        assert_eq!(results[0]["name"], "Alice");

        db.close().unwrap();
    }
}

#[test]
fn test_database_persist_deletes() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // Insert and delete documents
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        db.insert_one("users", doc_with_id(1, "Alice")).unwrap();
        db.insert_one("users", doc_with_id(2, "Bob")).unwrap();
        db.insert_one("users", doc_with_id(3, "Charlie")).unwrap();

        // Delete Bob
        db.delete_one("users", &json!({"_id": 2})).unwrap();

        db.close().unwrap();
    }

    // Reopen and verify delete persisted
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let coll = db.collection("users").unwrap();

        assert_eq!(coll.count_documents(&json!({})).unwrap(), 2);

        let bob = coll.find(&json!({"_id": 2})).unwrap();
        assert!(bob.is_empty());

        let alice = coll.find(&json!({"_id": 1})).unwrap();
        assert_eq!(alice.len(), 1);

        let charlie = coll.find(&json!({"_id": 3})).unwrap();
        assert_eq!(charlie.len(), 1);

        db.close().unwrap();
    }
}

// ============================================================================
// MULTIPLE REOPEN CYCLES
// ============================================================================

#[test]
fn test_database_multiple_reopen_cycles() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // Cycle 1: Create collection and insert
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        db.insert_one("data", doc_with_id(1, "First")).unwrap();
        db.close().unwrap();
    }

    // Cycle 2: Add more data
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        db.insert_one("data", doc_with_id(2, "Second")).unwrap();
        db.insert_one("data", doc_with_id(3, "Third")).unwrap();
        db.close().unwrap();
    }

    // Cycle 3: Update existing data
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        db.update_one(
            "data",
            &json!({"_id": 1}),
            &json!({"$set": {"name": "Updated First"}}),
        )
        .unwrap();
        db.close().unwrap();
    }

    // Cycle 4: Delete some data
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        db.delete_one("data", &json!({"_id": 2})).unwrap();
        db.close().unwrap();
    }

    // Cycle 5: Verify final state
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let coll = db.collection("data").unwrap();

        assert_eq!(coll.count_documents(&json!({})).unwrap(), 2);

        let first = coll.find(&json!({"_id": 1})).unwrap();
        assert_eq!(first[0]["name"], "Updated First");

        let second = coll.find(&json!({"_id": 2})).unwrap();
        assert!(second.is_empty());

        let third = coll.find(&json!({"_id": 3})).unwrap();
        assert_eq!(third[0]["name"], "Third");

        db.close().unwrap();
    }
}

// ============================================================================
// DROP WITHOUT EXPLICIT CLOSE
// ============================================================================

#[test]
fn test_database_drop_persists_data() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // Insert without explicit close (drop should handle it)
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        db.insert_one("items", doc_with_id(1, "Test")).unwrap();
        // No explicit close - drop will be called
    }

    // Reopen and verify
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let coll = db.collection("items").unwrap();
        let count = coll.count_documents(&json!({})).unwrap();
        assert_eq!(count, 1);
    }
}

// ============================================================================
// LARGE DATA PERSISTENCE
// ============================================================================

#[test]
fn test_database_persist_large_documents() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    let large_text = "x".repeat(10000); // 10KB string

    // Insert large document
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let mut doc = HashMap::new();
        doc.insert("_id".to_string(), json!(1));
        doc.insert("data".to_string(), json!(large_text.clone()));
        db.insert_one("large", doc).unwrap();
        db.close().unwrap();
    }

    // Verify large document persists correctly
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let coll = db.collection("large").unwrap();
        let results = coll.find(&json!({"_id": 1})).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["data"], large_text);
        db.close().unwrap();
    }
}

#[test]
fn test_database_persist_many_documents() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    let doc_count = 1000;

    // Insert many documents
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        for i in 1..=doc_count {
            db.insert_one(
                "bulk",
                doc_with_fields(i, &format!("User{}", i), i % 100, "City"),
            )
            .unwrap();
        }
        db.close().unwrap();
    }

    // Verify all documents persist
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let coll = db.collection("bulk").unwrap();
        assert_eq!(coll.count_documents(&json!({})).unwrap(), doc_count as u64);

        // Verify random samples
        for sample_id in [1, 100, 500, 999, 1000] {
            let results = coll.find(&json!({"_id": sample_id})).unwrap();
            assert_eq!(results.len(), 1);
            assert_eq!(results[0]["name"], format!("User{}", sample_id));
        }

        db.close().unwrap();
    }
}

// ============================================================================
// SPECIAL CHARACTERS AND UNICODE
// ============================================================================

#[test]
fn test_database_persist_unicode_data() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // Insert unicode data
    {
        let db = DatabaseCore::open(&db_path).unwrap();

        let mut doc = HashMap::new();
        doc.insert("_id".to_string(), json!(1));
        doc.insert("name".to_string(), json!("日本語テスト"));
        doc.insert("emoji".to_string(), json!("🎉🚀💾"));
        doc.insert("hungarian".to_string(), json!("árvíztűrő tükörfúrógép"));
        doc.insert("chinese".to_string(), json!("中文测试"));
        db.insert_one("unicode", doc).unwrap();

        db.close().unwrap();
    }

    // Verify unicode data persists correctly
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let coll = db.collection("unicode").unwrap();
        let results = coll.find(&json!({"_id": 1})).unwrap();
        assert_eq!(results.len(), 1);

        let doc = &results[0];
        assert_eq!(doc["name"], "日本語テスト");
        assert_eq!(doc["emoji"], "🎉🚀💾");
        assert_eq!(doc["hungarian"], "árvíztűrő tükörfúrógép");
        assert_eq!(doc["chinese"], "中文测试");

        db.close().unwrap();
    }
}

#[test]
fn test_database_persist_special_characters() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // Insert special characters
    {
        let db = DatabaseCore::open(&db_path).unwrap();

        let mut doc = HashMap::new();
        doc.insert("_id".to_string(), json!(1));
        doc.insert("quotes".to_string(), json!("He said \"Hello\""));
        doc.insert("backslash".to_string(), json!("path\\to\\file"));
        doc.insert("newlines".to_string(), json!("line1\nline2\nline3"));
        doc.insert("tabs".to_string(), json!("col1\tcol2\tcol3"));
        db.insert_one("special", doc).unwrap();

        db.close().unwrap();
    }

    // Verify special characters persist
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let coll = db.collection("special").unwrap();
        let results = coll.find(&json!({"_id": 1})).unwrap();
        assert_eq!(results.len(), 1);

        let doc = &results[0];
        assert_eq!(doc["quotes"], "He said \"Hello\"");
        assert_eq!(doc["backslash"], "path\\to\\file");
        assert_eq!(doc["newlines"], "line1\nline2\nline3");

        db.close().unwrap();
    }
}

// ============================================================================
// NESTED DOCUMENT PERSISTENCE
// ============================================================================

#[test]
fn test_database_persist_nested_documents() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // Insert nested document
    {
        let db = DatabaseCore::open(&db_path).unwrap();

        let mut doc = HashMap::new();
        doc.insert("_id".to_string(), json!(1));
        doc.insert(
            "user".to_string(),
            json!({
                "name": "Alice",
                "address": {
                    "street": "123 Main St",
                    "city": "NYC",
                    "zip": "10001"
                },
                "contacts": [
                    {"type": "email", "value": "alice@example.com"},
                    {"type": "phone", "value": "555-1234"}
                ]
            }),
        );
        db.insert_one("nested", doc).unwrap();

        db.close().unwrap();
    }

    // Verify nested document persists
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let coll = db.collection("nested").unwrap();
        let results = coll.find(&json!({"_id": 1})).unwrap();
        assert_eq!(results.len(), 1);

        let doc = &results[0];
        assert_eq!(doc["user"]["name"], "Alice");
        assert_eq!(doc["user"]["address"]["city"], "NYC");
        assert_eq!(doc["user"]["contacts"][0]["type"], "email");

        db.close().unwrap();
    }
}

// ============================================================================
// ARRAY PERSISTENCE
// ============================================================================

#[test]
fn test_database_persist_arrays() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // Insert document with various array types
    {
        let db = DatabaseCore::open(&db_path).unwrap();

        let mut doc = HashMap::new();
        doc.insert("_id".to_string(), json!(1));
        doc.insert("numbers".to_string(), json!([1, 2, 3, 4, 5]));
        doc.insert("strings".to_string(), json!(["a", "b", "c"]));
        doc.insert("mixed".to_string(), json!([1, "two", 3.0, true, null]));
        doc.insert("nested_arrays".to_string(), json!([[1, 2], [3, 4], [5, 6]]));
        doc.insert("empty".to_string(), json!([]));
        db.insert_one("arrays", doc).unwrap();

        db.close().unwrap();
    }

    // Verify arrays persist
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let coll = db.collection("arrays").unwrap();
        let results = coll.find(&json!({"_id": 1})).unwrap();
        assert_eq!(results.len(), 1);

        let doc = &results[0];
        assert_eq!(doc["numbers"], json!([1, 2, 3, 4, 5]));
        assert_eq!(doc["strings"], json!(["a", "b", "c"]));
        assert_eq!(doc["empty"], json!([]));

        db.close().unwrap();
    }
}

// ============================================================================
// NUMERIC EDGE CASES
// ============================================================================

#[test]
fn test_database_persist_numeric_edge_cases() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // Insert numeric edge cases
    {
        let db = DatabaseCore::open(&db_path).unwrap();

        let mut doc = HashMap::new();
        doc.insert("_id".to_string(), json!(1));
        doc.insert("max_i64".to_string(), json!(i64::MAX));
        doc.insert("min_i64".to_string(), json!(i64::MIN));
        doc.insert("zero".to_string(), json!(0));
        doc.insert("negative".to_string(), json!(-12345));
        doc.insert("float".to_string(), json!(3.14159));
        doc.insert("negative_float".to_string(), json!(-273.15));
        db.insert_one("numbers", doc).unwrap();

        db.close().unwrap();
    }

    // Verify numeric values persist
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let coll = db.collection("numbers").unwrap();
        let results = coll.find(&json!({"_id": 1})).unwrap();
        assert_eq!(results.len(), 1);

        let doc = &results[0];
        assert_eq!(doc["max_i64"], json!(i64::MAX));
        assert_eq!(doc["min_i64"], json!(i64::MIN));
        assert_eq!(doc["zero"], json!(0));

        db.close().unwrap();
    }
}

// ============================================================================
// FILE SYSTEM EDGE CASES
// ============================================================================

#[test]
fn test_database_file_permissions() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // Create and populate database
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        db.insert_one("test", doc_with_id(1, "Test")).unwrap();
        db.close().unwrap();
    }

    // Verify file exists
    assert!(db_path.exists());

    // Reopen should work
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let coll = db.collection("test").unwrap();
        assert_eq!(coll.count_documents(&json!({})).unwrap(), 1);
    }
}

#[test]
fn test_database_path_with_spaces() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("path with spaces").join("test.mlite");

    // Create parent directory
    fs::create_dir_all(db_path.parent().unwrap()).unwrap();

    // Create and populate database
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        db.insert_one("test", doc_with_id(1, "Test")).unwrap();
        db.close().unwrap();
    }

    // Reopen and verify
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let coll = db.collection("test").unwrap();
        assert_eq!(coll.count_documents(&json!({})).unwrap(), 1);
    }
}

// ============================================================================
// TRANSACTION PERSISTENCE
// ============================================================================

#[test]
fn test_database_transaction_commit_persists() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // Perform transaction and commit
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let tx_id = db.begin_transaction();
        db.insert_one_tx("txn", doc_with_id(1, "TxnDoc"), tx_id)
            .unwrap();
        db.commit_transaction(tx_id).unwrap();
        db.close().unwrap();
    }

    // Verify committed data persists
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let coll = db.collection("txn").unwrap();
        let count = coll.count_documents(&json!({})).unwrap();
        assert_eq!(count, 1);
    }
}

#[test]
fn test_database_transaction_rollback_discards() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // Insert initial data
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        db.insert_one("txn", doc_with_id(1, "Initial")).unwrap();
        db.close().unwrap();
    }

    // Perform transaction and rollback
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let tx_id = db.begin_transaction();
        db.insert_one_tx("txn", doc_with_id(2, "RolledBack"), tx_id)
            .unwrap();
        db.rollback_transaction(tx_id).unwrap();
        db.close().unwrap();
    }

    // Verify rolled back data is not persisted
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let coll = db.collection("txn").unwrap();
        let count = coll.count_documents(&json!({})).unwrap();
        assert_eq!(count, 1);

        let results = coll.find(&json!({"_id": 2})).unwrap();
        assert!(results.is_empty());
    }
}

// ============================================================================
// COLLECTION OPERATIONS
// ============================================================================

#[test]
fn test_database_drop_collection_persists() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // Create collections and drop one
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        db.insert_one("keep", doc_with_id(1, "Keep")).unwrap();
        db.insert_one("drop", doc_with_id(1, "Drop")).unwrap();
        db.drop_collection("drop").unwrap();
        db.close().unwrap();
    }

    // Verify drop persisted
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let collections = db.list_collections();
        assert!(collections.contains(&"keep".to_string()));
        assert!(!collections.contains(&"drop".to_string()));

        let keep = db.collection("keep").unwrap();
        assert_eq!(keep.count_documents(&json!({})).unwrap(), 1);
    }
}

// ============================================================================
// COMPLEX QUERY AFTER REOPEN
// ============================================================================

#[test]
fn test_database_complex_queries_after_reopen() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // Insert test data
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        for i in 1..=50 {
            db.insert_one(
                "products",
                doc_with_fields(
                    i,
                    &format!("Product{}", i),
                    i * 10,
                    if i % 2 == 0 { "A" } else { "B" },
                ),
            )
            .unwrap();
        }
        db.close().unwrap();
    }

    // Perform complex queries after reopen
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let coll = db.collection("products").unwrap();

        // Range query
        let range = coll
            .find(&json!({"age": {"$gte": 200, "$lte": 300}}))
            .unwrap();
        assert_eq!(range.len(), 11); // 20-30 inclusive

        // Equality query
        let city_a = coll.find(&json!({"city": "A"})).unwrap();
        assert_eq!(city_a.len(), 25); // Even numbers

        // Combined query
        let combined = coll
            .find(&json!({"city": "A", "age": {"$gt": 400}}))
            .unwrap();
        assert!(!combined.is_empty());

        db.close().unwrap();
    }
}

// ============================================================================
// INDEX PERSISTENCE
// ============================================================================

#[test]
fn test_database_index_persists_after_reopen() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // Create index and insert data
    {
        let db = DatabaseCore::open(&db_path).unwrap();

        // Insert data first
        for i in 1..=100 {
            db.insert_one(
                "indexed",
                doc_with_fields(i, &format!("User{}", i), i, "City"),
            )
            .unwrap();
        }

        // Create index
        let coll = db.collection("indexed").unwrap();
        coll.create_index("age".to_string(), false).unwrap();

        db.close().unwrap();
    }

    // Verify index works after reopen
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let coll = db.collection("indexed").unwrap();

        // Query should use index and return results
        let results = coll.find(&json!({"age": 50})).unwrap();
        assert!(!results.is_empty(), "Index query should return results");

        // Verify the correct document is in results
        let has_user50 = results.iter().any(|doc| doc["name"] == "User50");
        assert!(has_user50, "Results should contain User50");

        db.close().unwrap();
    }
}

// ============================================================================
// SEQUENTIAL ACCESS
// ============================================================================

#[test]
fn test_database_sequential_access() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // Multiple sequential open/close cycles
    for i in 1..=10 {
        let db = DatabaseCore::open(&db_path).unwrap();
        db.insert_one("seq", doc_with_id(i, &format!("Doc{}", i)))
            .unwrap();
        db.close().unwrap();
    }

    // Verify all data
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let coll = db.collection("seq").unwrap();
        assert_eq!(coll.count_documents(&json!({})).unwrap(), 10);
    }
}

// ============================================================================
// ERROR RECOVERY
// ============================================================================

#[test]
fn test_database_reopen_after_error() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // Create database with valid data
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        db.insert_one("test", doc_with_id(1, "Valid")).unwrap();
        db.close().unwrap();
    }

    // Attempt operation that might fail (non-existent collection query)
    {
        let db = DatabaseCore::open(&db_path).unwrap();

        // This should not corrupt the database
        let _result = db.collection("nonexistent");
        // Collection might be created or error - either way, db should be usable

        db.close().unwrap();
    }

    // Verify database is still usable
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let coll = db.collection("test").unwrap();
        let count = coll.count_documents(&json!({})).unwrap();
        assert_eq!(count, 1);
    }
}
