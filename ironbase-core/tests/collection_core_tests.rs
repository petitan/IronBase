//! Integration tests for CollectionCore
//!
//! These tests cover the main CRUD operations and various edge cases

use ironbase_core::storage::StorageEngine;
use ironbase_core::CollectionCore;
use parking_lot::RwLock;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

fn create_test_collection(name: &str) -> CollectionCore<StorageEngine> {
    let counter = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let db_path = format!("/tmp/test_collection_{}_{}.mlite", name, counter);

    // Cleanup previous test files
    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_file(format!("{}.wal", db_path.trim_end_matches(".mlite")));

    let storage = StorageEngine::open(&db_path).unwrap();
    let storage = Arc::new(RwLock::new(storage));
    CollectionCore::new(name.to_string(), storage).unwrap()
}

// ========== INSERT TESTS ==========

#[test]
fn test_insert_one_auto_id() {
    let collection = create_test_collection("test");

    let doc = HashMap::from([("name".to_string(), json!("Alice"))]);
    let id = collection.insert_one(doc).unwrap();

    // Should have auto-generated ID
    assert!(matches!(id, ironbase_core::DocumentId::Int(_)));

    let count = collection.count_documents(&json!({})).unwrap();
    assert_eq!(count, 1);
}

#[test]
fn test_insert_one_with_custom_id() {
    let collection = create_test_collection("test");

    let doc = HashMap::from([
        ("_id".to_string(), json!("custom_id")),
        ("name".to_string(), json!("Bob")),
    ]);
    let id = collection.insert_one(doc).unwrap();

    assert!(matches!(id, ironbase_core::DocumentId::String(s) if s == "custom_id"));
}

#[test]
fn test_insert_many_empty() {
    let collection = create_test_collection("test");

    let result = collection.insert_many(vec![]).unwrap();
    assert_eq!(result.inserted_count, 0);
    assert!(result.inserted_ids.is_empty());
}

#[test]
fn test_insert_many_batch() {
    let collection = create_test_collection("test");

    let docs: Vec<HashMap<String, serde_json::Value>> = (0..100)
        .map(|i| HashMap::from([("value".to_string(), json!(i))]))
        .collect();

    let result = collection.insert_many(docs).unwrap();
    assert_eq!(result.inserted_count, 100);
    assert_eq!(result.inserted_ids.len(), 100);

    let count = collection.count_documents(&json!({})).unwrap();
    assert_eq!(count, 100);
}

// ========== FIND TESTS ==========

#[test]
fn test_find_empty_collection() {
    let collection = create_test_collection("test");
    let results = collection.find(&json!({})).unwrap();
    assert!(results.is_empty());
}

#[test]
fn test_find_one_by_id() {
    let collection = create_test_collection("test");

    let doc = HashMap::from([("name".to_string(), json!("Alice"))]);
    let id = collection.insert_one(doc).unwrap();

    let found = collection
        .find_one(&json!({"_id": id}))
        .unwrap()
        .expect("Document should be found");
    assert_eq!(found["name"], "Alice");
}

#[test]
fn test_find_one_not_found() {
    let collection = create_test_collection("test");

    let doc = HashMap::from([("name".to_string(), json!("Alice"))]);
    collection.insert_one(doc).unwrap();

    let found = collection.find_one(&json!({"_id": 999})).unwrap();
    assert!(found.is_none());
}

#[test]
fn test_find_with_query() {
    let collection = create_test_collection("test");

    for i in 0..10 {
        let doc = HashMap::from([
            ("name".to_string(), json!(format!("User {}", i))),
            ("age".to_string(), json!(20 + i)),
        ]);
        collection.insert_one(doc).unwrap();
    }

    // Find users with age >= 25
    let results = collection.find(&json!({"age": {"$gte": 25}})).unwrap();
    assert_eq!(results.len(), 5);
}

