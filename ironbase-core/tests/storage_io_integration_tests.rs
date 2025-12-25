// storage_io_integration_tests.rs
// Integration tests for storage I/O operations, file handling, and edge cases

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

// ============================================================================
// FILE SIZE AND GROWTH TESTS
// ============================================================================

#[test]
fn test_storage_file_grows_with_data() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // Create empty database and check initial size
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        db.close().unwrap();
    }

    let initial_size = fs::metadata(&db_path).unwrap().len();
    assert!(initial_size > 0, "Empty database should have header");

    // Add documents and verify file grows
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        for i in 1..=100 {
            let mut doc = HashMap::new();
            doc.insert("_id".to_string(), json!(i));
            doc.insert("data".to_string(), json!("x".repeat(1000)));
            db.insert_one("large", doc).unwrap();
        }
        db.close().unwrap();
    }

    let final_size = fs::metadata(&db_path).unwrap().len();
    assert!(
        final_size > initial_size,
        "File should grow after adding data"
    );
    assert!(
        final_size > 100000,
        "File should be at least 100KB with 100 1KB documents"
    );
}

#[test]
fn test_storage_multiple_collections_in_single_file() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // Create multiple collections
    {
        let db = DatabaseCore::open(&db_path).unwrap();

        for coll_name in ["users", "products", "orders", "logs", "settings"] {
            for i in 1..=10 {
                db.insert_one(coll_name, doc_with_id(i, &format!("{}_{}", coll_name, i)))
                    .unwrap();
            }
        }

        db.close().unwrap();
    }

    // Verify all collections are in single file
    let files: Vec<_> = fs::read_dir(temp_dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "mlite"))
        .collect();

    assert_eq!(files.len(), 1, "Should have exactly one .mlite file");

    // Verify all data is accessible
    {
        let db = DatabaseCore::open(&db_path).unwrap();

        for coll_name in ["users", "products", "orders", "logs", "settings"] {
            let coll = db.collection(coll_name).unwrap();
            assert_eq!(
                coll.count_documents(&json!({})).unwrap(),
                10,
                "Collection {} should have 10 documents",
                coll_name
            );
        }
    }
}

// ============================================================================
// REOPEN TESTS
// ============================================================================

#[test]
fn test_storage_reopen_after_clean_close() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // First open/close cycle
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        db.insert_one("test", doc_with_id(1, "First")).unwrap();
        db.close().unwrap();
    }

    // Second open should succeed
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let coll = db.collection("test").unwrap();
        assert_eq!(coll.count_documents(&json!({})).unwrap(), 1);
        db.close().unwrap();
    }

    // Third open should succeed
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let coll = db.collection("test").unwrap();
        assert_eq!(coll.count_documents(&json!({})).unwrap(), 1);
    }
}

// ============================================================================
// BOUNDARY SIZE TESTS
// ============================================================================

#[test]
fn test_storage_empty_string_values() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // Insert document with empty strings
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let mut doc = HashMap::new();
        doc.insert("_id".to_string(), json!(1));
        doc.insert("empty".to_string(), json!(""));
        doc.insert("name".to_string(), json!(""));
        doc.insert("data".to_string(), json!(""));
        db.insert_one("empty", doc).unwrap();
        db.close().unwrap();
    }

    // Verify empty strings persist
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let coll = db.collection("empty").unwrap();
        let results = coll.find(&json!({"_id": 1})).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["empty"], "");
        assert_eq!(results[0]["name"], "");
    }
}

#[test]
fn test_storage_null_values() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // Insert document with null values
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let mut doc = HashMap::new();
        doc.insert("_id".to_string(), json!(1));
        doc.insert("null_field".to_string(), json!(null));
        doc.insert("name".to_string(), json!("Test"));
        db.insert_one("nulls", doc).unwrap();
        db.close().unwrap();
    }

    // Verify null values persist
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let coll = db.collection("nulls").unwrap();
        let results = coll.find(&json!({"_id": 1})).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0]["null_field"].is_null());
    }
}

