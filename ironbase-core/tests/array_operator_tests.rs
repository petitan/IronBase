// array_operator_tests.rs
// Comprehensive tests for array update operators: $push, $pull, $addToSet, $pop

use ironbase_core::DatabaseCore;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;

/// Helper to create a test database
fn setup_test_db(name: &str) -> DatabaseCore<ironbase_core::storage::StorageEngine> {
    let path = format!("test_{}.mlite", name);
    let _ = fs::remove_file(&path); // Clean up if exists
    DatabaseCore::open(&path).expect("Failed to open database")
}

/// Helper to cleanup test database
fn cleanup_test_db(name: &str) {
    let path = format!("test_{}.mlite", name);
    let _ = fs::remove_file(&path);
}

/// Helper to convert JSON to HashMap for insert_one
fn json_to_hashmap(json: Value) -> HashMap<String, Value> {
    if let Value::Object(map) = json {
        map.into_iter().collect()
    } else {
        HashMap::new()
    }
}

// ========== $push TESTS ==========

#[test]
fn test_push_simple() {
    let db = setup_test_db("push_simple");
    let coll = db.collection("test").unwrap();

    // Insert document with array
    coll.insert_one(json_to_hashmap(json!({"_id": 1, "tags": ["rust"]})))
        .unwrap();

    // Push single element
    let (matched, modified) = coll
        .update_one(&json!({"_id": 1}), &json!({"$push": {"tags": "mongodb"}}))
        .unwrap();

    assert_eq!(matched, 1);
    assert_eq!(modified, 1);

    // Verify
    let docs = coll.find(&json!({"_id": 1})).unwrap();
    assert_eq!(docs[0]["tags"], json!(["rust", "mongodb"]));

    cleanup_test_db("push_simple");
}

#[test]
fn test_push_to_nonexistent_field() {
    let db = setup_test_db("push_nonexistent");
    let coll = db.collection("test").unwrap();

    // Insert document without array field
    coll.insert_one(json_to_hashmap(json!({"_id": 1, "name": "test"})))
        .unwrap();

    // Push to nonexistent field (should create array)
    let (matched, modified) = coll
        .update_one(&json!({"_id": 1}), &json!({"$push": {"tags": "new"}}))
        .unwrap();

    assert_eq!(matched, 1);
    assert_eq!(modified, 1);

    // Verify
    let docs = coll.find(&json!({"_id": 1})).unwrap();
    assert_eq!(docs[0]["tags"], json!(["new"]));

    cleanup_test_db("push_nonexistent");
}

#[test]
fn test_push_each() {
    let db = setup_test_db("push_each");
    let coll = db.collection("test").unwrap();

    // Insert document with array
    coll.insert_one(json_to_hashmap(json!({"_id": 1, "scores": [10, 20]})))
        .unwrap();

    // Push multiple elements with $each
    let (matched, modified) = coll
        .update_one(
            &json!({"_id": 1}),
            &json!({"$push": {"scores": {"$each": [30, 40, 50]}}}),
        )
        .unwrap();

    assert_eq!(matched, 1);
    assert_eq!(modified, 1);

    // Verify
    let docs = coll.find(&json!({"_id": 1})).unwrap();
    assert_eq!(docs[0]["scores"], json!([10, 20, 30, 40, 50]));

    cleanup_test_db("push_each");
}

#[test]
fn test_push_position() {
    let db = setup_test_db("push_position");
    let coll = db.collection("test").unwrap();

    // Insert document with array
    coll.insert_one(json_to_hashmap(json!({"_id": 1, "items": ["a", "b", "c"]})))
        .unwrap();

    // Push at position 1
    let (matched, modified) = coll
        .update_one(
            &json!({"_id": 1}),
            &json!({"$push": {"items": {"$each": ["x", "y"], "$position": 1}}}),
        )
        .unwrap();

    assert_eq!(matched, 1);
    assert_eq!(modified, 1);

    // Verify
    let docs = coll.find(&json!({"_id": 1})).unwrap();
    assert_eq!(docs[0]["items"], json!(["a", "x", "y", "b", "c"]));

    cleanup_test_db("push_position");
}