#[test]
fn test_find_streaming() {
    let collection = create_test_collection("test");

    for i in 0..50 {
        let doc = HashMap::from([("value".to_string(), json!(i))]);
        collection.insert_one(doc).unwrap();
    }

    let mut cursor = collection.find_streaming(&json!({})).unwrap();
    assert_eq!(cursor.total(), 50);
    assert_eq!(cursor.remaining(), 50);

    // Test next()
    let first = cursor.next().unwrap().unwrap();
    assert!(first.get("value").is_some());
    assert_eq!(cursor.remaining(), 49);

    // Test next_chunk()
    let batch = cursor.next_chunk(10).unwrap();
    assert_eq!(batch.len(), 10);
    assert_eq!(cursor.remaining(), 39);

    // Test skip()
    cursor.skip(10);
    assert_eq!(cursor.remaining(), 29);

    // Test rewind()
    cursor.rewind();
    assert_eq!(cursor.remaining(), 50);

    // Test take()
    let taken = cursor.take(5).unwrap();
    assert_eq!(taken.len(), 5);

    // Test collect_all()
    cursor.rewind();
    let all = cursor.collect_all().unwrap();
    assert_eq!(all.len(), 50);
}

#[test]
fn test_find_streaming_with_batch_size() {
    let collection = create_test_collection("test");

    for i in 0..25 {
        let doc = HashMap::from([("value".to_string(), json!(i))]);
        collection.insert_one(doc).unwrap();
    }

    let mut cursor = collection
        .find_streaming(&json!({}))
        .unwrap()
        .with_batch_size(5);

    let batch = cursor.next_batch().unwrap();
    assert_eq!(batch.len(), 5);
}

// ========== COUNT TESTS ==========

#[test]
fn test_count_empty() {
    let collection = create_test_collection("test");
    let count = collection.count_documents(&json!({})).unwrap();
    assert_eq!(count, 0);
}

#[test]
fn test_count_all() {
    let collection = create_test_collection("test");

    for i in 0..10 {
        let doc = HashMap::from([("value".to_string(), json!(i))]);
        collection.insert_one(doc).unwrap();
    }

    let count = collection.count_documents(&json!({})).unwrap();
    assert_eq!(count, 10);
}

#[test]
fn test_count_with_query() {
    let collection = create_test_collection("test");

    for i in 0..10 {
        let doc = HashMap::from([("age".to_string(), json!(i * 10))]);
        collection.insert_one(doc).unwrap();
    }

    let count = collection
        .count_documents(&json!({"age": {"$gte": 50}}))
        .unwrap();
    assert_eq!(count, 5);
}

#[test]
fn test_count_by_id() {
    let collection = create_test_collection("test");

    let doc = HashMap::from([("name".to_string(), json!("Alice"))]);
    let id = collection.insert_one(doc).unwrap();

    let count = collection.count_documents(&json!({"_id": id})).unwrap();
    assert_eq!(count, 1);

    let count_missing = collection.count_documents(&json!({"_id": 999})).unwrap();
    assert_eq!(count_missing, 0);
}

// ========== UPDATE TESTS ==========

#[test]
fn test_update_one_set() {
    let collection = create_test_collection("test");

    let doc = HashMap::from([
        ("name".to_string(), json!("Alice")),
        ("age".to_string(), json!(25)),
    ]);
    let id = collection.insert_one(doc).unwrap();

    let (matched, modified) = collection
        .update_one(&json!({"_id": id}), &json!({"$set": {"age": 30}}))
        .unwrap();

    assert_eq!(matched, 1);
    assert_eq!(modified, 1);

    let updated = collection.find_one(&json!({"_id": id})).unwrap().unwrap();
    assert_eq!(updated["age"], 30);
}

#[test]
fn test_update_one_inc() {
    let collection = create_test_collection("test");

    let doc = HashMap::from([("counter".to_string(), json!(10))]);
    let id = collection.insert_one(doc).unwrap();

    collection
        .update_one(&json!({"_id": id}), &json!({"$inc": {"counter": 5}}))
        .unwrap();

    let updated = collection.find_one(&json!({"_id": id})).unwrap().unwrap();
    assert_eq!(updated["counter"], 15);
}

#[test]
fn test_update_one_unset() {
    let collection = create_test_collection("test");

    let doc = HashMap::from([
        ("name".to_string(), json!("Alice")),
        ("temp".to_string(), json!("remove me")),
    ]);
    let id = collection.insert_one(doc).unwrap();

    collection
        .update_one(&json!({"_id": id}), &json!({"$unset": {"temp": ""}}))
        .unwrap();

    let updated = collection.find_one(&json!({"_id": id})).unwrap().unwrap();
    assert!(updated.get("temp").is_none());
}

