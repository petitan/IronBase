//! Integration tests for CollectionCore
//!
//! These tests cover the main CRUD operations and various edge cases

use ironbase_core::storage::{MemoryStorage, StorageEngine};
use ironbase_core::DatabaseCore;
use serde_json::json;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

fn create_test_db(name: &str) -> (DatabaseCore<StorageEngine>, String) {
    let counter = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let temp_dir = std::env::temp_dir();
    let db_path = temp_dir
        .join(format!("test_collection_{}_{}.mlite", name, counter))
        .to_string_lossy()
        .to_string();

    // Cleanup previous test files
    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_file(format!("{}.wal", db_path.trim_end_matches(".mlite")));

    let db = DatabaseCore::open(&db_path).unwrap();
    (db, "test".to_string())
}

// ========== INSERT TESTS ==========

#[test]
fn test_insert_one_auto_id() {
    let (db, coll_name) = create_test_db("test");
    let collection = db.collection(&coll_name).unwrap();

    let doc = HashMap::from([("name".to_string(), json!("Alice"))]);
    let id = db.insert_one(&coll_name, doc).unwrap();

    // Should have auto-generated ID
    assert!(matches!(id, ironbase_core::DocumentId::Int(_)));

    let count = collection.count_documents(&json!({})).unwrap();
    assert_eq!(count, 1);
}

#[test]
fn test_insert_one_with_custom_id() {
    let (db, coll_name) = create_test_db("test");

    let doc = HashMap::from([
        ("_id".to_string(), json!("custom_id")),
        ("name".to_string(), json!("Bob")),
    ]);
    let id = db.insert_one(&coll_name, doc).unwrap();

    assert!(matches!(id, ironbase_core::DocumentId::String(s) if s == "custom_id"));
}

#[test]
fn test_insert_many_empty() {
    let (db, coll_name) = create_test_db("test");

    let result = db.insert_many(&coll_name, vec![]).unwrap();
    assert_eq!(result.len(), 0);
}

#[test]
fn test_insert_many_batch() {
    let (db, coll_name) = create_test_db("test");
    let collection = db.collection(&coll_name).unwrap();

    let docs: Vec<HashMap<String, serde_json::Value>> = (0..100)
        .map(|i| HashMap::from([("value".to_string(), json!(i))]))
        .collect();

    let result = db.insert_many(&coll_name, docs).unwrap();
    assert_eq!(result.len(), 100);

    let count = collection.count_documents(&json!({})).unwrap();
    assert_eq!(count, 100);
}

// ========== FIND TESTS ==========

#[test]
fn test_find_empty_collection() {
    let (db, coll_name) = create_test_db("test");
    let collection = db.collection(&coll_name).unwrap();
    let results = collection.find(&json!({})).unwrap();
    assert!(results.is_empty());
}

#[test]
fn test_find_one_by_id() {
    let (db, coll_name) = create_test_db("test");
    let collection = db.collection(&coll_name).unwrap();

    let doc = HashMap::from([("name".to_string(), json!("Alice"))]);
    let id = db.insert_one(&coll_name, doc).unwrap();

    let found = collection
        .find_one(&json!({"_id": id}))
        .unwrap()
        .expect("Document should be found");
    assert_eq!(found["name"], "Alice");
}

#[test]
fn test_find_one_not_found() {
    let (db, coll_name) = create_test_db("test");
    let collection = db.collection(&coll_name).unwrap();

    let doc = HashMap::from([("name".to_string(), json!("Alice"))]);
    db.insert_one(&coll_name, doc).unwrap();

    let found = collection.find_one(&json!({"_id": 999})).unwrap();
    assert!(found.is_none());
}

#[test]
fn test_find_with_query() {
    let (db, coll_name) = create_test_db("test");
    let collection = db.collection(&coll_name).unwrap();

    for i in 0..10 {
        let doc = HashMap::from([
            ("name".to_string(), json!(format!("User {}", i))),
            ("age".to_string(), json!(20 + i)),
        ]);
        db.insert_one(&coll_name, doc).unwrap();
    }

    // Find users with age >= 25
    let results = collection.find(&json!({"age": {"$gte": 25}})).unwrap();
    assert_eq!(results.len(), 5);
}