#[test]
fn test_push_slice_positive() {
    let db = setup_test_db("push_slice_pos");
    let coll = db.collection("test").unwrap();

    // Insert document with array
    coll.insert_one(json_to_hashmap(json!({"_id": 1, "items": [1, 2, 3]})))
        .unwrap();

    // Push and keep only first 3 elements
    let (matched, modified) = coll
        .update_one(
            &json!({"_id": 1}),
            &json!({"$push": {"items": {"$each": [4, 5], "$slice": 3}}}),
        )
        .unwrap();

    assert_eq!(matched, 1);
    assert_eq!(modified, 1);

    // Verify
    let docs = coll.find(&json!({"_id": 1})).unwrap();
    assert_eq!(docs[0]["items"], json!([1, 2, 3])); // Only first 3 kept

    cleanup_test_db("push_slice_pos");
}

#[test]
fn test_push_slice_negative() {
    let db = setup_test_db("push_slice_neg");
    let coll = db.collection("test").unwrap();

    // Insert document with array
    coll.insert_one(json_to_hashmap(json!({"_id": 1, "items": [1, 2, 3]})))
        .unwrap();

    // Push and keep only last 3 elements
    let (matched, modified) = coll
        .update_one(
            &json!({"_id": 1}),
            &json!({"$push": {"items": {"$each": [4, 5], "$slice": -3}}}),
        )
        .unwrap();

    assert_eq!(matched, 1);
    assert_eq!(modified, 1);

    // Verify
    let docs = coll.find(&json!({"_id": 1})).unwrap();
    assert_eq!(docs[0]["items"], json!([3, 4, 5])); // Only last 3 kept

    cleanup_test_db("push_slice_neg");
}

#[test]
fn test_push_to_non_array_field_should_error() {
    let db = setup_test_db("push_error");
    let coll = db.collection("test").unwrap();

    // Insert document with non-array field
    coll.insert_one(json_to_hashmap(json!({"_id": 1, "name": "test"})))
        .unwrap();

    // Try to push to non-array field (should fail)
    let result = coll.update_one(&json!({"_id": 1}), &json!({"$push": {"name": "value"}}));

    assert!(result.is_err());

    cleanup_test_db("push_error");
}

// ========== $pull TESTS ==========

#[test]
fn test_pull_simple_equality() {
    let db = setup_test_db("pull_simple");
    let coll = db.collection("test").unwrap();

    // Insert document with array
    coll.insert_one(json_to_hashmap(
        json!({"_id": 1, "tags": ["rust", "python", "rust", "java"]}),
    ))
    .unwrap();

    // Pull all "rust" elements
    let (matched, modified) = coll
        .update_one(&json!({"_id": 1}), &json!({"$pull": {"tags": "rust"}}))
        .unwrap();

    assert_eq!(matched, 1);
    assert_eq!(modified, 1);

    // Verify
    let docs = coll.find(&json!({"_id": 1})).unwrap();
    assert_eq!(docs[0]["tags"], json!(["python", "java"]));

    cleanup_test_db("pull_simple");
}

#[test]
fn test_pull_with_condition() {
    let db = setup_test_db("pull_condition");
    let coll = db.collection("test").unwrap();

    // Insert document with array of numbers
    coll.insert_one(json_to_hashmap(
        json!({"_id": 1, "scores": [5, 10, 15, 20, 25]}),
    ))
    .unwrap();

    // Pull elements less than 15
    let (matched, modified) = coll
        .update_one(
            &json!({"_id": 1}),
            &json!({"$pull": {"scores": {"$lt": 15}}}),
        )
        .unwrap();

    assert_eq!(matched, 1);
    assert_eq!(modified, 1);

    // Verify
    let docs = coll.find(&json!({"_id": 1})).unwrap();
    assert_eq!(docs[0]["scores"], json!([15, 20, 25]));

    cleanup_test_db("pull_condition");
}