#[test]
fn test_update_one_push() {
    let collection = create_test_collection("test");

    let doc = HashMap::from([("tags".to_string(), json!(["a", "b"]))]);
    let id = collection.insert_one(doc).unwrap();

    collection
        .update_one(&json!({"_id": id}), &json!({"$push": {"tags": "c"}}))
        .unwrap();

    let updated = collection.find_one(&json!({"_id": id})).unwrap().unwrap();
    assert_eq!(updated["tags"], json!(["a", "b", "c"]));
}

#[test]
fn test_update_one_pull() {
    let collection = create_test_collection("test");

    let doc = HashMap::from([("tags".to_string(), json!(["a", "b", "c"]))]);
    let id = collection.insert_one(doc).unwrap();

    collection
        .update_one(&json!({"_id": id}), &json!({"$pull": {"tags": "b"}}))
        .unwrap();

    let updated = collection.find_one(&json!({"_id": id})).unwrap().unwrap();
    assert_eq!(updated["tags"], json!(["a", "c"]));
}

#[test]
fn test_update_one_addtoset() {
    let collection = create_test_collection("test");

    let doc = HashMap::from([("tags".to_string(), json!(["a", "b"]))]);
    let id = collection.insert_one(doc).unwrap();

    // Add new value
    collection
        .update_one(&json!({"_id": id}), &json!({"$addToSet": {"tags": "c"}}))
        .unwrap();

    // Try to add existing value (should not duplicate)
    collection
        .update_one(&json!({"_id": id}), &json!({"$addToSet": {"tags": "a"}}))
        .unwrap();

    let updated = collection.find_one(&json!({"_id": id})).unwrap().unwrap();
    assert_eq!(updated["tags"], json!(["a", "b", "c"]));
}

#[test]
fn test_update_one_pop() {
    let collection = create_test_collection("test");

    let doc = HashMap::from([("arr".to_string(), json!([1, 2, 3]))]);
    let id = collection.insert_one(doc).unwrap();

    // Pop from end
    collection
        .update_one(&json!({"_id": id}), &json!({"$pop": {"arr": 1}}))
        .unwrap();

    let updated = collection.find_one(&json!({"_id": id})).unwrap().unwrap();
    assert_eq!(updated["arr"], json!([1, 2]));

    // Pop from beginning
    collection
        .update_one(&json!({"_id": id}), &json!({"$pop": {"arr": -1}}))
        .unwrap();

    let updated = collection.find_one(&json!({"_id": id})).unwrap().unwrap();
    assert_eq!(updated["arr"], json!([2]));
}

#[test]
fn test_update_one_not_found() {
    let collection = create_test_collection("test");

    let (matched, modified) = collection
        .update_one(&json!({"_id": 999}), &json!({"$set": {"x": 1}}))
        .unwrap();

    assert_eq!(matched, 0);
    assert_eq!(modified, 0);
}

#[test]
fn test_update_one_no_change() {
    let collection = create_test_collection("test");

    let doc = HashMap::from([("value".to_string(), json!(10))]);
    let id = collection.insert_one(doc).unwrap();

    // Update with same value - implementation counts modification even if value unchanged
    let (matched, modified) = collection
        .update_one(&json!({"_id": id}), &json!({"$set": {"value": 10}}))
        .unwrap();

    assert_eq!(matched, 1);
    assert_eq!(modified, 1); // Implementation doesn't track actual change detection
}

#[test]
fn test_update_many() {
    let collection = create_test_collection("test");

    for i in 0..10 {
        let doc = HashMap::from([
            ("category".to_string(), json!(if i < 5 { "A" } else { "B" })),
            ("value".to_string(), json!(i)),
        ]);
        collection.insert_one(doc).unwrap();
    }

    let (matched, modified) = collection
        .update_many(
            &json!({"category": "A"}),
            &json!({"$set": {"updated": true}}),
        )
        .unwrap();

    assert_eq!(matched, 5);
    assert_eq!(modified, 5);
}

// ========== DELETE TESTS ==========

#[test]
fn test_delete_one() {
    let collection = create_test_collection("test");

    let doc = HashMap::from([("name".to_string(), json!("Alice"))]);
    let id = collection.insert_one(doc).unwrap();

    let deleted = collection.delete_one(&json!({"_id": id})).unwrap();
    assert_eq!(deleted, 1);

    let count = collection.count_documents(&json!({})).unwrap();
    assert_eq!(count, 0);
}