#[test]
fn test_storage_boolean_values() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // Insert document with boolean values
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let mut doc = HashMap::new();
        doc.insert("_id".to_string(), json!(1));
        doc.insert("active".to_string(), json!(true));
        doc.insert("deleted".to_string(), json!(false));
        db.insert_one("bools", doc).unwrap();
        db.close().unwrap();
    }

    // Verify boolean values persist
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let coll = db.collection("bools").unwrap();
        let results = coll.find(&json!({"_id": 1})).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["active"], true);
        assert_eq!(results[0]["deleted"], false);
    }
}

// ============================================================================
// DOCUMENT ID TYPES
// ============================================================================

#[test]
fn test_storage_string_document_ids() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // Insert documents with string IDs
    {
        let db = DatabaseCore::open(&db_path).unwrap();

        for id in ["alpha", "beta", "gamma", "delta"] {
            let mut doc = HashMap::new();
            doc.insert("_id".to_string(), json!(id));
            doc.insert("name".to_string(), json!(format!("Name-{}", id)));
            db.insert_one("string_ids", doc).unwrap();
        }

        db.close().unwrap();
    }

    // Verify string IDs work after reopen
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let coll = db.collection("string_ids").unwrap();

        assert_eq!(coll.count_documents(&json!({})).unwrap(), 4);

        let results = coll.find(&json!({"_id": "beta"})).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["name"], "Name-beta");
    }
}

#[test]
fn test_storage_mixed_document_id_types() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // Insert documents with mixed ID types
    {
        let db = DatabaseCore::open(&db_path).unwrap();

        let mut doc1 = HashMap::new();
        doc1.insert("_id".to_string(), json!(1));
        doc1.insert("type".to_string(), json!("integer"));
        db.insert_one("mixed", doc1).unwrap();

        let mut doc2 = HashMap::new();
        doc2.insert("_id".to_string(), json!("string-id"));
        doc2.insert("type".to_string(), json!("string"));
        db.insert_one("mixed", doc2).unwrap();

        let mut doc3 = HashMap::new();
        doc3.insert("_id".to_string(), json!("507f1f77bcf86cd799439011"));
        doc3.insert("type".to_string(), json!("objectid"));
        db.insert_one("mixed", doc3).unwrap();

        db.close().unwrap();
    }

    // Verify mixed IDs work
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let coll = db.collection("mixed").unwrap();

        assert_eq!(coll.count_documents(&json!({})).unwrap(), 3);

        // Query by integer ID
        let int_results = coll.find(&json!({"_id": 1})).unwrap();
        assert_eq!(int_results.len(), 1);
        assert_eq!(int_results[0]["type"], "integer");

        // Query by string ID
        let str_results = coll.find(&json!({"_id": "string-id"})).unwrap();
        assert_eq!(str_results.len(), 1);
        assert_eq!(str_results[0]["type"], "string");
    }
}

// ============================================================================
// DEEP NESTING TESTS
// ============================================================================

#[test]
fn test_storage_deeply_nested_documents() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // Create deeply nested structure
    let nested = json!({
        "level1": {
            "level2": {
                "level3": {
                    "level4": {
                        "level5": {
                            "value": "deep"
                        }
                    }
                }
            }
        }
    });

    // Insert deeply nested document
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let mut doc = HashMap::new();
        doc.insert("_id".to_string(), json!(1));
        doc.insert("nested".to_string(), nested.clone());
        db.insert_one("deep", doc).unwrap();
        db.close().unwrap();
    }

    // Verify deep nesting persists
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let coll = db.collection("deep").unwrap();
        let results = coll.find(&json!({"_id": 1})).unwrap();
        assert_eq!(results.len(), 1);

        let doc = &results[0];
        assert_eq!(
            doc["nested"]["level1"]["level2"]["level3"]["level4"]["level5"]["value"],
            "deep"
        );
    }
}