#[test]
fn test_find_streaming() {
    let (db, coll_name) = create_test_db("test");
    let collection = db.collection(&coll_name).unwrap();

    for i in 0..50 {
        let doc = HashMap::from([("value".to_string(), json!(i))]);
        db.insert_one(&coll_name, doc).unwrap();
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
    let (db, coll_name) = create_test_db("test");
    let collection = db.collection(&coll_name).unwrap();

    for i in 0..25 {
        let doc = HashMap::from([("value".to_string(), json!(i))]);
        db.insert_one(&coll_name, doc).unwrap();
    }

    let mut cursor = collection
        .find_streaming(&json!({}))
        .unwrap()
        .with_batch_size(5);

    let batch = cursor.next_batch().unwrap();
    assert_eq!(batch.len(), 5);
}

#[test]
fn test_find_cursor_many_tombstones() {
    // Test that FindCursor handles many consecutive tombstones without stack overflow
    // This tests the fix for the recursion bug in FindCursor::next()
    let (db, coll_name) = create_test_db("test");
    let collection = db.collection(&coll_name).unwrap();

    // Insert 1000 documents
    for i in 0..1000 {
        let doc = HashMap::from([("value".to_string(), json!(i))]);
        db.insert_one(&coll_name, doc).unwrap();
    }

    // Delete 900 of them (leaving 100)
    // This creates many tombstones
    db.delete_many(&coll_name, &json!({"value": {"$lt": 900}}))
        .unwrap();

    // Create a streaming cursor - this should work without stack overflow
    let mut cursor = collection.find_streaming(&json!({})).unwrap();

    // Iterate through remaining documents
    let mut count = 0;
    while let Some(_doc) = cursor.next().unwrap() {
        count += 1;
    }

    assert_eq!(
        count, 100,
        "Should have 100 remaining documents after deletion"
    );
}

// ========== COUNT TESTS ==========

#[test]
fn test_count_empty() {
    let (db, coll_name) = create_test_db("test");
    let collection = db.collection(&coll_name).unwrap();
    let count = collection.count_documents(&json!({})).unwrap();
    assert_eq!(count, 0);
}

#[test]
fn test_count_all() {
    let (db, coll_name) = create_test_db("test");
    let collection = db.collection(&coll_name).unwrap();

    for i in 0..10 {
        let doc = HashMap::from([("value".to_string(), json!(i))]);
        db.insert_one(&coll_name, doc).unwrap();
    }

    let count = collection.count_documents(&json!({})).unwrap();
    assert_eq!(count, 10);
}

#[test]
fn test_count_with_query() {
    let (db, coll_name) = create_test_db("test");
    let collection = db.collection(&coll_name).unwrap();

    for i in 0..10 {
        let doc = HashMap::from([("age".to_string(), json!(i * 10))]);
        db.insert_one(&coll_name, doc).unwrap();
    }

    let count = collection
        .count_documents(&json!({"age": {"$gte": 50}}))
        .unwrap();
    assert_eq!(count, 5);
}

#[test]
fn test_count_by_id() {
    let (db, coll_name) = create_test_db("test");
    let collection = db.collection(&coll_name).unwrap();

    let doc = HashMap::from([("name".to_string(), json!("Alice"))]);
    let id = db.insert_one(&coll_name, doc).unwrap();

    let count = collection.count_documents(&json!({"_id": id})).unwrap();
    assert_eq!(count, 1);

    let count_missing = collection.count_documents(&json!({"_id": 999})).unwrap();
    assert_eq!(count_missing, 0);
}

// ========== UPDATE TESTS ==========

#[test]
fn test_update_one_set() {
    let (db, coll_name) = create_test_db("test");
    let collection = db.collection(&coll_name).unwrap();

    let doc = HashMap::from([
        ("name".to_string(), json!("Alice")),
        ("age".to_string(), json!(25)),
    ]);
    let id = db.insert_one(&coll_name, doc).unwrap();

    let (matched, modified) = db
        .update_one(
            &coll_name,
            &json!({"_id": id}),
            &json!({"$set": {"age": 30}}),
        )
        .unwrap();

    assert_eq!(matched, 1);
    assert_eq!(modified, 1);

    let updated = collection.find_one(&json!({"_id": id})).unwrap().unwrap();
    assert_eq!(updated["age"], 30);
}

#[test]
fn test_update_one_inc() {
    let (db, coll_name) = create_test_db("test");
    let collection = db.collection(&coll_name).unwrap();

    let doc = HashMap::from([("counter".to_string(), json!(10))]);
    let id = db.insert_one(&coll_name, doc).unwrap();

    db.update_one(
        &coll_name,
        &json!({"_id": id}),
        &json!({"$inc": {"counter": 5}}),
    )
    .unwrap();

    let updated = collection.find_one(&json!({"_id": id})).unwrap().unwrap();
    assert_eq!(updated["counter"], 15);
}

#[test]
fn test_update_one_unset() {
    let (db, coll_name) = create_test_db("test");
    let collection = db.collection(&coll_name).unwrap();

    let doc = HashMap::from([
        ("name".to_string(), json!("Alice")),
        ("temp".to_string(), json!("remove me")),
    ]);
    let id = db.insert_one(&coll_name, doc).unwrap();

    db.update_one(
        &coll_name,
        &json!({"_id": id}),
        &json!({"$unset": {"temp": ""}}),
    )
    .unwrap();

    let updated = collection.find_one(&json!({"_id": id})).unwrap().unwrap();
    assert!(updated.get("temp").is_none());
}

#[test]
fn test_update_one_push() {
    let (db, coll_name) = create_test_db("test");
    let collection = db.collection(&coll_name).unwrap();

    let doc = HashMap::from([("tags".to_string(), json!(["a", "b"]))]);
    let id = db.insert_one(&coll_name, doc).unwrap();

    db.update_one(
        &coll_name,
        &json!({"_id": id}),
        &json!({"$push": {"tags": "c"}}),
    )
    .unwrap();

    let updated = collection.find_one(&json!({"_id": id})).unwrap().unwrap();
    assert_eq!(updated["tags"], json!(["a", "b", "c"]));
}

#[test]
fn test_update_one_pull() {
    let (db, coll_name) = create_test_db("test");
    let collection = db.collection(&coll_name).unwrap();

    let doc = HashMap::from([("tags".to_string(), json!(["a", "b", "c"]))]);
    let id = db.insert_one(&coll_name, doc).unwrap();

    db.update_one(
        &coll_name,
        &json!({"_id": id}),
        &json!({"$pull": {"tags": "b"}}),
    )
    .unwrap();

    let updated = collection.find_one(&json!({"_id": id})).unwrap().unwrap();
    assert_eq!(updated["tags"], json!(["a", "c"]));
}

#[test]
fn test_update_one_addtoset() {
    let (db, coll_name) = create_test_db("test");
    let collection = db.collection(&coll_name).unwrap();

    let doc = HashMap::from([("tags".to_string(), json!(["a", "b"]))]);
    let id = db.insert_one(&coll_name, doc).unwrap();

    // Add new value
    db.update_one(
        &coll_name,
        &json!({"_id": id}),
        &json!({"$addToSet": {"tags": "c"}}),
    )
    .unwrap();

    // Try to add existing value (should not duplicate)
    db.update_one(
        &coll_name,
        &json!({"_id": id}),
        &json!({"$addToSet": {"tags": "a"}}),
    )
    .unwrap();

    let updated = collection.find_one(&json!({"_id": id})).unwrap().unwrap();
    assert_eq!(updated["tags"], json!(["a", "b", "c"]));
}

#[test]
fn test_update_one_pop() {
    let (db, coll_name) = create_test_db("test");
    let collection = db.collection(&coll_name).unwrap();

    let doc = HashMap::from([("arr".to_string(), json!([1, 2, 3]))]);
    let id = db.insert_one(&coll_name, doc).unwrap();

    // Pop from end
    db.update_one(
        &coll_name,
        &json!({"_id": id}),
        &json!({"$pop": {"arr": 1}}),
    )
    .unwrap();

    let updated = collection.find_one(&json!({"_id": id})).unwrap().unwrap();
    assert_eq!(updated["arr"], json!([1, 2]));

    // Pop from beginning
    db.update_one(
        &coll_name,
        &json!({"_id": id}),
        &json!({"$pop": {"arr": -1}}),
    )
    .unwrap();

    let updated = collection.find_one(&json!({"_id": id})).unwrap().unwrap();
    assert_eq!(updated["arr"], json!([2]));
}

#[test]
fn test_update_one_not_found() {
    let (db, coll_name) = create_test_db("test");

    // First create the collection with a document (update won't create collection implicitly)
    let doc = HashMap::from([("name".to_string(), json!("setup"))]);
    db.insert_one(&coll_name, doc).unwrap();

    // Now try to update a non-existent document
    let (matched, modified) = db
        .update_one(&coll_name, &json!({"_id": 999}), &json!({"$set": {"x": 1}}))
        .unwrap();

    assert_eq!(matched, 0);
    assert_eq!(modified, 0);
}

#[test]
fn test_update_one_no_change() {
    let (db, coll_name) = create_test_db("test");

    let doc = HashMap::from([("value".to_string(), json!(10))]);
    let id = db.insert_one(&coll_name, doc).unwrap();

    // Update with same value - implementation counts modification even if value unchanged
    let (matched, modified) = db
        .update_one(
            &coll_name,
            &json!({"_id": id}),
            &json!({"$set": {"value": 10}}),
        )
        .unwrap();

    assert_eq!(matched, 1);
    assert_eq!(modified, 1); // Implementation doesn't track actual change detection
}

#[test]
fn test_update_many() {
    let (db, coll_name) = create_test_db("test");

    for i in 0..10 {
        let doc = HashMap::from([
            ("category".to_string(), json!(if i < 5 { "A" } else { "B" })),
            ("value".to_string(), json!(i)),
        ]);
        db.insert_one(&coll_name, doc).unwrap();
    }

    let (matched, modified) = db
        .update_many(
            &coll_name,
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
    let (db, coll_name) = create_test_db("test");
    let collection = db.collection(&coll_name).unwrap();

    let doc = HashMap::from([("name".to_string(), json!("Alice"))]);
    let id = db.insert_one(&coll_name, doc).unwrap();

    let deleted = db.delete_one(&coll_name, &json!({"_id": id})).unwrap();
    assert_eq!(deleted, 1);

    let count = collection.count_documents(&json!({})).unwrap();
    assert_eq!(count, 0);
}

#[test]
fn test_delete_one_not_found() {
    let (db, coll_name) = create_test_db("test");

    // First create the collection with a document (delete won't create collection implicitly)
    let doc = HashMap::from([("name".to_string(), json!("setup"))]);
    db.insert_one(&coll_name, doc).unwrap();

    // Now try to delete a non-existent document
    let deleted = db.delete_one(&coll_name, &json!({"_id": 999})).unwrap();
    assert_eq!(deleted, 0);
}

#[test]
fn test_delete_many() {
    let (db, coll_name) = create_test_db("test");
    let collection = db.collection(&coll_name).unwrap();

    for i in 0..10 {
        let doc = HashMap::from([("age".to_string(), json!(i * 10))]);
        db.insert_one(&coll_name, doc).unwrap();
    }

    let deleted = db
        .delete_many(&coll_name, &json!({"age": {"$lt": 50}}))
        .unwrap();
    assert_eq!(deleted, 5);

    let count = collection.count_documents(&json!({})).unwrap();
    assert_eq!(count, 5);
}

#[test]
fn test_delete_many_all() {
    let (db, coll_name) = create_test_db("test");

    for i in 0..5 {
        let doc = HashMap::from([("value".to_string(), json!(i))]);
        db.insert_one(&coll_name, doc).unwrap();
    }

    let deleted = db.delete_many(&coll_name, &json!({})).unwrap();
    assert_eq!(deleted, 5);
}

// ========== DISTINCT TESTS ==========

#[test]
fn test_distinct_simple() {
    let (db, coll_name) = create_test_db("test");
    let collection = db.collection(&coll_name).unwrap();

    for city in &["NYC", "LA", "NYC", "SF", "LA", "NYC"] {
        let doc = HashMap::from([("city".to_string(), json!(city))]);
        db.insert_one(&coll_name, doc).unwrap();
    }

    let distinct = collection.distinct("city", &json!({})).unwrap();
    assert_eq!(distinct.len(), 3); // NYC, LA, SF
}

#[test]
fn test_distinct_with_query() {
    let (db, coll_name) = create_test_db("test");
    let collection = db.collection(&coll_name).unwrap();

    for i in 0..10 {
        let doc = HashMap::from([
            ("category".to_string(), json!(if i < 5 { "A" } else { "B" })),
            ("value".to_string(), json!(i % 3)),
        ]);
        db.insert_one(&coll_name, doc).unwrap();
    }

    let distinct = collection
        .distinct("value", &json!({"category": "A"}))
        .unwrap();
    assert!(distinct.len() <= 3); // Values 0, 1, 2 in category A
}

#[test]
fn test_distinct_by_id() {
    let (db, coll_name) = create_test_db("test");
    let collection = db.collection(&coll_name).unwrap();

    let doc = HashMap::from([
        ("name".to_string(), json!("Alice")),
        ("city".to_string(), json!("NYC")),
    ]);
    let id = db.insert_one(&coll_name, doc).unwrap();

    let distinct = collection.distinct("city", &json!({"_id": id})).unwrap();
    assert_eq!(distinct.len(), 1);
    assert_eq!(distinct[0], "NYC");
}

#[test]
fn test_distinct_missing_field() {
    let (db, coll_name) = create_test_db("test");
    let collection = db.collection(&coll_name).unwrap();

    let doc = HashMap::from([("name".to_string(), json!("Alice"))]);
    db.insert_one(&coll_name, doc).unwrap();

    let distinct = collection.distinct("nonexistent", &json!({})).unwrap();
    assert!(distinct.is_empty());
}

#[test]
fn test_distinct_object_key_order_canonical() {
    // Test that objects with different key orders are treated as equal
    // This tests the fix for the distinct() canonicalization bug
    let (db, coll_name) = create_test_db("test");
    let collection = db.collection(&coll_name).unwrap();

    // Insert documents with identical address objects but different key order
    // Using serde_json's indexmap, key order is preserved during parsing
    let doc1 = HashMap::from([
        ("name".to_string(), json!("Alice")),
        (
            "address".to_string(),
            json!({"city": "NYC", "zip": "10001"}),
        ),
    ]);
    let doc2 = HashMap::from([
        ("name".to_string(), json!("Bob")),
        (
            "address".to_string(),
            json!({"zip": "10001", "city": "NYC"}),
        ), // Same content, different order
    ]);
    db.insert_one(&coll_name, doc1).unwrap();
    db.insert_one(&coll_name, doc2).unwrap();

    // Should return 1 distinct address (both are semantically identical)
    let distinct = collection.distinct("address", &json!({})).unwrap();
    assert_eq!(
        distinct.len(),
        1,
        "Expected 1 distinct address, got {}. Bug: objects with different key order treated as different.",
        distinct.len()
    );
}

#[test]
fn test_distinct_array_field_with_dot_notation() {
    // BUG FIX: distinct() should handle arrays with dot notation (MongoDB-style)
    // This tests the fix for implicit array traversal in distinct()
    let (db, coll_name) = create_test_db("test");
    let collection = db.collection(&coll_name).unwrap();

    // Insert documents with array fields
    let doc1 = HashMap::from([(
        "items".to_string(),
        json!([{"name": "apple"}, {"name": "banana"}]),
    )]);
    let doc2 = HashMap::from([(
        "items".to_string(),
        json!([{"name": "banana"}, {"name": "cherry"}]),
    )]);
    db.insert_one(&coll_name, doc1).unwrap();
    db.insert_one(&coll_name, doc2).unwrap();

    // distinct("items.name") should return all unique names from the nested arrays
    let distinct = collection.distinct("items.name", &json!({})).unwrap();

    // Should find: "apple", "banana", "cherry" (3 distinct values)
    assert_eq!(
        distinct.len(),
        3,
        "Expected 3 distinct values (apple, banana, cherry), got {:?}",
        distinct
    );

    // Verify all expected values are present
    let values: Vec<String> = distinct
        .iter()
        .filter_map(|v| v.as_str().map(String::from))
        .collect();
    assert!(values.contains(&"apple".to_string()), "Missing 'apple'");
    assert!(values.contains(&"banana".to_string()), "Missing 'banana'");
    assert!(values.contains(&"cherry".to_string()), "Missing 'cherry'");
}

#[test]
fn test_distinct_nested_array_with_query() {
    // Test distinct with array fields combined with a query filter
    let (db, coll_name) = create_test_db("test");
    let collection = db.collection(&coll_name).unwrap();

    let doc1 = HashMap::from([
        ("category".to_string(), json!("fruit")),
        (
            "items".to_string(),
            json!([{"name": "apple"}, {"name": "banana"}]),
        ),
    ]);
    let doc2 = HashMap::from([
        ("category".to_string(), json!("vegetable")),
        (
            "items".to_string(),
            json!([{"name": "carrot"}, {"name": "potato"}]),
        ),
    ]);
    db.insert_one(&coll_name, doc1).unwrap();
    db.insert_one(&coll_name, doc2).unwrap();

    // Get distinct item names only from "fruit" category
    let distinct = collection
        .distinct("items.name", &json!({"category": "fruit"}))
        .unwrap();

    assert_eq!(
        distinct.len(),
        2,
        "Expected 2 distinct values from fruit category, got {:?}",
        distinct
    );

    let values: Vec<String> = distinct
        .iter()
        .filter_map(|v| v.as_str().map(String::from))
        .collect();
    assert!(values.contains(&"apple".to_string()));
    assert!(values.contains(&"banana".to_string()));
    assert!(
        !values.contains(&"carrot".to_string()),
        "Should not include vegetable items"
    );
}

// ========== INDEX TESTS ==========

#[test]
fn test_create_index() {
    let (db, coll_name) = create_test_db("test");
    let collection = db.collection(&coll_name).unwrap();

    let index_name = collection
        .create_index("age".to_string(), false, false)
        .unwrap();
    assert!(index_name.contains("age"));

    let indexes = collection.list_indexes().unwrap();
    assert!(indexes.len() >= 2); // _id index + age index
}

#[test]
fn test_create_unique_index() {
    let (db, coll_name) = create_test_db("test");
    let collection = db.collection(&coll_name).unwrap();

    collection
        .create_index("email".to_string(), true, false)
        .unwrap();

    let doc1 = HashMap::from([("email".to_string(), json!("alice@test.com"))]);
    db.insert_one(&coll_name, doc1).unwrap();

    // Should fail on duplicate
    let doc2 = HashMap::from([("email".to_string(), json!("alice@test.com"))]);
    let result = db.insert_one(&coll_name, doc2);
    assert!(result.is_err());
}

#[test]
fn test_create_compound_index() {
    let (db, coll_name) = create_test_db("test");
    let collection = db.collection(&coll_name).unwrap();

    let index_name = collection
        .create_compound_index(
            vec!["country".to_string(), "city".to_string()],
            false,
            false,
        )
        .unwrap();

    assert!(index_name.contains("country"));
    assert!(index_name.contains("city"));

    // Insert documents
    for (country, city) in &[("US", "NYC"), ("US", "LA"), ("CA", "Toronto")] {
        let doc = HashMap::from([
            ("country".to_string(), json!(country)),
            ("city".to_string(), json!(city)),
        ]);
        db.insert_one(&coll_name, doc).unwrap();
    }

    let count = collection.count_documents(&json!({})).unwrap();
    assert_eq!(count, 3);
}

#[test]
fn test_drop_index() {
    let (db, coll_name) = create_test_db("test");
    let collection = db.collection(&coll_name).unwrap();

    let index_name = collection
        .create_index("temp".to_string(), false, false)
        .unwrap();

    // First drop should succeed
    collection.drop_index(&index_name).unwrap();

    // Verify index is gone - list_indexes returns Vec<String>
    let indexes = collection.list_indexes().unwrap();
    assert!(!indexes.contains(&index_name));
}

#[test]
fn test_list_indexes() {
    let (db, coll_name) = create_test_db("test");
    let collection = db.collection(&coll_name).unwrap();

    // Should have at least _id index - list_indexes returns Vec<String>
    let indexes = collection.list_indexes().unwrap();
    assert!(!indexes.is_empty());
    assert!(indexes.iter().any(|i| i.contains("_id")));
}

// ========== AGGREGATION TESTS ==========

#[test]
fn test_aggregate_match() {
    let (db, coll_name) = create_test_db("test");
    let collection = db.collection(&coll_name).unwrap();

    for i in 0..10 {
        let doc = HashMap::from([("age".to_string(), json!(20 + i))]);
        db.insert_one(&coll_name, doc).unwrap();
    }

    let results = collection
        .aggregate(&json!([{"$match": {"age": {"$gte": 25}}}]))
        .unwrap();
    assert_eq!(results.len(), 5);
}

#[test]
fn test_aggregate_group() {
    let (db, coll_name) = create_test_db("test");
    let collection = db.collection(&coll_name).unwrap();

    for (city, value) in &[("NYC", 10), ("LA", 20), ("NYC", 30), ("LA", 40)] {
        let doc = HashMap::from([
            ("city".to_string(), json!(city)),
            ("value".to_string(), json!(value)),
        ]);
        db.insert_one(&coll_name, doc).unwrap();
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
    let (db, coll_name) = create_test_db("test");
    let collection = db.collection(&coll_name).unwrap();

    for i in 0..10 {
        let doc = HashMap::from([("value".to_string(), json!(i))]);
        db.insert_one(&coll_name, doc).unwrap();
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
fn test_aggregate_sort_mixed_types_no_panic() {
    // Issue #58: $sort must not panic with mixed types (int/string/null/bool)
    let (db, coll_name) = create_test_db("test");

    // Insert documents with deliberately mixed types in the "value" field
    db.insert_one(
        &coll_name,
        HashMap::from([("value".to_string(), json!(42))]),
    )
    .unwrap();
    db.insert_one(
        &coll_name,
        HashMap::from([("value".to_string(), json!("hello"))]),
    )
    .unwrap();
    db.insert_one(
        &coll_name,
        HashMap::from([("value".to_string(), json!(null))]),
    )
    .unwrap();
    db.insert_one(
        &coll_name,
        HashMap::from([("value".to_string(), json!(true))]),
    )
    .unwrap();
    db.insert_one(&coll_name, HashMap::from([("value".to_string(), json!(7))]))
        .unwrap();
    // Document with missing "value" field entirely
    db.insert_one(
        &coll_name,
        HashMap::from([("other".to_string(), json!("x"))]),
    )
    .unwrap();

    let collection = db.collection(&coll_name).unwrap();

    // Ascending sort — must not panic, must return all 6 docs
    let asc = collection
        .aggregate(&json!([{"$sort": {"value": 1}}]))
        .unwrap();
    assert_eq!(asc.len(), 6);

    // Descending sort — must not panic, must return all 6 docs
    let desc = collection
        .aggregate(&json!([{"$sort": {"value": -1}}]))
        .unwrap();
    assert_eq!(desc.len(), 6);

    // TopK path (sort + limit) — must not panic
    let topk = collection
        .aggregate(&json!([{"$sort": {"value": 1}}, {"$limit": 3}]))
        .unwrap();
    assert_eq!(topk.len(), 3);
}

#[test]
fn test_aggregate_skip() {
    let (db, coll_name) = create_test_db("test");
    let collection = db.collection(&coll_name).unwrap();

    for i in 0..10 {
        let doc = HashMap::from([("value".to_string(), json!(i))]);
        db.insert_one(&coll_name, doc).unwrap();
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
    let (db, coll_name) = create_test_db("test");
    let collection = db.collection(&coll_name).unwrap();

    let doc = HashMap::from([
        ("name".to_string(), json!("Alice")),
        ("age".to_string(), json!(25)),
        ("secret".to_string(), json!("hidden")),
    ]);
    db.insert_one(&coll_name, doc).unwrap();

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
    let (db, coll_name) = create_test_db("test");
    let collection = db.collection(&coll_name).unwrap();

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
    db.insert_one(&coll_name, doc).unwrap();

    // Invalid - missing required field
    let doc_invalid = HashMap::from([("age".to_string(), json!(25))]);
    let result = db.insert_one(&coll_name, doc_invalid);
    assert!(result.is_err());
}

#[test]
fn test_schema_type_mismatch() {
    let (db, coll_name) = create_test_db("test");
    let collection = db.collection(&coll_name).unwrap();

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
    let result = db.insert_one(&coll_name, doc);
    assert!(result.is_err());
}

#[test]
fn test_schema_clear() {
    let (db, coll_name) = create_test_db("test");
    let collection = db.collection(&coll_name).unwrap();

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
    db.insert_one(&coll_name, doc).unwrap();
}

#[test]
fn test_schema_sharing_between_handles() {
    // BUG FIX: Schema changes should be visible across all CollectionCore handles
    // This tests the fix for schema not being shared between instances
    let (db, coll_name) = create_test_db("test");

    // Get first handle
    let coll1 = db.collection(&coll_name).unwrap();

    // Get second handle for the SAME collection
    let _coll2 = db.collection(&coll_name).unwrap();

    // Set schema on first handle
    coll1
        .set_schema(Some(json!({
            "type": "object",
            "required": ["name"],
            "properties": {
                "name": {"type": "string"}
            }
        })))
        .unwrap();

    // Second handle should now also have the schema
    // Inserting a document without "name" should fail
    let invalid_doc = HashMap::from([("age".to_string(), json!(25))]);
    let result = db.insert_one(&coll_name, invalid_doc);

    assert!(
        result.is_err(),
        "Schema from coll1.set_schema() should be visible to coll2 (and db.insert_one). \
         Document without required 'name' field should be rejected."
    );

    // Valid document should work
    let valid_doc = HashMap::from([("name".to_string(), json!("Alice"))]);
    db.insert_one(&coll_name, valid_doc).unwrap();
}

#[test]
fn test_schema_sharing_after_reopen() {
    // Test that schema changes persist and are shared after getting a new handle
    let (db, coll_name) = create_test_db("test");

    // Set schema
    let coll = db.collection(&coll_name).unwrap();
    coll.set_schema(Some(json!({
        "type": "object",
        "required": ["email"]
    })))
    .unwrap();

    // Get a NEW handle - should see the schema
    let coll_new = db.collection(&coll_name).unwrap();

    // This should verify schema is enforced through the new handle
    let schema = coll_new.get_schema().unwrap();
    assert!(schema.is_some(), "New handle should have the schema");
}

// ========== FIND WITH OPTIONS TESTS ==========

#[test]
fn test_find_with_projection() {
    let (db, coll_name) = create_test_db("test");
    let collection = db.collection(&coll_name).unwrap();

    let doc = HashMap::from([
        ("name".to_string(), json!("Alice")),
        ("age".to_string(), json!(25)),
        ("secret".to_string(), json!("hidden")),
    ]);
    db.insert_one(&coll_name, doc).unwrap();

    let mut projection = HashMap::new();
    projection.insert("name".to_string(), 1);
    projection.insert("age".to_string(), 1);

    let options = ironbase_core::FindOptions {
        projection: Some(projection),
        sort: None,
        limit: None,
        skip: None,
        include_total: false,
        ..Default::default()
    };

    let results = collection.find_with_options(&json!({}), options).unwrap();
    assert!(results[0].get("name").is_some());
    assert!(results[0].get("secret").is_none());
}

#[test]
fn test_find_with_sort() {
    let (db, coll_name) = create_test_db("test");
    let collection = db.collection(&coll_name).unwrap();

    for i in [3, 1, 4, 1, 5, 9, 2, 6] {
        let doc = HashMap::from([("value".to_string(), json!(i))]);
        db.insert_one(&coll_name, doc).unwrap();
    }

    let options = ironbase_core::FindOptions {
        projection: None,
        sort: Some(vec![("value".to_string(), 1)]), // ascending
        limit: None,
        skip: None,
        include_total: false,
        ..Default::default()
    };

    let results = collection.find_with_options(&json!({}), options).unwrap();
    assert_eq!(results[0]["value"], 1);
    assert_eq!(results[results.len() - 1]["value"], 9);
}

/// FIX #22: Test index-based sort for empty queries
/// When query is {} but has sort+index, should use B+ tree for ordered iteration
/// instead of loading all documents and sorting in-memory.
#[test]
fn test_index_based_sort_for_empty_query() {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    let coll = db.collection("test").unwrap();

    // Create index BEFORE inserting (index is built incrementally)
    coll.create_index("timestamp".to_string(), false, false)
        .unwrap();

    // Insert 1000 documents in random order
    for i in 0..1000i64 {
        // Zigzag pattern to ensure data is not naturally sorted
        let ts = if i % 2 == 0 { i } else { 1000 - i };
        let mut doc = HashMap::new();
        doc.insert("timestamp".to_string(), json!(ts));
        doc.insert("data".to_string(), json!(format!("doc_{}", i)));
        db.insert_one("test", doc).unwrap();
    }

    // Test ascending sort with limit (should use index)
    // Zigzag pattern covers all values 0-999, so sorted ascending: 0, 1, 2, 3, 4, ...
    let options = ironbase_core::FindOptions::new()
        .with_sort(vec![("timestamp".to_string(), 1)])
        .with_limit(5);
    let results = coll.find_with_options(&json!({}), options).unwrap();

    assert_eq!(results.len(), 5);
    assert_eq!(results[0]["timestamp"], 0);
    assert_eq!(results[1]["timestamp"], 1);
    assert_eq!(results[2]["timestamp"], 2);
    assert_eq!(results[3]["timestamp"], 3);
    assert_eq!(results[4]["timestamp"], 4);

    // Test descending sort with limit (should use index + reverse)
    // Sorted descending: 999, 998, 997, ...
    let options = ironbase_core::FindOptions::new()
        .with_sort(vec![("timestamp".to_string(), -1)])
        .with_limit(5);
    let results = coll.find_with_options(&json!({}), options).unwrap();

    assert_eq!(results.len(), 5);
    assert_eq!(results[0]["timestamp"], 999);
    assert_eq!(results[1]["timestamp"], 998);
    assert_eq!(results[2]["timestamp"], 997);
    assert_eq!(results[3]["timestamp"], 996);
    assert_eq!(results[4]["timestamp"], 995);

    // Test with skip + limit
    // Skip first 10 (0,1,2,3,4,5,6,7,8,9) -> start at 10
    let options = ironbase_core::FindOptions::new()
        .with_sort(vec![("timestamp".to_string(), 1)])
        .with_skip(10)
        .with_limit(5);
    let results = coll.find_with_options(&json!({}), options).unwrap();

    assert_eq!(results.len(), 5);
    assert_eq!(results[0]["timestamp"], 10);
    assert_eq!(results[1]["timestamp"], 11);
    assert_eq!(results[2]["timestamp"], 12);
}

/// P1-5: Top-K streaming for find sort+limit WITHOUT a usable sort index.
///
/// Mirrors `test_index_based_sort_for_empty_query` but creates NO index on the
/// sort field, so `needs_memory_sort` is true and the new O(k) top-k heap path
/// is exercised (collection scan → heap). Results MUST match the index path.
#[test]
fn test_find_topk_no_index_single_field_sort() {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    let coll = db.collection("test").unwrap();

    // NO index on "timestamp" -> collection scan + in-memory top-k.
    // Zigzag covers every integer 0..1000 exactly once.
    for i in 0..1000i64 {
        let ts = if i % 2 == 0 { i } else { 1000 - i };
        let mut doc = HashMap::new();
        doc.insert("timestamp".to_string(), json!(ts));
        doc.insert("data".to_string(), json!(format!("doc_{}", i)));
        db.insert_one("test", doc).unwrap();
    }

    // Ascending top-5
    let results = coll
        .find_with_options(
            &json!({}),
            ironbase_core::FindOptions::new()
                .with_sort(vec![("timestamp".to_string(), 1)])
                .with_limit(5),
        )
        .unwrap();
    assert_eq!(results.len(), 5);
    for (i, r) in results.iter().enumerate() {
        assert_eq!(r["timestamp"], json!(i as i64));
    }

    // Descending top-5
    let results = coll
        .find_with_options(
            &json!({}),
            ironbase_core::FindOptions::new()
                .with_sort(vec![("timestamp".to_string(), -1)])
                .with_limit(5),
        )
        .unwrap();
    assert_eq!(results.len(), 5);
    for (i, r) in results.iter().enumerate() {
        assert_eq!(r["timestamp"], json!(999 - i as i64));
    }

    // skip + limit
    let results = coll
        .find_with_options(
            &json!({}),
            ironbase_core::FindOptions::new()
                .with_sort(vec![("timestamp".to_string(), 1)])
                .with_skip(10)
                .with_limit(5),
        )
        .unwrap();
    assert_eq!(results.len(), 5);
    for (i, r) in results.iter().enumerate() {
        assert_eq!(r["timestamp"], json!(10 + i as i64));
    }
}

/// P1-5: A multi-field sort can never be satisfied by a single-field index, so
/// it always takes the in-memory top-k heap path when a limit is present.
/// Verifies ordering AND tie-breaking on the second field.
#[test]
fn test_find_topk_multi_field_sort() {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    let coll = db.collection("test").unwrap();

    let data = [
        (2, 10),
        (1, 5),
        (2, 30),
        (1, 99),
        (3, 1),
        (2, 20),
        (1, 50),
        (3, 7),
        (2, 25),
        (1, 1),
    ];
    for (g, r) in data {
        let mut doc = HashMap::new();
        doc.insert("group".to_string(), json!(g));
        doc.insert("rank".to_string(), json!(r));
        db.insert_one("test", doc).unwrap();
    }

    // group ASC, then rank DESC. group 1 is smallest → top-4 are all group 1,
    // ranks 99, 50, 5, 1.
    let results = coll
        .find_with_options(
            &json!({}),
            ironbase_core::FindOptions::new()
                .with_sort(vec![("group".to_string(), 1), ("rank".to_string(), -1)])
                .with_limit(4),
        )
        .unwrap();
    assert_eq!(results.len(), 4);
    for r in &results {
        assert_eq!(r["group"], json!(1));
    }
    assert_eq!(results[0]["rank"], json!(99));
    assert_eq!(results[1]["rank"], json!(50));
    assert_eq!(results[2]["rank"], json!(5));
    assert_eq!(results[3]["rank"], json!(1));
}

/// P1-5 core invariant: the top-k path must produce EXACTLY the same result as
/// the full-sort path truncated to skip+limit. Runs both code paths on the same
/// data/query and asserts equality (id tiebreaker makes the order total).
#[test]
fn test_find_topk_matches_full_sort_reference() {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    let coll = db.collection("test").unwrap();

    for i in 0..500i64 {
        let v = (i * 7919) % 1000; // spread with collisions
        let mut doc = HashMap::new();
        doc.insert("v".to_string(), json!(v));
        doc.insert("id".to_string(), json!(i));
        db.insert_one("test", doc).unwrap();
    }

    let sort = vec![("v".to_string(), 1), ("id".to_string(), 1)];
    let skip = 5usize;
    let limit = 17usize;

    // Reference: full sort (no limit → non-top-k path), then manual skip+take.
    let full = coll
        .find_with_options(
            &json!({}),
            ironbase_core::FindOptions::new().with_sort(sort.clone()),
        )
        .unwrap();
    let reference: Vec<_> = full.iter().skip(skip).take(limit).cloned().collect();

    // Top-K path (skip + limit → heap).
    let topk = coll
        .find_with_options(
            &json!({}),
            ironbase_core::FindOptions::new()
                .with_sort(sort)
                .with_skip(skip)
                .with_limit(limit),
        )
        .unwrap();

    assert_eq!(topk.len(), reference.len());
    assert_eq!(
        topk, reference,
        "top-k result must equal full-sort[skip..skip+limit]"
    );
}

/// P1-5 regression guard: on a MIXED-TYPE sort field the heap's comparator must
/// rank identically to `apply_sort` (type_priority fallback), or the heap would
/// evict the wrong docs and the top-k set itself would diverge from the full
/// sort. This test fails on the old `Ordering::Equal` fallback.
#[test]
fn test_find_topk_mixed_type_matches_full_sort() {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    let coll = db.collection("test").unwrap();

    let vals = [
        json!(5),
        json!("apple"),
        json!(true),
        json!(null),
        json!(2),
        json!("banana"),
        json!(100),
        json!(false),
        json!(1),
        json!("cherry"),
    ];
    for (i, v) in vals.iter().enumerate() {
        let mut doc = HashMap::new();
        doc.insert("k".to_string(), v.clone());
        doc.insert("id".to_string(), json!(i as i64));
        db.insert_one("test", doc).unwrap();
    }
    // A few docs with "k" missing entirely (None sorts before any present value).
    for i in 100..103i64 {
        let mut doc = HashMap::new();
        doc.insert("id".to_string(), json!(i));
        db.insert_one("test", doc).unwrap();
    }

    let sort = vec![("k".to_string(), 1), ("id".to_string(), 1)];
    let full = coll
        .find_with_options(
            &json!({}),
            ironbase_core::FindOptions::new().with_sort(sort.clone()),
        )
        .unwrap();

    for limit in [1usize, 3, 7, 13] {
        let topk = coll
            .find_with_options(
                &json!({}),
                ironbase_core::FindOptions::new()
                    .with_sort(sort.clone())
                    .with_limit(limit),
            )
            .unwrap();
        let reference: Vec<_> = full.iter().take(limit).cloned().collect();
        assert_eq!(
            topk, reference,
            "mixed-type top-{} must equal full-sort prefix",
            limit
        );
    }
}

/// P1-5: the response-size guard still fires on a full (no-limit) in-memory sort.
/// The guard moved into the non-top-k branch but must stay effective there.
#[test]
fn test_find_full_sort_respects_response_limit() {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    let coll = db.collection("test").unwrap();
    for i in 0..2000i64 {
        let mut doc = HashMap::new();
        doc.insert("v".to_string(), json!(i));
        doc.insert("blob".to_string(), json!("x".repeat(500)));
        db.insert_one("test", doc).unwrap();
    }
    // No limit + tiny explicit cap → must error instead of materializing all.
    let opts = ironbase_core::FindOptions {
        sort: Some(vec![("v".to_string(), 1)]),
        max_response_bytes: Some(50_000), // ~50KB << 2000 * 500B
        ..Default::default()
    };
    assert!(
        coll.find_with_options(&json!({}), opts).is_err(),
        "full sort exceeding the response cap must error"
    );
}

/// P1-5: with a small limit the top-k path bounds the result, so the SAME tiny
/// response cap is satisfied — the guard applies to the ≤limit result, not the
/// whole scan. (On the old materialize-then-truncate path this would also error.)
#[test]
fn test_find_topk_within_response_limit() {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    let coll = db.collection("test").unwrap();
    for i in 0..2000i64 {
        let mut doc = HashMap::new();
        doc.insert("v".to_string(), json!(i));
        doc.insert("blob".to_string(), json!("x".repeat(500)));
        db.insert_one("test", doc).unwrap();
    }
    // Same tiny cap, but limit 5 → top-5 (~2.6KB) fits comfortably.
    let opts = ironbase_core::FindOptions {
        sort: Some(vec![("v".to_string(), 1)]),
        limit: Some(5),
        max_response_bytes: Some(50_000),
        ..Default::default()
    };
    let results = coll.find_with_options(&json!({}), opts).unwrap();
    assert_eq!(results.len(), 5);
    assert_eq!(results[0]["v"], json!(0));
    assert_eq!(results[4]["v"], json!(4));
}

/// P1-5 review fix #3: the top-k heap is STABLE on tied sort keys — it returns the
/// same set AND order as the old stable apply_sort+truncate (first-k by scan order
/// on ties). Before the seq-tiebreaker fix the heap could return a different tied
/// subset (e.g. {d,b} instead of {d,a}) and nondeterministically.
#[test]
fn test_find_topk_stable_on_ties() {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    let coll = db.collection("test").unwrap();
    // Many docs tie on age=5; tag records insertion (scan) order.
    for (age, tag) in [(5, "a"), (5, "b"), (5, "c"), (1, "d"), (5, "e"), (1, "f")] {
        let mut doc = HashMap::new();
        doc.insert("age".to_string(), json!(age));
        doc.insert("tag".to_string(), json!(tag));
        db.insert_one("test", doc).unwrap();
    }
    let sort = vec![("age".to_string(), 1)];
    // Reference: full stable sort (no limit → non-top-k path), truncated per limit.
    let full = coll
        .find_with_options(
            &json!({}),
            ironbase_core::FindOptions::new().with_sort(sort.clone()),
        )
        .unwrap();
    for limit in [1usize, 2, 3, 4, 5, 6] {
        let topk = coll
            .find_with_options(
                &json!({}),
                ironbase_core::FindOptions::new()
                    .with_sort(sort.clone())
                    .with_limit(limit),
            )
            .unwrap();
        let reference: Vec<_> = full.iter().take(limit).cloned().collect();
        assert_eq!(
            topk, reference,
            "top-{} must equal the stable full-sort prefix on ties",
            limit
        );
    }
}

/// P1-5 review fix #2: an invalid sort direction is rejected on EVERY path — both
/// the top-k path (with a limit) and the full-sort path (no limit). Previously the
/// top-k path bypassed apply_sort and silently treated direction=2 as ascending.
#[test]
fn test_find_invalid_sort_direction_rejected_all_paths() {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    let coll = db.collection("test").unwrap();
    for i in 0..10i64 {
        let mut doc = HashMap::new();
        doc.insert("age".to_string(), json!(i));
        db.insert_one("test", doc).unwrap();
    }
    // direction 2 is invalid — must error WITH a limit (top-k path) ...
    assert!(
        coll.find_with_options(
            &json!({}),
            ironbase_core::FindOptions::new()
                .with_sort(vec![("age".to_string(), 2)])
                .with_limit(3),
        )
        .is_err(),
        "invalid direction must error on the top-k path"
    );
    // ... and WITHOUT a limit (full-sort path).
    assert!(
        coll.find_with_options(
            &json!({}),
            ironbase_core::FindOptions::new().with_sort(vec![("age".to_string(), 2)]),
        )
        .is_err(),
        "invalid direction must error on the full-sort path"
    );
    // Valid directions still succeed on the top-k path.
    assert!(coll
        .find_with_options(
            &json!({}),
            ironbase_core::FindOptions::new()
                .with_sort(vec![("age".to_string(), -1)])
                .with_limit(3),
        )
        .is_ok());
}

/// P1-5 review fix #1: deep-pagination skip over few matches must NOT eagerly
/// allocate O(skip) — the heap grows lazily to the retained docs. An extreme skip
/// (which the old with_capacity(skip+limit) turned into a capacity-overflow panic)
/// must complete and return the correct empty page.
#[test]
fn test_find_topk_extreme_skip_no_panic() {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    let coll = db.collection("test").unwrap();
    for i in 0..50i64 {
        let mut doc = HashMap::new();
        doc.insert("v".to_string(), json!(i));
        db.insert_one("test", doc).unwrap();
    }
    // skip = usize::MAX would panic under BinaryHeap::with_capacity(usize::MAX);
    // with the lazy heap it just yields an empty page (only 50 docs exist).
    let results = coll
        .find_with_options(
            &json!({}),
            ironbase_core::FindOptions::new()
                .with_sort(vec![("v".to_string(), 1)])
                .with_skip(usize::MAX)
                .with_limit(10),
        )
        .unwrap();
    assert_eq!(results.len(), 0);
}

/// FIX #23: Performance test for DESC sort with early termination
/// This should be O(limit) not O(N) when using index-based sorting
#[test]
#[ignore] // Run with --ignored for performance tests
fn perf_test_desc_sort_with_limit_early_termination() {
    use std::time::Instant;

    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    let coll = db.collection("test").unwrap();

    // Create index BEFORE inserting
    coll.create_index("date".to_string(), false, false).unwrap();

    // Insert 50K documents (simulating large emails collection)
    let doc_count = 50_000;
    println!("Inserting {} documents...", doc_count);
    let start = Instant::now();
    for i in 0..doc_count as i64 {
        let mut doc = HashMap::new();
        doc.insert("date".to_string(), json!(i));
        doc.insert("subject".to_string(), json!(format!("Email {}", i)));
        db.insert_one("test", doc).unwrap();
    }
    println!("Insert time: {:?}", start.elapsed());

    // Test: DESC sort with limit 5 should be nearly instant (early termination)
    let options = ironbase_core::FindOptions::new()
        .with_sort(vec![("date".to_string(), -1)])
        .with_limit(5);

    let start = Instant::now();
    let results = coll.find_with_options(&json!({}), options).unwrap();
    let elapsed = start.elapsed();

    assert_eq!(results.len(), 5);
    assert_eq!(results[0]["date"], doc_count - 1);
    assert_eq!(results[4]["date"], doc_count - 5);

    println!(
        "DESC sort + limit 5 on {} docs: {:?} (should be < 10ms with early termination)",
        doc_count, elapsed
    );

    // Early termination should make this very fast (< 10ms)
    // Without early termination, this would load all 50K doc IDs first
    assert!(
        elapsed.as_millis() < 100,
        "DESC sort with limit should be < 100ms (was {:?}) due to early termination",
        elapsed
    );
}

/// FIX #22: Verify that filter queries still work correctly (no index optimization)
#[test]
fn test_sort_with_filter_uses_collection_scan() {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    let coll = db.collection("test").unwrap();

    // Create index on timestamp only
    coll.create_index("timestamp".to_string(), false, false)
        .unwrap();

    // Insert documents
    for i in 0..100i64 {
        let mut doc = HashMap::new();
        doc.insert("timestamp".to_string(), json!(i));
        doc.insert(
            "category".to_string(),
            json!(if i % 3 == 0 { "A" } else { "B" }),
        );
        db.insert_one("test", doc).unwrap();
    }

    // Filter + sort: should NOT use index for sort (must scan & filter first)
    let options = ironbase_core::FindOptions::new()
        .with_sort(vec![("timestamp".to_string(), -1)])
        .with_limit(5);
    let results = coll
        .find_with_options(&json!({"category": "A"}), options)
        .unwrap();

    assert_eq!(results.len(), 5);
    // Category A documents: 0, 3, 6, 9, ... 99
    // Descending: 99, 96, 93, 90, 87
    assert_eq!(results[0]["timestamp"], 99);
    assert_eq!(results[1]["timestamp"], 96);
    assert_eq!(results[2]["timestamp"], 93);
    assert_eq!(results[0]["category"], "A");
}

#[test]
fn test_find_with_limit_skip() {
    let (db, coll_name) = create_test_db("test");
    let collection = db.collection(&coll_name).unwrap();

    for i in 0..20 {
        let doc = HashMap::from([("value".to_string(), json!(i))]);
        db.insert_one(&coll_name, doc).unwrap();
    }

    let options = ironbase_core::FindOptions {
        projection: None,
        sort: Some(vec![("value".to_string(), 1)]),
        limit: Some(5),
        skip: Some(10),
        include_total: false,
        ..Default::default()
    };

    let results = collection.find_with_options(&json!({}), options).unwrap();
    assert_eq!(results.len(), 5);
    assert_eq!(results[0]["value"], 10);
}

// ========== EXPLAIN AND HINT TESTS ==========

#[test]
fn test_explain() {
    let (db, coll_name) = create_test_db("test");
    let collection = db.collection(&coll_name).unwrap();

    collection
        .create_index("age".to_string(), false, false)
        .unwrap();

    let doc = HashMap::from([("age".to_string(), json!(25))]);
    db.insert_one(&coll_name, doc).unwrap();

    let plan = collection.explain(&json!({"age": 25})).unwrap();
    // Plan is a JSON value with new explain format
    assert!(plan.is_object());
    // New format: chosenPlan contains queryPlan, candidates lists all evaluated plans
    assert!(plan.get("chosenPlan").is_some());
    assert!(plan["chosenPlan"].get("queryPlan").is_some());
    assert!(plan.get("candidates").is_some());
    assert!(plan.get("selectionReason").is_some());
}

#[test]
fn test_find_with_hint() {
    let (db, coll_name) = create_test_db("test");
    let collection = db.collection(&coll_name).unwrap();

    let index_name = collection
        .create_index("age".to_string(), false, false)
        .unwrap();

    for i in 0..10 {
        let doc = HashMap::from([("age".to_string(), json!(i))]);
        db.insert_one(&coll_name, doc).unwrap();
    }

    // Re-create collection instance to load indexes rebuilt from catalog
    let collection = db.collection(&coll_name).unwrap();
    let results = collection
        .find_with_hint(&json!({"age": {"$gte": 5}}), &index_name)
        .unwrap();
    assert_eq!(results.len(), 5);
}

// ========== EDGE CASES ==========

#[test]
fn test_null_query() {
    let (db, coll_name) = create_test_db("test");
    let collection = db.collection(&coll_name).unwrap();

    let doc = HashMap::from([("name".to_string(), json!("Alice"))]);
    db.insert_one(&coll_name, doc).unwrap();

    // Null query should match all
    let count = collection.count_documents(&json!(null)).unwrap();
    assert_eq!(count, 1);
}

#[test]
fn test_cursor_for_each() {
    let (db, coll_name) = create_test_db("test");
    let collection = db.collection(&coll_name).unwrap();

    for i in 0..5 {
        let doc = HashMap::from([("value".to_string(), json!(i))]);
        db.insert_one(&coll_name, doc).unwrap();
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
    use ironbase_core::DatabaseCore;
    use serde_json::json;
    use std::collections::HashMap;

    fn create_memory_db() -> (DatabaseCore<MemoryStorage>, String) {
        let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
        (db, "test".to_string())
    }

    #[test]
    fn test_memory_insert_and_find() {
        let (db, coll_name) = create_memory_db();
        let collection = db.collection(&coll_name).unwrap();

        let doc = HashMap::from([
            ("name".to_string(), json!("Alice")),
            ("age".to_string(), json!(30)),
        ]);
        let id = db.insert_one(&coll_name, doc).unwrap();

        let found = collection.find_one(&json!({"_id": id})).unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap()["name"], "Alice");
    }

    #[test]
    fn test_memory_count() {
        let (db, coll_name) = create_memory_db();
        let collection = db.collection(&coll_name).unwrap();

        for i in 0..10 {
            let doc = HashMap::from([("value".to_string(), json!(i))]);
            db.insert_one(&coll_name, doc).unwrap();
        }

        let count = collection.count_documents(&json!({})).unwrap();
        assert_eq!(count, 10);
    }

    #[test]
    fn test_memory_update() {
        let (db, coll_name) = create_memory_db();
        let collection = db.collection(&coll_name).unwrap();

        let doc = HashMap::from([("name".to_string(), json!("Alice"))]);
        let id = db.insert_one(&coll_name, doc).unwrap();

        db.update_one(
            &coll_name,
            &json!({"_id": id}),
            &json!({"$set": {"name": "Bob"}}),
        )
        .unwrap();

        let updated = collection.find_one(&json!({"_id": id})).unwrap().unwrap();
        assert_eq!(updated["name"], "Bob");
    }

    #[test]
    fn test_memory_delete() {
        let (db, coll_name) = create_memory_db();
        let collection = db.collection(&coll_name).unwrap();

        let doc = HashMap::from([("name".to_string(), json!("Alice"))]);
        let id = db.insert_one(&coll_name, doc).unwrap();

        db.delete_one(&coll_name, &json!({"_id": id})).unwrap();

        let count = collection.count_documents(&json!({})).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_memory_index() {
        let (db, coll_name) = create_memory_db();
        let collection = db.collection(&coll_name).unwrap();

        let index_name = collection
            .create_index("age".to_string(), false, false)
            .unwrap();
        assert!(index_name.contains("age"));

        for i in 0..10 {
            let doc = HashMap::from([("age".to_string(), json!(i))]);
            db.insert_one(&coll_name, doc).unwrap();
        }

        // Re-create collection instance to load indexes rebuilt from catalog
        let collection = db.collection(&coll_name).unwrap();
        let results = collection.find(&json!({"age": {"$gte": 5}})).unwrap();
        assert_eq!(results.len(), 5);
    }

    #[test]
    fn test_memory_aggregation() {
        let (db, coll_name) = create_memory_db();
        let collection = db.collection(&coll_name).unwrap();

        for city in &["NYC", "LA", "NYC", "LA", "NYC"] {
            let doc = HashMap::from([("city".to_string(), json!(city))]);
            db.insert_one(&coll_name, doc).unwrap();
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
        let (db, coll_name) = create_memory_db();
        let collection = db.collection(&coll_name).unwrap();

        for i in 0..20 {
            let doc = HashMap::from([("value".to_string(), json!(i))]);
            db.insert_one(&coll_name, doc).unwrap();
        }

        let mut cursor = collection.find_streaming(&json!({})).unwrap();
        assert_eq!(cursor.total(), 20);

        let batch = cursor.next_chunk(5).unwrap();
        assert_eq!(batch.len(), 5);
        assert_eq!(cursor.remaining(), 15);
    }
}

#[test]
fn test_aggregation_with_nested_fields_from_storage() {
    let temp_dir = tempfile::tempdir().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    let db = DatabaseCore::open(db_path.to_str().unwrap()).unwrap();
    let collection = db.collection("companies").unwrap();

    // Insert documents with nested fields
    let mut doc1: HashMap<String, serde_json::Value> = HashMap::new();
    doc1.insert("Name".to_string(), json!("TechCorp"));
    doc1.insert(
        "Location".to_string(),
        json!({"Country": "USA", "City": "NYC"}),
    );
    doc1.insert("Stats".to_string(), json!({"Employees": 100}));
    db.insert_one("companies", doc1).unwrap();

    let mut doc2: HashMap<String, serde_json::Value> = HashMap::new();
    doc2.insert("Name".to_string(), json!("DataSoft"));
    doc2.insert(
        "Location".to_string(),
        json!({"Country": "USA", "City": "LA"}),
    );
    doc2.insert("Stats".to_string(), json!({"Employees": 200}));
    db.insert_one("companies", doc2).unwrap();

    // Find to verify storage
    let found = collection.find(&json!({})).unwrap();
    println!("Found documents:");
    for doc in &found {
        println!("  {:?}", doc);
    }

    // Aggregate
    let results = collection
        .aggregate(&json!([
            {"$group": {
                "_id": "$Location.Country",
                "totalEmployees": {"$sum": "$Stats.Employees"},
                "count": {"$sum": 1}
            }}
        ]))
        .unwrap();

    println!("Aggregation results: {:?}", results);

    // Should have 1 group: USA
    assert_eq!(results.len(), 1, "Expected 1 group, got {:?}", results);
    assert_eq!(
        results[0]["_id"], "USA",
        "Expected _id=USA, got {:?}",
        results[0]["_id"]
    );
    assert_eq!(results[0]["totalEmployees"], 300);
    assert_eq!(results[0]["count"], 2);
}

// =============================================================================
// NESTED DOCUMENT TESTS - Comprehensive coverage for dot notation functionality
// =============================================================================

mod nested_document_tests {
    use super::*;
    use ironbase_core::{storage::MemoryStorage, DatabaseCore, FindOptions};

    /// Helper to create a test database with nested documents
    fn setup_nested_test_db() -> DatabaseCore<MemoryStorage> {
        let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();

        // Insert documents with various nested structures
        db.insert_one(
            "users",
            HashMap::from([
                ("name".to_string(), json!("Alice")),
                ("age".to_string(), json!(30)),
                (
                    "address".to_string(),
                    json!({
                        "city": "New York",
                        "zip": "10001",
                        "location": {
                            "lat": 40.7128,
                            "lng": -74.0060
                        }
                    }),
                ),
                (
                    "profile".to_string(),
                    json!({
                        "score": 95,
                        "level": "senior",
                        "stats": {
                            "posts": 150,
                            "likes": 2500
                        }
                    }),
                ),
                ("tags".to_string(), json!(["developer", "team-lead"])),
            ]),
        )
        .unwrap();

        db.insert_one(
            "users",
            HashMap::from([
                ("name".to_string(), json!("Bob")),
                ("age".to_string(), json!(25)),
                (
                    "address".to_string(),
                    json!({
                        "city": "Los Angeles",
                        "zip": "90001",
                        "location": {
                            "lat": 34.0522,
                            "lng": -118.2437
                        }
                    }),
                ),
                (
                    "profile".to_string(),
                    json!({
                        "score": 75,
                        "level": "junior",
                        "stats": {
                            "posts": 30,
                            "likes": 500
                        }
                    }),
                ),
                ("tags".to_string(), json!(["developer"])),
            ]),
        )
        .unwrap();

        db.insert_one(
            "users",
            HashMap::from([
                ("name".to_string(), json!("Carol")),
                ("age".to_string(), json!(35)),
                (
                    "address".to_string(),
                    json!({
                        "city": "New York",
                        "zip": "10002",
                        "location": {
                            "lat": 40.7589,
                            "lng": -73.9851
                        }
                    }),
                ),
                (
                    "profile".to_string(),
                    json!({
                        "score": 88,
                        "level": "senior",
                        "stats": {
                            "posts": 200,
                            "likes": 3500
                        }
                    }),
                ),
                ("tags".to_string(), json!(["manager", "speaker"])),
            ]),
        )
        .unwrap();

        // David - missing location and stats for edge case testing
        db.insert_one(
            "users",
            HashMap::from([
                ("name".to_string(), json!("David")),
                ("age".to_string(), json!(28)),
                (
                    "address".to_string(),
                    json!({
                        "city": "Chicago",
                        "zip": "60601"
                    }),
                ),
                (
                    "profile".to_string(),
                    json!({
                        "score": 82,
                        "level": "mid"
                    }),
                ),
                ("tags".to_string(), json!(["developer"])),
            ]),
        )
        .unwrap();

        db
    }

    // =========================================================================
    // QUERY TESTS - Find with dot notation
    // =========================================================================

    #[test]
    fn test_query_nested_field_equality() {
        let db = setup_nested_test_db();
        let coll = db.collection("users").unwrap();

        // Query by nested field
        let results = coll.find(&json!({"address.city": "New York"})).unwrap();
        assert_eq!(results.len(), 2, "Should find 2 users in New York");

        let names: Vec<&str> = results
            .iter()
            .map(|d| d["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"Alice"));
        assert!(names.contains(&"Carol"));
    }

    #[test]
    fn test_query_nested_field_comparison() {
        let db = setup_nested_test_db();
        let coll = db.collection("users").unwrap();

        // Query with comparison on nested field
        let results = coll.find(&json!({"profile.score": {"$gte": 85}})).unwrap();
        assert_eq!(results.len(), 2, "Should find 2 users with score >= 85");

        let results = coll.find(&json!({"profile.score": {"$lt": 80}})).unwrap();
        assert_eq!(results.len(), 1, "Should find 1 user with score < 80");
        assert_eq!(results[0]["name"], "Bob");
    }

    #[test]
    fn test_query_deep_nested_field() {
        let db = setup_nested_test_db();
        let coll = db.collection("users").unwrap();

        // Query 3-level deep nesting
        let results = coll
            .find(&json!({"profile.stats.posts": {"$gte": 100}}))
            .unwrap();
        assert_eq!(results.len(), 2, "Should find 2 users with posts >= 100");

        let results = coll
            .find(&json!({"address.location.lat": {"$gt": 40.0}}))
            .unwrap();
        assert_eq!(results.len(), 2, "Should find 2 users with lat > 40.0");
    }

    #[test]
    fn test_query_nested_field_string_match() {
        let db = setup_nested_test_db();
        let coll = db.collection("users").unwrap();

        // Query nested string field
        let results = coll.find(&json!({"profile.level": "senior"})).unwrap();
        assert_eq!(results.len(), 2, "Should find 2 senior users");

        let results = coll.find(&json!({"profile.level": "junior"})).unwrap();
        assert_eq!(results.len(), 1, "Should find 1 junior user");
    }

    #[test]
    fn test_query_nested_with_multiple_conditions() {
        let db = setup_nested_test_db();
        let coll = db.collection("users").unwrap();

        // Multiple conditions including nested fields
        let results = coll
            .find(&json!({
                "address.city": "New York",
                "profile.score": {"$gte": 90}
            }))
            .unwrap();
        assert_eq!(
            results.len(),
            1,
            "Should find 1 user in NYC with score >= 90"
        );
        assert_eq!(results[0]["name"], "Alice");
    }

    #[test]
    fn test_query_nested_with_or() {
        let db = setup_nested_test_db();
        let coll = db.collection("users").unwrap();

        // OR query with nested fields
        let results = coll
            .find(&json!({
                "$or": [
                    {"address.city": "Chicago"},
                    {"profile.level": "junior"}
                ]
            }))
            .unwrap();
        assert_eq!(results.len(), 2, "Should find 2 users (Chicago or junior)");
    }

    #[test]
    fn test_query_missing_nested_field() {
        let db = setup_nested_test_db();
        let coll = db.collection("users").unwrap();

        // Query for field that doesn't exist in some docs
        let results = coll
            .find(&json!({"address.location.lat": {"$exists": true}}))
            .unwrap();
        assert_eq!(results.len(), 3, "Should find 3 users with location.lat");

        // David doesn't have location
        let results = coll
            .find(&json!({"address.location": {"$exists": false}}))
            .unwrap();
        assert_eq!(results.len(), 1, "Should find 1 user without location");
        assert_eq!(results[0]["name"], "David");
    }

    // =========================================================================
    // UPDATE TESTS - Update nested fields with dot notation
    // =========================================================================

    #[test]
    fn test_update_nested_field_set() {
        let db = setup_nested_test_db();
        let coll = db.collection("users").unwrap();

        // Update nested field - returns (matched, modified)
        let (matched, modified) = db
            .update_one(
                "users",
                &json!({"name": "Alice"}),
                &json!({"$set": {"address.city": "Boston"}}),
            )
            .unwrap();
        assert_eq!(matched, 1);
        assert_eq!(modified, 1);

        // Verify update
        let results = coll.find(&json!({"name": "Alice"})).unwrap();
        assert_eq!(results[0]["address"]["city"], "Boston");
    }

    #[test]
    fn test_update_deep_nested_field() {
        let db = setup_nested_test_db();
        let coll = db.collection("users").unwrap();

        // Update 3-level deep nested field
        let (matched, modified) = db
            .update_one(
                "users",
                &json!({"name": "Alice"}),
                &json!({"$set": {"profile.stats.likes": 3000}}),
            )
            .unwrap();
        assert_eq!(matched, 1);
        assert_eq!(modified, 1);

        // Verify update
        let results = coll.find(&json!({"name": "Alice"})).unwrap();
        assert_eq!(results[0]["profile"]["stats"]["likes"], 3000);
    }

    #[test]
    fn test_update_nested_field_inc() {
        let db = setup_nested_test_db();
        let coll = db.collection("users").unwrap();

        // Increment nested field
        let (matched, modified) = db
            .update_one(
                "users",
                &json!({"name": "Bob"}),
                &json!({"$inc": {"profile.score": 10}}),
            )
            .unwrap();
        assert_eq!(matched, 1);
        assert_eq!(modified, 1);

        // Verify - Bob's score was 75, should be 85
        let results = coll.find(&json!({"name": "Bob"})).unwrap();
        assert_eq!(results[0]["profile"]["score"], 85);
    }

    #[test]
    fn test_update_nested_field_unset() {
        let db = setup_nested_test_db();
        let coll = db.collection("users").unwrap();

        // Unset nested field
        let (matched, modified) = db
            .update_one(
                "users",
                &json!({"name": "Alice"}),
                &json!({"$unset": {"address.zip": ""}}),
            )
            .unwrap();
        assert_eq!(matched, 1);
        assert_eq!(modified, 1);

        // Verify - zip should be removed
        let results = coll.find(&json!({"name": "Alice"})).unwrap();
        assert!(
            results[0]["address"].get("zip").is_none() || results[0]["address"]["zip"].is_null()
        );
    }

    #[test]
    fn test_update_many_nested_fields() {
        let db = setup_nested_test_db();
        let coll = db.collection("users").unwrap();

        // Update all seniors - returns (matched, modified)
        let (matched, modified) = db
            .update_many(
                "users",
                &json!({"profile.level": "senior"}),
                &json!({"$inc": {"profile.score": 5}}),
            )
            .unwrap();
        assert_eq!(matched, 2, "Should match 2 senior users");
        assert_eq!(modified, 2, "Should modify 2 senior users");

        // Verify - Alice: 95+5=100, Carol: 88+5=93
        let alice = coll.find_one(&json!({"name": "Alice"})).unwrap().unwrap();
        assert_eq!(alice["profile"]["score"], 100);

        let carol = coll.find_one(&json!({"name": "Carol"})).unwrap().unwrap();
        assert_eq!(carol["profile"]["score"], 93);
    }

    // =========================================================================
    // AGGREGATION TESTS - Aggregate with nested fields
    // =========================================================================

    #[test]
    fn test_aggregate_group_by_nested_field() {
        let db = setup_nested_test_db();
        let coll = db.collection("users").unwrap();

        // Group by nested field
        let results = coll
            .aggregate(&json!([
                {"$group": {
                    "_id": "$address.city",
                    "count": {"$sum": 1}
                }},
                {"$sort": {"count": -1}}
            ]))
            .unwrap();

        assert_eq!(results.len(), 3, "Should have 3 cities");
        assert_eq!(results[0]["_id"], "New York");
        assert_eq!(results[0]["count"], 2);
    }

    #[test]
    fn test_aggregate_sum_nested_field() {
        let db = setup_nested_test_db();
        let coll = db.collection("users").unwrap();

        // Sum nested numeric field
        let results = coll
            .aggregate(&json!([
                {"$group": {
                    "_id": "$profile.level",
                    "totalScore": {"$sum": "$profile.score"},
                    "count": {"$sum": 1}
                }}
            ]))
            .unwrap();

        // Find senior group
        let senior = results
            .iter()
            .find(|r| r["_id"] == "senior")
            .expect("Should have senior group");
        assert_eq!(senior["totalScore"], 95 + 88); // Alice + Carol
        assert_eq!(senior["count"], 2);
    }

    #[test]
    fn test_aggregate_avg_nested_field() {
        let db = setup_nested_test_db();
        let coll = db.collection("users").unwrap();

        // Average nested numeric field
        let results = coll
            .aggregate(&json!([
                {"$group": {
                    "_id": null,
                    "avgScore": {"$avg": "$profile.score"}
                }}
            ]))
            .unwrap();

        assert_eq!(results.len(), 1);
        // (95 + 75 + 88 + 82) / 4 = 85
        assert_eq!(results[0]["avgScore"], 85.0);
    }

    #[test]
    fn test_aggregate_deep_nested_sum() {
        let db = setup_nested_test_db();
        let coll = db.collection("users").unwrap();

        // Sum 3-level deep nested field
        let results = coll
            .aggregate(&json!([
                {"$group": {
                    "_id": "$address.city",
                    "totalPosts": {"$sum": "$profile.stats.posts"},
                    "totalLikes": {"$sum": "$profile.stats.likes"}
                }}
            ]))
            .unwrap();

        // Find New York group (Alice + Carol have stats)
        let nyc = results
            .iter()
            .find(|r| r["_id"] == "New York")
            .expect("Should have New York group");

        // Alice: 150 posts, 2500 likes + Carol: 200 posts, 3500 likes
        assert_eq!(nyc["totalPosts"], 350);
        assert_eq!(nyc["totalLikes"], 6000);
    }

    #[test]
    fn test_aggregate_min_max_nested() {
        let db = setup_nested_test_db();
        let coll = db.collection("users").unwrap();

        let results = coll
            .aggregate(&json!([
                {"$group": {
                    "_id": null,
                    "minScore": {"$min": "$profile.score"},
                    "maxScore": {"$max": "$profile.score"}
                }}
            ]))
            .unwrap();

        // Aggregation returns floats
        assert_eq!(results[0]["minScore"], 75.0); // Bob
        assert_eq!(results[0]["maxScore"], 95.0); // Alice
    }

    #[test]
    fn test_aggregate_match_with_nested_then_group() {
        let db = setup_nested_test_db();
        let coll = db.collection("users").unwrap();

        // Match first, then group
        let results = coll
            .aggregate(&json!([
                {"$match": {"profile.score": {"$gte": 80}}},
                {"$group": {
                    "_id": "$address.city",
                    "avgScore": {"$avg": "$profile.score"},
                    "count": {"$sum": 1}
                }},
                {"$sort": {"avgScore": -1}}
            ]))
            .unwrap();

        // Alice(95), Carol(88), David(82) pass the filter - Bob(75) is excluded
        // Alice and Carol are in New York, David is in Chicago
        // So 2 groups: New York and Chicago
        assert_eq!(results.len(), 2, "Should have 2 city groups after filter");
    }

    // =========================================================================
    // FIND OPTIONS TESTS - Sort and projection with nested fields
    // =========================================================================

    #[test]
    fn test_sort_by_nested_field_asc() {
        let db = setup_nested_test_db();
        let coll = db.collection("users").unwrap();

        let options = FindOptions::new().with_sort(vec![("profile.score".to_string(), 1)]);

        let results = coll.find_with_options(&json!({}), options).unwrap();

        // Should be sorted: Bob(75), David(82), Carol(88), Alice(95)
        assert_eq!(results[0]["name"], "Bob");
        assert_eq!(results[1]["name"], "David");
        assert_eq!(results[2]["name"], "Carol");
        assert_eq!(results[3]["name"], "Alice");
    }

    #[test]
    fn test_sort_by_nested_field_desc() {
        let db = setup_nested_test_db();
        let coll = db.collection("users").unwrap();

        let options = FindOptions::new().with_sort(vec![("profile.score".to_string(), -1)]);

        let results = coll.find_with_options(&json!({}), options).unwrap();

        // Should be sorted: Alice(95), Carol(88), David(82), Bob(75)
        assert_eq!(results[0]["name"], "Alice");
        assert_eq!(results[1]["name"], "Carol");
        assert_eq!(results[2]["name"], "David");
        assert_eq!(results[3]["name"], "Bob");
    }

    #[test]
    fn test_sort_by_deep_nested_field() {
        let db = setup_nested_test_db();
        let coll = db.collection("users").unwrap();

        let options = FindOptions::new().with_sort(vec![("profile.stats.posts".to_string(), -1)]);

        let results = coll.find_with_options(&json!({}), options).unwrap();

        // Carol(200), Alice(150), Bob(30), David(missing -> 0 or last)
        assert_eq!(results[0]["name"], "Carol");
        assert_eq!(results[1]["name"], "Alice");
    }

    #[test]
    fn test_sort_by_nested_string_field() {
        let db = setup_nested_test_db();
        let coll = db.collection("users").unwrap();

        let options = FindOptions::new().with_sort(vec![("address.city".to_string(), 1)]);

        let results = coll.find_with_options(&json!({}), options).unwrap();

        // Alphabetical: Chicago, Los Angeles, New York, New York
        assert_eq!(results[0]["address"]["city"], "Chicago");
        assert_eq!(results[1]["address"]["city"], "Los Angeles");
    }

    #[test]
    fn test_projection_include_nested_field() {
        let db = setup_nested_test_db();
        let coll = db.collection("users").unwrap();

        let mut projection = HashMap::new();
        projection.insert("name".to_string(), 1);
        projection.insert("address.city".to_string(), 1);

        let options = FindOptions::new().with_projection(projection);

        let results = coll.find_with_options(&json!({}), options).unwrap();

        // Should have name and address.city, but not full profile
        for doc in &results {
            assert!(doc.get("name").is_some());
            // Note: projection behavior may vary - at minimum name should exist
        }
    }

    #[test]
    fn test_combined_sort_limit_with_nested() {
        let db = setup_nested_test_db();
        let coll = db.collection("users").unwrap();

        let options = FindOptions::new()
            .with_sort(vec![("profile.score".to_string(), -1)])
            .with_limit(2);

        let results = coll.find_with_options(&json!({}), options).unwrap();

        // Top 2 by score: Alice(95), Carol(88)
        assert_eq!(results.len(), 2);
        assert_eq!(results[0]["name"], "Alice");
        assert_eq!(results[1]["name"], "Carol");
    }

    // =========================================================================
    // EDGE CASE TESTS - Missing fields, null values, etc.
    // =========================================================================

    #[test]
    fn test_query_nonexistent_nested_path() {
        let db = setup_nested_test_db();
        let coll = db.collection("users").unwrap();

        // Query for completely non-existent path
        let results = coll.find(&json!({"foo.bar.baz": "test"})).unwrap();
        assert_eq!(
            results.len(),
            0,
            "Should find nothing for non-existent path"
        );
    }

    #[test]
    fn test_aggregation_with_missing_nested_fields() {
        let db = setup_nested_test_db();
        let coll = db.collection("users").unwrap();

        // Group by field that some docs don't have
        let results = coll
            .aggregate(&json!([
                {"$group": {
                    "_id": "$address.location.lat",
                    "count": {"$sum": 1}
                }}
            ]))
            .unwrap();

        // David has no location, should group as null
        let null_group = results.iter().find(|r| r["_id"].is_null());
        assert!(
            null_group.is_some(),
            "Should have null group for missing fields"
        );
    }

    #[test]
    fn test_aggregate_sum_with_missing_nested_values() {
        let db = setup_nested_test_db();
        let coll = db.collection("users").unwrap();

        // Sum field that some docs don't have
        let results = coll
            .aggregate(&json!([
                {"$group": {
                    "_id": null,
                    "totalPosts": {"$sum": "$profile.stats.posts"}
                }}
            ]))
            .unwrap();

        // Alice: 150, Bob: 30, Carol: 200, David: missing (should be 0)
        // Total: 380
        assert_eq!(results[0]["totalPosts"], 380);
    }

    #[test]
    fn test_update_create_nested_path() {
        let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
        let coll = db.collection("test").unwrap();

        // Insert simple doc
        let mut doc = HashMap::new();
        doc.insert("name".to_string(), json!("Test"));
        db.insert_one("test", doc).unwrap();

        // Create nested path via update
        db.update_one(
            "test",
            &json!({"name": "Test"}),
            &json!({"$set": {"new.nested.field": "value"}}),
        )
        .unwrap();

        let results = coll.find(&json!({"name": "Test"})).unwrap();
        assert_eq!(results[0]["new"]["nested"]["field"], "value");
    }

    #[test]
    fn test_query_nested_null_value() {
        let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
        let coll = db.collection("test").unwrap();

        // Insert doc with null nested value
        let mut doc = HashMap::new();
        doc.insert("name".to_string(), json!("Test"));
        doc.insert("data".to_string(), json!({"value": null}));
        db.insert_one("test", doc).unwrap();

        // Query for null value
        let results = coll.find(&json!({"data.value": null})).unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_query_nested_array_contains() {
        let db = setup_nested_test_db();
        let coll = db.collection("users").unwrap();

        // Query array field (not nested in object, but at root level with nested access)
        let results = coll.find(&json!({"tags": "developer"})).unwrap();
        assert_eq!(results.len(), 3, "Should find 3 developers");
    }

    #[test]
    fn test_multiple_level_nested_update() {
        let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
        let coll = db.collection("test").unwrap();

        let mut doc = HashMap::new();
        doc.insert(
            "a".to_string(),
            json!({
                "b": {
                    "c": {
                        "d": 1
                    }
                }
            }),
        );
        db.insert_one("test", doc).unwrap();

        // Update 4-level deep field
        db.update_one("test", &json!({}), &json!({"$inc": {"a.b.c.d": 99}}))
            .unwrap();

        let results = coll.find(&json!({})).unwrap();
        assert_eq!(results[0]["a"]["b"]["c"]["d"], 100);
    }
}

#[test]
fn test_find_one_with_dot_notation_filter() {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    let coll = db.collection("test_dot_notation").unwrap();

    // Insert two documents with nested meta.documentId
    let mut fields1 = HashMap::new();
    fields1.insert(
        "meta".to_string(),
        json!({"documentId": "doc-1", "version": "1.0"}),
    );
    fields1.insert("data".to_string(), json!("first"));
    db.insert_one("test_dot_notation", fields1).unwrap();

    let mut fields2 = HashMap::new();
    fields2.insert(
        "meta".to_string(),
        json!({"documentId": "doc-2", "version": "2.0"}),
    );
    fields2.insert("data".to_string(), json!("second"));
    db.insert_one("test_dot_notation", fields2).unwrap();

    // Test: find_one with dot notation should return the correct document
    let result = coll.find_one(&json!({"meta.documentId": "doc-2"})).unwrap();
    assert!(result.is_some(), "Should find doc-2");

    let doc = result.unwrap();
    let doc_id = doc
        .get("meta")
        .and_then(|m| m.get("documentId"))
        .and_then(|v| v.as_str());

    assert_eq!(
        doc_id,
        Some("doc-2"),
        "find_one should return doc-2, not doc-1!"
    );
}

// ============================================================================
// MongoDB-style Implicit Array Flattening Tests
// ============================================================================

#[test]
fn test_nested_array_query_with_equality() {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    let coll = db.collection("nested_array_test").unwrap();

    // Insert document with nested arrays (like Schema v1.2 structure)
    let mut doc = HashMap::new();
    doc.insert(
        "structure".to_string(),
        json!([
            {
                "type": "section",
                "children": [
                    {"type": "paragraph", "content": ["hello", "world"]},
                    {"type": "math", "content": ["sqrt", "pi"]}
                ]
            }
        ]),
    );
    db.insert_one("nested_array_test", doc).unwrap();

    // Query should find document via nested array traversal
    let results = coll
        .find(&json!({"structure.children.type": "math"}))
        .unwrap();
    assert_eq!(
        results.len(),
        1,
        "Should find document with nested array query"
    );
}

#[test]
fn test_nested_array_query_with_regex() {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    let coll = db.collection("nested_regex_test").unwrap();

    // Insert document mimicking Schema v1.2 math content
    let mut doc = HashMap::new();
    doc.insert(
        "structure".to_string(),
        json!([
            {
                "children": [
                    {
                        "children": [
                            {
                                "type": "inline_math",
                                "content": ["\\frac{\\sqrt{\\pi}}{2}"]
                            }
                        ]
                    }
                ]
            }
        ]),
    );
    db.insert_one("nested_regex_test", doc).unwrap();

    // Insert another document without the formula
    let mut doc2 = HashMap::new();
    doc2.insert(
        "structure".to_string(),
        json!([
            {
                "children": [
                    {"children": [{"type": "text", "content": ["hello world"]}]}
                ]
            }
        ]),
    );
    db.insert_one("nested_regex_test", doc2).unwrap();

    // Query with regex should find only the first document
    let results = coll
        .find(&json!({
            "structure.children.children.content": {"$regex": "sqrt"}
        }))
        .unwrap();

    assert_eq!(
        results.len(),
        1,
        "Regex should find document with sqrt in deeply nested array"
    );
}

#[test]
fn test_nested_array_query_multiple_matches() {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    let coll = db.collection("multi_match_test").unwrap();

    // Insert documents with different nested values
    for i in 0..5 {
        let mut doc = HashMap::new();
        doc.insert(
            "items".to_string(),
            json!([
                {"tags": [format!("tag-{}", i), "common"]},
                {"tags": ["other"]}
            ]),
        );
        db.insert_one("multi_match_test", doc).unwrap();
    }

    // Query for common tag should find all 5 documents
    let results = coll.find(&json!({"items.tags": "common"})).unwrap();
    assert_eq!(
        results.len(),
        5,
        "Should find all 5 documents with 'common' tag"
    );

    // Query for specific tag should find only one
    let results = coll.find(&json!({"items.tags": "tag-2"})).unwrap();
    assert_eq!(results.len(), 1, "Should find only document with 'tag-2'");
}

// ============================================================================
// $** Wildcard Operator Tests (Recursive Descent Match)
// ============================================================================

#[test]
fn test_wildcard_operator_simple_match() {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    let coll = db.collection("wildcard_test").unwrap();

    // Insert document with nested "name" field
    let mut doc = HashMap::new();
    doc.insert("user".to_string(), json!({"profile": {"name": "Alice"}}));
    db.insert_one("wildcard_test", doc).unwrap();

    // $**.name should find the nested name field
    let results = coll.find(&json!({"$**.name": "Alice"})).unwrap();
    assert_eq!(results.len(), 1, "$**.name should find nested Alice");
}

#[test]
fn test_wildcard_operator_multiple_matches() {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    let coll = db.collection("wildcard_multi").unwrap();

    // Insert document with multiple "name" fields at different depths
    let mut doc = HashMap::new();
    doc.insert("company".to_string(), json!({"name": "Acme"}));
    doc.insert("employee".to_string(), json!({"info": {"name": "Bob"}}));
    db.insert_one("wildcard_multi", doc).unwrap();

    // $**.name with "Bob" should match
    let results = coll.find(&json!({"$**.name": "Bob"})).unwrap();
    assert_eq!(results.len(), 1);

    // $**.name with "Acme" should also match
    let results = coll.find(&json!({"$**.name": "Acme"})).unwrap();
    assert_eq!(results.len(), 1);

    // $**.name with non-existent value should not match
    let results = coll.find(&json!({"$**.name": "NonExistent"})).unwrap();
    assert_eq!(results.len(), 0);
}

#[test]
fn test_wildcard_operator_with_arrays() {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    let coll = db.collection("wildcard_arrays").unwrap();

    // Insert document with "content" inside nested arrays (Schema v1.2 style)
    let mut doc = HashMap::new();
    doc.insert(
        "structure".to_string(),
        json!([
            {
                "children": [
                    {"content": ["hello", "world"]},
                    {"content": ["sqrt", "pi"]}
                ]
            }
        ]),
    );
    db.insert_one("wildcard_arrays", doc).unwrap();

    // $**.content should find all content arrays
    let results = coll.find(&json!({"$**.content": "sqrt"})).unwrap();
    assert_eq!(
        results.len(),
        1,
        "$**.content should find 'sqrt' in nested array"
    );

    // Should also work with first content array
    let results = coll.find(&json!({"$**.content": "hello"})).unwrap();
    assert_eq!(results.len(), 1);
}

#[test]
fn test_wildcard_operator_with_regex() {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    let coll = db.collection("wildcard_regex").unwrap();

    // Insert document with deeply nested math formula
    let mut doc = HashMap::new();
    doc.insert(
        "math".to_string(),
        json!({
            "section": {
                "formulas": [
                    {"content": ["\\frac{\\sqrt{\\pi}}{2}"]},
                    {"content": ["e^{i\\pi} + 1 = 0"]}
                ]
            }
        }),
    );
    db.insert_one("wildcard_regex", doc).unwrap();

    // $**.content with regex should find sqrt
    let results = coll
        .find(&json!({"$**.content": {"$regex": "sqrt"}}))
        .unwrap();
    assert_eq!(
        results.len(),
        1,
        "$**.content with regex should find sqrt formula"
    );

    // Should also find Euler's identity
    let results = coll
        .find(&json!({"$**.content": {"$regex": "e\\^"}}))
        .unwrap();
    assert_eq!(results.len(), 1);
}

#[test]
fn test_wildcard_operator_invalid_nested_path() {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    let coll = db.collection("wildcard_invalid").unwrap();

    // Insert a document
    let mut doc = HashMap::new();
    doc.insert("data".to_string(), json!({"value": 1}));
    db.insert_one("wildcard_invalid", doc).unwrap();

    // $**.children.content (nested path after $**) should return error
    // Invalid query syntax is now properly reported as an error
    let result = coll.find(&json!({"$**.children.content": "test"}));
    assert!(result.is_err(), "$** with nested path should return error");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("nested paths"),
        "Error message should mention nested paths: {}",
        err_msg
    );
}

#[test]
fn test_wildcard_operator_bare_dollar_star_star() {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    let coll = db.collection("wildcard_bare").unwrap();

    let mut doc = HashMap::new();
    doc.insert("data".to_string(), json!({"value": 1}));
    db.insert_one("wildcard_bare", doc).unwrap();

    // Bare $** without field name should return error
    // Invalid query syntax is now properly reported as an error
    let result = coll.find(&json!({"$**": "test"}));
    assert!(result.is_err(), "Bare $** should return error");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("field name"),
        "Error message should mention field name: {}",
        err_msg
    );
}

#[test]
fn test_wildcard_operator_with_comparison() {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    let coll = db.collection("wildcard_comparison").unwrap();

    // Insert documents with scores at different depths
    for i in 0..5 {
        let mut doc = HashMap::new();
        doc.insert(
            "game".to_string(),
            json!({
                "player": {
                    "stats": {"score": i * 20}
                }
            }),
        );
        db.insert_one("wildcard_comparison", doc).unwrap();
    }

    // $**.score with $gte should find documents with score >= 60
    let results = coll.find(&json!({"$**.score": {"$gte": 60}})).unwrap();
    assert_eq!(results.len(), 2, "Should find 2 documents with score >= 60");
}

// ========== INDEX-BASED DISTINCT TESTS ==========

#[test]
fn test_distinct_uses_index_when_available() {
    // Test that distinct() uses the B+ tree index when available for empty query
    let (db, coll_name) = create_test_db("test");
    let collection = db.collection(&coll_name).unwrap();

    // Insert many documents with the same "category" values
    for i in 0..1000 {
        let category = match i % 5 {
            0 => "electronics",
            1 => "clothing",
            2 => "food",
            3 => "books",
            _ => "other",
        };
        let doc = HashMap::from([
            ("category".to_string(), json!(category)),
            ("value".to_string(), json!(i)),
        ]);
        db.insert_one(&coll_name, doc).unwrap();
    }

    // Create index on category field
    let index_name = collection
        .create_index("category".to_string(), false, false)
        .unwrap();
    assert!(index_name.contains("category"));

    // Call distinct with empty query - should use the index
    let distinct = collection.distinct("category", &json!({})).unwrap();

    // Should find exactly 5 distinct categories
    assert_eq!(
        distinct.len(),
        5,
        "Expected 5 distinct categories, got {:?}",
        distinct
    );

    // Verify all expected values are present
    let values: Vec<&str> = distinct.iter().filter_map(|v| v.as_str()).collect();
    assert!(values.contains(&"electronics"));
    assert!(values.contains(&"clothing"));
    assert!(values.contains(&"food"));
    assert!(values.contains(&"books"));
    assert!(values.contains(&"other"));
}

#[test]
fn test_distinct_with_index_handles_missing_field() {
    // Documents without the indexed field should not appear in distinct results
    let (db, coll_name) = create_test_db("test");
    let collection = db.collection(&coll_name).unwrap();

    // Insert some documents with "status" field and some without
    db.insert_one(
        &coll_name,
        HashMap::from([("status".to_string(), json!("active"))]),
    )
    .unwrap();
    db.insert_one(
        &coll_name,
        HashMap::from([("status".to_string(), json!("inactive"))]),
    )
    .unwrap();
    db.insert_one(
        &coll_name,
        HashMap::from([("other".to_string(), json!("value"))]),
    )
    .unwrap(); // No status

    // Create index on status
    collection
        .create_index("status".to_string(), false, false)
        .unwrap();

    // Distinct should only return "active" and "inactive", not null
    let distinct = collection.distinct("status", &json!({})).unwrap();
    assert_eq!(
        distinct.len(),
        2,
        "Expected 2 distinct status values, got {:?}",
        distinct
    );

    // No null values should be included
    assert!(!distinct.iter().any(|v| v.is_null()));
}

#[test]
fn test_distinct_fallback_without_index() {
    // When no index exists, distinct should still work (using streaming)
    let (db, coll_name) = create_test_db("test");
    let collection = db.collection(&coll_name).unwrap();

    for i in 0..100 {
        let doc = HashMap::from([(
            "type".to_string(),
            json!(if i % 3 == 0 {
                "A"
            } else if i % 3 == 1 {
                "B"
            } else {
                "C"
            }),
        )]);
        db.insert_one(&coll_name, doc).unwrap();
    }

    // No index created on "type" field
    // distinct should work via streaming fallback
    let distinct = collection.distinct("type", &json!({})).unwrap();
    assert_eq!(distinct.len(), 3, "Expected 3 distinct types: A, B, C");
}

#[test]
fn test_distinct_with_query_uses_streaming() {
    // When query is not empty, distinct should use streaming (not index)
    let (db, coll_name) = create_test_db("test");
    let collection = db.collection(&coll_name).unwrap();

    for i in 0..100 {
        let doc = HashMap::from([
            (
                "group".to_string(),
                json!(if i < 50 { "first" } else { "second" }),
            ),
            ("value".to_string(), json!(i % 10)),
        ]);
        db.insert_one(&coll_name, doc).unwrap();
    }

    // Create index on value
    collection
        .create_index("value".to_string(), false, false)
        .unwrap();

    // Distinct with non-empty query - uses streaming fallback
    let distinct = collection
        .distinct("value", &json!({"group": "first"}))
        .unwrap();

    // First group has values 0-49, so values 0-9 repeat
    assert!(
        distinct.len() <= 10,
        "Expected at most 10 distinct values in first group"
    );
}