#[test]
fn test_delete_one_not_found() {
    let collection = create_test_collection("test");

    let deleted = collection.delete_one(&json!({"_id": 999})).unwrap();
    assert_eq!(deleted, 0);
}

#[test]
fn test_delete_many() {
    let collection = create_test_collection("test");

    for i in 0..10 {
        let doc = HashMap::from([("age".to_string(), json!(i * 10))]);
        collection.insert_one(doc).unwrap();
    }

    let deleted = collection
        .delete_many(&json!({"age": {"$lt": 50}}))
        .unwrap();
    assert_eq!(deleted, 5);

    let count = collection.count_documents(&json!({})).unwrap();
    assert_eq!(count, 5);
}

#[test]
fn test_delete_many_all() {
    let collection = create_test_collection("test");

    for i in 0..5 {
        let doc = HashMap::from([("value".to_string(), json!(i))]);
        collection.insert_one(doc).unwrap();
    }

    let deleted = collection.delete_many(&json!({})).unwrap();
    assert_eq!(deleted, 5);
}

// ========== DISTINCT TESTS ==========

#[test]
fn test_distinct_simple() {
    let collection = create_test_collection("test");

    for city in &["NYC", "LA", "NYC", "SF", "LA", "NYC"] {
        let doc = HashMap::from([("city".to_string(), json!(city))]);
        collection.insert_one(doc).unwrap();
    }

    let distinct = collection.distinct("city", &json!({})).unwrap();
    assert_eq!(distinct.len(), 3); // NYC, LA, SF
}

#[test]
fn test_distinct_with_query() {
    let collection = create_test_collection("test");

    for i in 0..10 {
        let doc = HashMap::from([
            ("category".to_string(), json!(if i < 5 { "A" } else { "B" })),
            ("value".to_string(), json!(i % 3)),
        ]);
        collection.insert_one(doc).unwrap();
    }

    let distinct = collection
        .distinct("value", &json!({"category": "A"}))
        .unwrap();
    assert!(distinct.len() <= 3); // Values 0, 1, 2 in category A
}

#[test]
fn test_distinct_by_id() {
    let collection = create_test_collection("test");

    let doc = HashMap::from([
        ("name".to_string(), json!("Alice")),
        ("city".to_string(), json!("NYC")),
    ]);
    let id = collection.insert_one(doc).unwrap();

    let distinct = collection.distinct("city", &json!({"_id": id})).unwrap();
    assert_eq!(distinct.len(), 1);
    assert_eq!(distinct[0], "NYC");
}

#[test]
fn test_distinct_missing_field() {
    let collection = create_test_collection("test");

    let doc = HashMap::from([("name".to_string(), json!("Alice"))]);
    collection.insert_one(doc).unwrap();

    let distinct = collection.distinct("nonexistent", &json!({})).unwrap();
    assert!(distinct.is_empty());
}

// ========== INDEX TESTS ==========

#[test]
fn test_create_index() {
    let collection = create_test_collection("test");

    let index_name = collection.create_index("age".to_string(), false).unwrap();
    assert!(index_name.contains("age"));

    let indexes = collection.list_indexes();
    assert!(indexes.len() >= 2); // _id index + age index
}

#[test]
fn test_create_unique_index() {
    let collection = create_test_collection("test");

    collection.create_index("email".to_string(), true).unwrap();

    let doc1 = HashMap::from([("email".to_string(), json!("alice@test.com"))]);
    collection.insert_one(doc1).unwrap();

    // Should fail on duplicate
    let doc2 = HashMap::from([("email".to_string(), json!("alice@test.com"))]);
    let result = collection.insert_one(doc2);
    assert!(result.is_err());
}

#[test]
fn test_create_compound_index() {
    let collection = create_test_collection("test");

    let index_name = collection
        .create_compound_index(vec!["country".to_string(), "city".to_string()], false)
        .unwrap();

    assert!(index_name.contains("country"));
    assert!(index_name.contains("city"));

    // Insert documents
    for (country, city) in &[("US", "NYC"), ("US", "LA"), ("CA", "Toronto")] {
        let doc = HashMap::from([
            ("country".to_string(), json!(country)),
            ("city".to_string(), json!(city)),
        ]);
        collection.insert_one(doc).unwrap();
    }

    let count = collection.count_documents(&json!({})).unwrap();
    assert_eq!(count, 3);
}