// ============================================================================
// LARGE ARRAY TESTS
// ============================================================================

#[test]
fn test_storage_large_arrays() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    let large_array: Vec<i64> = (1..=1000).collect();

    // Insert document with large array
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let mut doc = HashMap::new();
        doc.insert("_id".to_string(), json!(1));
        doc.insert("numbers".to_string(), json!(large_array.clone()));
        db.insert_one("arrays", doc).unwrap();
        db.close().unwrap();
    }

    // Verify large array persists
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let coll = db.collection("arrays").unwrap();
        let results = coll.find(&json!({"_id": 1})).unwrap();
        assert_eq!(results.len(), 1);

        let arr = results[0]["numbers"].as_array().unwrap();
        assert_eq!(arr.len(), 1000);
        assert_eq!(arr[0], 1);
        assert_eq!(arr[999], 1000);
    }
}

// ============================================================================
// UPDATE OPERATIONS PERSISTENCE
// ============================================================================

#[test]
fn test_storage_update_set_persists() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // Insert and update with $set
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        db.insert_one("test", doc_with_id(1, "Original")).unwrap();
        db.update_one(
            "test",
            &json!({"_id": 1}),
            &json!({"$set": {"name": "Updated", "new_field": "added"}}),
        )
        .unwrap();
        db.close().unwrap();
    }

    // Verify update persisted
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let coll = db.collection("test").unwrap();
        let results = coll.find(&json!({"_id": 1})).unwrap();
        assert_eq!(results[0]["name"], "Updated");
        assert_eq!(results[0]["new_field"], "added");
    }
}

#[test]
fn test_storage_update_unset_persists() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // Insert and update with $unset
    {
        let db = DatabaseCore::open(&db_path).unwrap();

        let mut doc = HashMap::new();
        doc.insert("_id".to_string(), json!(1));
        doc.insert("name".to_string(), json!("Test"));
        doc.insert("to_remove".to_string(), json!("Remove me"));
        db.insert_one("test", doc).unwrap();

        db.update_one(
            "test",
            &json!({"_id": 1}),
            &json!({"$unset": {"to_remove": ""}}),
        )
        .unwrap();
        db.close().unwrap();
    }

    // Verify $unset persisted
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let coll = db.collection("test").unwrap();
        let results = coll.find(&json!({"_id": 1})).unwrap();
        assert_eq!(results[0]["name"], "Test");
        assert!(results[0].get("to_remove").is_none());
    }
}

#[test]
fn test_storage_update_inc_persists() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // Insert and update with $inc
    {
        let db = DatabaseCore::open(&db_path).unwrap();

        let mut doc = HashMap::new();
        doc.insert("_id".to_string(), json!(1));
        doc.insert("counter".to_string(), json!(10));
        db.insert_one("test", doc).unwrap();

        db.update_one("test", &json!({"_id": 1}), &json!({"$inc": {"counter": 5}}))
            .unwrap();
        db.close().unwrap();
    }

    // Verify $inc persisted
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let coll = db.collection("test").unwrap();
        let results = coll.find(&json!({"_id": 1})).unwrap();
        assert_eq!(results[0]["counter"], 15);
    }
}

#[test]
fn test_storage_update_push_persists() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // Insert and update with $push
    {
        let db = DatabaseCore::open(&db_path).unwrap();

        let mut doc = HashMap::new();
        doc.insert("_id".to_string(), json!(1));
        doc.insert("items".to_string(), json!(["a", "b"]));
        db.insert_one("test", doc).unwrap();

        db.update_one(
            "test",
            &json!({"_id": 1}),
            &json!({"$push": {"items": "c"}}),
        )
        .unwrap();
        db.close().unwrap();
    }

    // Verify $push persisted
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let coll = db.collection("test").unwrap();
        let results = coll.find(&json!({"_id": 1})).unwrap();
        assert_eq!(results[0]["items"], json!(["a", "b", "c"]));
    }
}