#[test]
fn test_pull_with_in_operator() {
    let db = setup_test_db("pull_in");
    let coll = db.collection("test").unwrap();

    // Insert document with array
    coll.insert_one(json_to_hashmap(
        json!({"_id": 1, "tags": ["a", "b", "c", "d", "e"]}),
    ))
    .unwrap();

    // Pull elements in ["b", "d"]
    let (matched, modified) = coll
        .update_one(
            &json!({"_id": 1}),
            &json!({"$pull": {"tags": {"$in": ["b", "d"]}}}),
        )
        .unwrap();

    assert_eq!(matched, 1);
    assert_eq!(modified, 1);

    // Verify
    let docs = coll.find(&json!({"_id": 1})).unwrap();
    assert_eq!(docs[0]["tags"], json!(["a", "c", "e"]));

    cleanup_test_db("pull_in");
}

#[test]
fn test_pull_from_nonexistent_field() {
    let db = setup_test_db("pull_nonexistent");
    let coll = db.collection("test").unwrap();

    // Insert document without array field
    coll.insert_one(json_to_hashmap(json!({"_id": 1, "name": "test"})))
        .unwrap();

    // Pull from nonexistent field (should be no-op)
    let (matched, modified) = coll
        .update_one(&json!({"_id": 1}), &json!({"$pull": {"tags": "value"}}))
        .unwrap();

    assert_eq!(matched, 1);
    assert_eq!(modified, 0); // No modification since field doesn't exist

    cleanup_test_db("pull_nonexistent");
}

#[test]
fn test_pull_from_non_array_field_should_error() {
    let db = setup_test_db("pull_error");
    let coll = db.collection("test").unwrap();

    // Insert document with non-array field
    coll.insert_one(json_to_hashmap(json!({"_id": 1, "name": "test"})))
        .unwrap();

    // Try to pull from non-array field (should fail)
    let result = coll.update_one(&json!({"_id": 1}), &json!({"$pull": {"name": "value"}}));

    assert!(result.is_err());

    cleanup_test_db("pull_error");
}

// ========== $addToSet TESTS ==========

#[test]
fn test_addtoset_simple() {
    let db = setup_test_db("addtoset_simple");
    let coll = db.collection("test").unwrap();

    // Insert document with array
    coll.insert_one(json_to_hashmap(
        json!({"_id": 1, "tags": ["rust", "python"]}),
    ))
    .unwrap();

    // Add unique element
    let (matched, modified) = coll
        .update_one(&json!({"_id": 1}), &json!({"$addToSet": {"tags": "java"}}))
        .unwrap();

    assert_eq!(matched, 1);
    assert_eq!(modified, 1);

    // Verify
    let docs = coll.find(&json!({"_id": 1})).unwrap();
    assert_eq!(docs[0]["tags"], json!(["rust", "python", "java"]));

    cleanup_test_db("addtoset_simple");
}

#[test]
fn test_addtoset_duplicate() {
    let db = setup_test_db("addtoset_dup");
    let coll = db.collection("test").unwrap();

    // Insert document with array
    coll.insert_one(json_to_hashmap(
        json!({"_id": 1, "tags": ["rust", "python"]}),
    ))
    .unwrap();

    // Try to add duplicate element
    let (matched, modified) = coll
        .update_one(&json!({"_id": 1}), &json!({"$addToSet": {"tags": "rust"}}))
        .unwrap();

    assert_eq!(matched, 1);
    assert_eq!(modified, 0); // No modification since "rust" already exists

    // Verify (array unchanged)
    let docs = coll.find(&json!({"_id": 1})).unwrap();
    assert_eq!(docs[0]["tags"], json!(["rust", "python"]));

    cleanup_test_db("addtoset_dup");
}