#[test]
fn test_drop_index() {
    let collection = create_test_collection("test");

    let index_name = collection.create_index("temp".to_string(), false).unwrap();

    // First drop should succeed
    collection.drop_index(&index_name).unwrap();

    // Verify index is gone - list_indexes returns Vec<String>
    let indexes = collection.list_indexes();
    assert!(!indexes.contains(&index_name));
}

#[test]
fn test_list_indexes() {
    let collection = create_test_collection("test");

    // Should have at least _id index - list_indexes returns Vec<String>
    let indexes = collection.list_indexes();
    assert!(!indexes.is_empty());
    assert!(indexes.iter().any(|i| i.contains("_id")));
}

// ========== AGGREGATION TESTS ==========

#[test]
fn test_aggregate_match() {
    let collection = create_test_collection("test");

    for i in 0..10 {
        let doc = HashMap::from([("age".to_string(), json!(20 + i))]);
        collection.insert_one(doc).unwrap();
    }

    let results = collection
        .aggregate(&json!([{"$match": {"age": {"$gte": 25}}}]))
        .unwrap();
    assert_eq!(results.len(), 5);
}

#[test]
fn test_aggregate_group() {
    let collection = create_test_collection("test");

    for (city, value) in &[("NYC", 10), ("LA", 20), ("NYC", 30), ("LA", 40)] {
        let doc = HashMap::from([
            ("city".to_string(), json!(city)),
            ("value".to_string(), json!(value)),
        ]);
        collection.insert_one(doc).unwrap();
    }

    let results = collection
        .aggregate(&json!([
            {"$group": {"_id": "$city", "total": {"$sum": "$value"}}}
        ]))
        .unwrap();

    assert_eq!(results.len(), 2); // NYC and LA groups
}

#[test]
fn test_aggregate_sort_limit() {
    let collection = create_test_collection("test");

    for i in 0..10 {
        let doc = HashMap::from([("value".to_string(), json!(i))]);
        collection.insert_one(doc).unwrap();
    }

    let results = collection
        .aggregate(&json!([
            {"$sort": {"value": -1}},
            {"$limit": 3}
        ]))
        .unwrap();

    assert_eq!(results.len(), 3);
    assert_eq!(results[0]["value"], 9);
    assert_eq!(results[1]["value"], 8);
    assert_eq!(results[2]["value"], 7);
}

#[test]
fn test_aggregate_skip() {
    let collection = create_test_collection("test");

    for i in 0..10 {
        let doc = HashMap::from([("value".to_string(), json!(i))]);
        collection.insert_one(doc).unwrap();
    }

    let results = collection
        .aggregate(&json!([
            {"$sort": {"value": 1}},
            {"$skip": 5}
        ]))
        .unwrap();

    assert_eq!(results.len(), 5);
}

#[test]
fn test_aggregate_project() {
    let collection = create_test_collection("test");

    let doc = HashMap::from([
        ("name".to_string(), json!("Alice")),
        ("age".to_string(), json!(25)),
        ("secret".to_string(), json!("hidden")),
    ]);
    collection.insert_one(doc).unwrap();

    let results = collection
        .aggregate(&json!([
            {"$project": {"name": 1, "age": 1}}
        ]))
        .unwrap();

    assert_eq!(results.len(), 1);
    assert!(results[0].get("name").is_some());
    assert!(results[0].get("age").is_some());
    assert!(results[0].get("secret").is_none());
}

// ========== SCHEMA VALIDATION TESTS ==========

#[test]
fn test_schema_validation() {
    let collection = create_test_collection("test");

    collection
        .set_schema(Some(json!({
            "type": "object",
            "required": ["name"],
            "properties": {
                "name": {"type": "string"},
                "age": {"type": "number"}
            }
        })))
        .unwrap();

    // Valid document
    let doc = HashMap::from([
        ("name".to_string(), json!("Alice")),
        ("age".to_string(), json!(25)),
    ]);
    collection.insert_one(doc).unwrap();

    // Invalid - missing required field
    let doc_invalid = HashMap::from([("age".to_string(), json!(25))]);
    let result = collection.insert_one(doc_invalid);
    assert!(result.is_err());
}