// ============================================================================
// DELETE OPERATIONS PERSISTENCE
// ============================================================================

#[test]
fn test_storage_delete_one_persists() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // Insert and delete
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        db.insert_one("test", doc_with_id(1, "One")).unwrap();
        db.insert_one("test", doc_with_id(2, "Two")).unwrap();
        db.insert_one("test", doc_with_id(3, "Three")).unwrap();

        db.delete_one("test", &json!({"_id": 2})).unwrap();
        db.close().unwrap();
    }

    // Verify delete persisted
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let coll = db.collection("test").unwrap();

        assert_eq!(coll.count_documents(&json!({})).unwrap(), 2);

        let deleted = coll.find(&json!({"_id": 2})).unwrap();
        assert!(deleted.is_empty());

        let remaining = coll.find(&json!({"_id": 1})).unwrap();
        assert_eq!(remaining.len(), 1);
    }
}

#[test]
fn test_storage_delete_many_persists() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // Insert and delete many
    {
        let db = DatabaseCore::open(&db_path).unwrap();

        for i in 1..=10 {
            let mut doc = HashMap::new();
            doc.insert("_id".to_string(), json!(i));
            doc.insert(
                "category".to_string(),
                json!(if i % 2 == 0 { "even" } else { "odd" }),
            );
            db.insert_one("test", doc).unwrap();
        }

        db.delete_many("test", &json!({"category": "even"}))
            .unwrap();
        db.close().unwrap();
    }

    // Verify delete_many persisted
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let coll = db.collection("test").unwrap();

        assert_eq!(coll.count_documents(&json!({})).unwrap(), 5);

        let even = coll.find(&json!({"category": "even"})).unwrap();
        assert!(even.is_empty());

        let odd = coll.find(&json!({"category": "odd"})).unwrap();
        assert_eq!(odd.len(), 5);
    }
}

// ============================================================================
// STRESS TESTS
// ============================================================================

#[test]
fn test_storage_rapid_insert_close_reopen() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // Rapid cycles of insert + close + reopen
    for cycle in 1..=20 {
        {
            let db = DatabaseCore::open(&db_path).unwrap();
            db.insert_one("rapid", doc_with_id(cycle, &format!("Cycle{}", cycle)))
                .unwrap();
            db.close().unwrap();
        }
    }

    // Verify all data persisted
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let coll = db.collection("rapid").unwrap();
        assert_eq!(coll.count_documents(&json!({})).unwrap(), 20);
    }
}

#[test]
fn test_storage_mixed_operations_persistence() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // Mixed operations in single session
    {
        let db = DatabaseCore::open(&db_path).unwrap();

        // Insert batch
        for i in 1..=50 {
            db.insert_one("mixed", doc_with_id(i, &format!("Doc{}", i)))
                .unwrap();
        }

        // Update some
        for i in [5, 10, 15, 20, 25] {
            db.update_one(
                "mixed",
                &json!({"_id": i}),
                &json!({"$set": {"updated": true}}),
            )
            .unwrap();
        }

        // Delete some
        for i in [3, 6, 9, 12] {
            db.delete_one("mixed", &json!({"_id": i})).unwrap();
        }

        // Insert more
        for i in 51..=60 {
            db.insert_one("mixed", doc_with_id(i, &format!("Doc{}", i)))
                .unwrap();
        }

        db.close().unwrap();
    }

    // Verify final state
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let coll = db.collection("mixed").unwrap();

        // 50 original - 4 deleted + 10 new = 56
        assert_eq!(coll.count_documents(&json!({})).unwrap(), 56);

        // Verify updates persisted
        let updated = coll.find(&json!({"updated": true})).unwrap();
        assert_eq!(updated.len(), 5);

        // Verify deletes persisted
        let deleted = coll.find(&json!({"_id": 3})).unwrap();
        assert!(deleted.is_empty());
    }
}