#[test]
fn test_addtoset_each() {
    let db = setup_test_db("addtoset_each");
    let coll = db.collection("test").unwrap();

    // Insert document with array
    coll.insert_one(json_to_hashmap(
        json!({"_id": 1, "tags": ["rust", "python"]}),
    ))
    .unwrap();

    // Add multiple unique elements with $each
    let (matched, modified) = coll
        .update_one(
            &json!({"_id": 1}),
            &json!({"$addToSet": {"tags": {"$each": ["java", "rust", "go"]}}}),
        )
        .unwrap();

    assert_eq!(matched, 1);
    assert_eq!(modified, 1); // Modified because "java" and "go" were added

    // Verify ("rust" not duplicated)
    let docs = coll.find(&json!({"_id": 1})).unwrap();
    let tags = docs[0]["tags"].as_array().unwrap();
    assert_eq!(tags.len(), 4); // rust, python, java, go
    assert!(tags.contains(&json!("rust")));
    assert!(tags.contains(&json!("python")));
    assert!(tags.contains(&json!("java")));
    assert!(tags.contains(&json!("go")));

    cleanup_test_db("addtoset_each");
}

#[test]
fn test_addtoset_to_nonexistent_field() {
    let db = setup_test_db("addtoset_nonexistent");
    let coll = db.collection("test").unwrap();

    // Insert document without array field
    coll.insert_one(json_to_hashmap(json!({"_id": 1, "name": "test"})))
        .unwrap();

    // AddToSet to nonexistent field (should create array)
    let (matched, modified) = coll
        .update_one(&json!({"_id": 1}), &json!({"$addToSet": {"tags": "new"}}))
        .unwrap();

    assert_eq!(matched, 1);
    assert_eq!(modified, 1);

    // Verify
    let docs = coll.find(&json!({"_id": 1})).unwrap();
    assert_eq!(docs[0]["tags"], json!(["new"]));

    cleanup_test_db("addtoset_nonexistent");
}

#[test]
fn test_addtoset_to_non_array_field_should_error() {
    let db = setup_test_db("addtoset_error");
    let coll = db.collection("test").unwrap();

    // Insert document with non-array field
    coll.insert_one(json_to_hashmap(json!({"_id": 1, "name": "test"})))
        .unwrap();

    // Try to addToSet to non-array field (should fail)
    let result = coll.update_one(&json!({"_id": 1}), &json!({"$addToSet": {"name": "value"}}));

    assert!(result.is_err());

    cleanup_test_db("addtoset_error");
}

// ========== $pop TESTS ==========

#[test]
fn test_pop_first() {
    let db = setup_test_db("pop_first");
    let coll = db.collection("test").unwrap();

    // Insert document with array
    coll.insert_one(json_to_hashmap(json!({"_id": 1, "items": [1, 2, 3, 4, 5]})))
        .unwrap();

    // Pop first element
    let (matched, modified) = coll
        .update_one(&json!({"_id": 1}), &json!({"$pop": {"items": -1}}))
        .unwrap();

    assert_eq!(matched, 1);
    assert_eq!(modified, 1);

    // Verify
    let docs = coll.find(&json!({"_id": 1})).unwrap();
    assert_eq!(docs[0]["items"], json!([2, 3, 4, 5]));

    cleanup_test_db("pop_first");
}

#[test]
fn test_pop_last() {
    let db = setup_test_db("pop_last");
    let coll = db.collection("test").unwrap();

    // Insert document with array
    coll.insert_one(json_to_hashmap(json!({"_id": 1, "items": [1, 2, 3, 4, 5]})))
        .unwrap();

    // Pop last element
    let (matched, modified) = coll
        .update_one(&json!({"_id": 1}), &json!({"$pop": {"items": 1}}))
        .unwrap();

    assert_eq!(matched, 1);
    assert_eq!(modified, 1);

    // Verify
    let docs = coll.find(&json!({"_id": 1})).unwrap();
    assert_eq!(docs[0]["items"], json!([1, 2, 3, 4]));

    cleanup_test_db("pop_last");
}