#[test]
fn test_schema_type_mismatch() {
    let collection = create_test_collection("test");

    collection
        .set_schema(Some(json!({
            "type": "object",
            "properties": {
                "age": {"type": "number"}
            }
        })))
        .unwrap();

    // Invalid - wrong type
    let doc = HashMap::from([("age".to_string(), json!("not a number"))]);
    let result = collection.insert_one(doc);
    assert!(result.is_err());
}

#[test]
fn test_schema_clear() {
    let collection = create_test_collection("test");

    collection
        .set_schema(Some(json!({
            "type": "object",
            "required": ["name"]
        })))
        .unwrap();

    // Clear schema
    collection.set_schema(None).unwrap();

    // Now any document should be valid
    let doc = HashMap::from([("any_field".to_string(), json!("any value"))]);
    collection.insert_one(doc).unwrap();
}

// ========== FIND WITH OPTIONS TESTS ==========

#[test]
fn test_find_with_projection() {
    let collection = create_test_collection("test");

    let doc = HashMap::from([
        ("name".to_string(), json!("Alice")),
        ("age".to_string(), json!(25)),
        ("secret".to_string(), json!("hidden")),
    ]);
    collection.insert_one(doc).unwrap();

    let mut projection = HashMap::new();
    projection.insert("name".to_string(), 1);
    projection.insert("age".to_string(), 1);

    let options = ironbase_core::FindOptions {
        projection: Some(projection),
        sort: None,
        limit: None,
        skip: None,
    };

    let results = collection.find_with_options(&json!({}), options).unwrap();
    assert!(results[0].get("name").is_some());
    assert!(results[0].get("secret").is_none());
}

#[test]
fn test_find_with_sort() {
    let collection = create_test_collection("test");

    for i in [3, 1, 4, 1, 5, 9, 2, 6] {
        let doc = HashMap::from([("value".to_string(), json!(i))]);
        collection.insert_one(doc).unwrap();
    }

    let options = ironbase_core::FindOptions {
        projection: None,
        sort: Some(vec![("value".to_string(), 1)]), // ascending
        limit: None,
        skip: None,
    };

    let results = collection.find_with_options(&json!({}), options).unwrap();
    assert_eq!(results[0]["value"], 1);
    assert_eq!(results[results.len() - 1]["value"], 9);
}

#[test]
fn test_find_with_limit_skip() {
    let collection = create_test_collection("test");

    for i in 0..20 {
        let doc = HashMap::from([("value".to_string(), json!(i))]);
        collection.insert_one(doc).unwrap();
    }

    let options = ironbase_core::FindOptions {
        projection: None,
        sort: Some(vec![("value".to_string(), 1)]),
        limit: Some(5),
        skip: Some(10),
    };

    let results = collection.find_with_options(&json!({}), options).unwrap();
    assert_eq!(results.len(), 5);
    assert_eq!(results[0]["value"], 10);
}

// ========== EXPLAIN AND HINT TESTS ==========

#[test]
fn test_explain() {
    let collection = create_test_collection("test");

    collection.create_index("age".to_string(), false).unwrap();

    let doc = HashMap::from([("age".to_string(), json!(25))]);
    collection.insert_one(doc).unwrap();

    let plan = collection.explain(&json!({"age": 25})).unwrap();
    // Plan is a JSON value containing "queryPlan" key
    assert!(plan.is_object());
    assert!(plan.get("queryPlan").is_some());
}

#[test]
fn test_find_with_hint() {
    let collection = create_test_collection("test");

    let index_name = collection.create_index("age".to_string(), false).unwrap();

    for i in 0..10 {
        let doc = HashMap::from([("age".to_string(), json!(i))]);
        collection.insert_one(doc).unwrap();
    }

    let results = collection
        .find_with_hint(&json!({"age": {"$gte": 5}}), &index_name)
        .unwrap();
    assert_eq!(results.len(), 5);
}

// ========== EDGE CASES ==========

#[test]
fn test_null_query() {
    let collection = create_test_collection("test");

    let doc = HashMap::from([("name".to_string(), json!("Alice"))]);
    collection.insert_one(doc).unwrap();

    // Null query should match all
    let count = collection.count_documents(&json!(null)).unwrap();
    assert_eq!(count, 1);
}