#[test]
fn test_pop_empty_array() {
    let db = setup_test_db("pop_empty");
    let coll = db.collection("test").unwrap();

    // Insert document with empty array
    coll.insert_one(json_to_hashmap(json!({"_id": 1, "items": []})))
        .unwrap();

    // Pop from empty array (should be no-op)
    let (matched, modified) = coll
        .update_one(&json!({"_id": 1}), &json!({"$pop": {"items": 1}}))
        .unwrap();

    assert_eq!(matched, 1);
    assert_eq!(modified, 0); // No modification on empty array

    // Verify (array still empty)
    let docs = coll.find(&json!({"_id": 1})).unwrap();
    assert_eq!(docs[0]["items"], json!([]));

    cleanup_test_db("pop_empty");
}

#[test]
fn test_pop_invalid_direction() {
    let db = setup_test_db("pop_invalid");
    let coll = db.collection("test").unwrap();

    // Insert document with array
    coll.insert_one(json_to_hashmap(json!({"_id": 1, "items": [1, 2, 3]})))
        .unwrap();

    // Try to pop with invalid direction (not -1 or 1)
    let result = coll.update_one(&json!({"_id": 1}), &json!({"$pop": {"items": 2}}));

    assert!(result.is_err());

    cleanup_test_db("pop_invalid");
}

#[test]
fn test_pop_from_non_array_field_should_error() {
    let db = setup_test_db("pop_error");
    let coll = db.collection("test").unwrap();

    // Insert document with non-array field
    coll.insert_one(json_to_hashmap(json!({"_id": 1, "name": "test"})))
        .unwrap();

    // Try to pop from non-array field (should fail)
    let result = coll.update_one(&json!({"_id": 1}), &json!({"$pop": {"name": 1}}));

    assert!(result.is_err());

    cleanup_test_db("pop_error");
}

// ========== COMBINED OPERATIONS TESTS ==========

#[test]
fn test_combined_array_operations() {
    let db = setup_test_db("combined");
    let coll = db.collection("test").unwrap();

    // Insert document
    coll.insert_one(json_to_hashmap(
        json!({"_id": 1, "tags": ["a", "b"], "scores": [10, 20]}),
    ))
    .unwrap();

    // Apply multiple array operations at once
    let (matched, modified) = coll
        .update_one(
            &json!({"_id": 1}),
            &json!({
                "$push": {"tags": "c"},
                "$addToSet": {"scores": 30}
            }),
        )
        .unwrap();

    assert_eq!(matched, 1);
    assert_eq!(modified, 1);

    // Verify both operations applied
    let docs = coll.find(&json!({"_id": 1})).unwrap();
    assert_eq!(docs[0]["tags"], json!(["a", "b", "c"]));
    assert_eq!(docs[0]["scores"], json!([10, 20, 30]));

    cleanup_test_db("combined");
}

#[test]
fn test_update_many_with_array_operators() {
    let db = setup_test_db("update_many_array");
    let coll = db.collection("test").unwrap();

    // Insert multiple documents
    coll.insert_one(json_to_hashmap(
        json!({"_id": 1, "category": "A", "tags": ["old"]}),
    ))
    .unwrap();
    coll.insert_one(json_to_hashmap(
        json!({"_id": 2, "category": "A", "tags": ["old"]}),
    ))
    .unwrap();
    coll.insert_one(json_to_hashmap(
        json!({"_id": 3, "category": "B", "tags": ["old"]}),
    ))
    .unwrap();

    // Update all category A documents
    let (matched, modified) = coll
        .update_many(
            &json!({"category": "A"}),
            &json!({"$push": {"tags": "new"}}),
        )
        .unwrap();

    assert_eq!(matched, 2);
    assert_eq!(modified, 2);

    // Verify
    let docs = coll.find(&json!({"category": "A"})).unwrap();
    assert_eq!(docs.len(), 2);
    for doc in docs {
        assert_eq!(doc["tags"], json!(["old", "new"]));
    }

    cleanup_test_db("update_many_array");
}