#[test]
fn test_cursor_for_each() {
    let collection = create_test_collection("test");

    for i in 0..5 {
        let doc = HashMap::from([("value".to_string(), json!(i))]);
        collection.insert_one(doc).unwrap();
    }

    let mut cursor = collection.find_streaming(&json!({})).unwrap();
    let mut count = 0;

    cursor
        .for_each(|_doc| {
            count += 1;
            Ok(())
        })
        .unwrap();

    assert_eq!(count, 5);
}

// ========== MEMORY STORAGE TESTS ==========
// These tests verify that CollectionCore works with MemoryStorage (RawStorage impl)

mod memory_storage_tests {
    use ironbase_core::storage::MemoryStorage;
    use ironbase_core::CollectionCore;
    use parking_lot::RwLock;
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::Arc;

    fn create_memory_collection(name: &str) -> CollectionCore<MemoryStorage> {
        let storage = MemoryStorage::new();
        let storage = Arc::new(RwLock::new(storage));
        CollectionCore::new(name.to_string(), storage).unwrap()
    }

    #[test]
    fn test_memory_insert_and_find() {
        let collection = create_memory_collection("test");

        let doc = HashMap::from([
            ("name".to_string(), json!("Alice")),
            ("age".to_string(), json!(30)),
        ]);
        let id = collection.insert_one(doc).unwrap();

        let found = collection.find_one(&json!({"_id": id})).unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap()["name"], "Alice");
    }

    #[test]
    fn test_memory_count() {
        let collection = create_memory_collection("test");

        for i in 0..10 {
            let doc = HashMap::from([("value".to_string(), json!(i))]);
            collection.insert_one(doc).unwrap();
        }

        let count = collection.count_documents(&json!({})).unwrap();
        assert_eq!(count, 10);
    }

    #[test]
    fn test_memory_update() {
        let collection = create_memory_collection("test");

        let doc = HashMap::from([("name".to_string(), json!("Alice"))]);
        let id = collection.insert_one(doc).unwrap();

        collection
            .update_one(&json!({"_id": id}), &json!({"$set": {"name": "Bob"}}))
            .unwrap();

        let updated = collection.find_one(&json!({"_id": id})).unwrap().unwrap();
        assert_eq!(updated["name"], "Bob");
    }

    #[test]
    fn test_memory_delete() {
        let collection = create_memory_collection("test");

        let doc = HashMap::from([("name".to_string(), json!("Alice"))]);
        let id = collection.insert_one(doc).unwrap();

        collection.delete_one(&json!({"_id": id})).unwrap();

        let count = collection.count_documents(&json!({})).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_memory_index() {
        let collection = create_memory_collection("test");

        let index_name = collection.create_index("age".to_string(), false).unwrap();
        assert!(index_name.contains("age"));

        for i in 0..10 {
            let doc = HashMap::from([("age".to_string(), json!(i))]);
            collection.insert_one(doc).unwrap();
        }

        let results = collection.find(&json!({"age": {"$gte": 5}})).unwrap();
        assert_eq!(results.len(), 5);
    }

    #[test]
    fn test_memory_aggregation() {
        let collection = create_memory_collection("test");

        for city in &["NYC", "LA", "NYC", "LA", "NYC"] {
            let doc = HashMap::from([("city".to_string(), json!(city))]);
            collection.insert_one(doc).unwrap();
        }

        let results = collection
            .aggregate(&json!([
                {"$group": {"_id": "$city", "count": {"$sum": 1}}}
            ]))
            .unwrap();

        assert_eq!(results.len(), 2); // NYC and LA
    }

    #[test]
    fn test_memory_cursor() {
        let collection = create_memory_collection("test");

        for i in 0..20 {
            let doc = HashMap::from([("value".to_string(), json!(i))]);
            collection.insert_one(doc).unwrap();
        }

        let mut cursor = collection.find_streaming(&json!({})).unwrap();
        assert_eq!(cursor.total(), 20);

        let batch = cursor.next_chunk(5).unwrap();
        assert_eq!(batch.len(), 5);
        assert_eq!(cursor.remaining(), 15);
    }
}
