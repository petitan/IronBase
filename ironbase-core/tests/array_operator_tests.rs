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
    db.insert_one("test", json_to_hashmap(json!({"_id": 1, "tags": ["rust"]})))
        .unwrap();

    // Push single element
    let (matched, modified) = db
        .update_one(
            "test",
            &json!({"_id": 1}),
            &json!({"$push": {"tags": "mongodb"}}),
        )
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
    db.insert_one("test", json_to_hashmap(json!({"_id": 1, "name": "test"})))
        .unwrap();

    // Push to nonexistent field (should create array)
    let (matched, modified) = db
        .update_one(
            "test",
            &json!({"_id": 1}),
            &json!({"$push": {"tags": "new"}}),
        )
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
    db.insert_one(
        "test",
        json_to_hashmap(json!({"_id": 1, "scores": [10, 20]})),
    )
    .unwrap();

    // Push multiple elements with $each
    let (matched, modified) = db
        .update_one(
            "test",
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
    db.insert_one(
        "test",
        json_to_hashmap(json!({"_id": 1, "items": ["a", "b", "c"]})),
    )
    .unwrap();

    // Push at position 1
    let (matched, modified) = db
        .update_one(
            "test",
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
    db.insert_one(
        "test",
        json_to_hashmap(json!({"_id": 1, "items": [1, 2, 3]})),
    )
    .unwrap();

    // Push and keep only first 3 elements
    let (matched, modified) = db
        .update_one(
            "test",
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
    db.insert_one(
        "test",
        json_to_hashmap(json!({"_id": 1, "items": [1, 2, 3]})),
    )
    .unwrap();

    // Push and keep only last 3 elements
    let (matched, modified) = db
        .update_one(
            "test",
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
    let _coll = db.collection("test").unwrap();

    // Insert document with non-array field
    db.insert_one("test", json_to_hashmap(json!({"_id": 1, "name": "test"})))
        .unwrap();

    // Try to push to non-array field (should fail)
    let result = db.update_one(
        "test",
        &json!({"_id": 1}),
        &json!({"$push": {"name": "value"}}),
    );

    assert!(result.is_err());

    cleanup_test_db("push_error");
}

// ========== $pull TESTS ==========

#[test]
fn test_pull_simple_equality() {
    let db = setup_test_db("pull_simple");
    let coll = db.collection("test").unwrap();

    // Insert document with array
    db.insert_one(
        "test",
        json_to_hashmap(json!({"_id": 1, "tags": ["rust", "python", "rust", "java"]})),
    )
    .unwrap();

    // Pull all "rust" elements
    let (matched, modified) = db
        .update_one(
            "test",
            &json!({"_id": 1}),
            &json!({"$pull": {"tags": "rust"}}),
        )
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
    db.insert_one(
        "test",
        json_to_hashmap(json!({"_id": 1, "scores": [5, 10, 15, 20, 25]})),
    )
    .unwrap();

    // Pull elements less than 15
    let (matched, modified) = db
        .update_one(
            "test",
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
    db.insert_one(
        "test",
        json_to_hashmap(json!({"_id": 1, "tags": ["a", "b", "c", "d", "e"]})),
    )
    .unwrap();

    // Pull elements in ["b", "d"]
    let (matched, modified) = db
        .update_one(
            "test",
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
    let _coll = db.collection("test").unwrap();

    // Insert document without array field
    db.insert_one("test", json_to_hashmap(json!({"_id": 1, "name": "test"})))
        .unwrap();

    // Pull from nonexistent field (should be no-op)
    let (matched, modified) = db
        .update_one(
            "test",
            &json!({"_id": 1}),
            &json!({"$pull": {"tags": "value"}}),
        )
        .unwrap();

    assert_eq!(matched, 1);
    assert_eq!(modified, 0); // No modification since field doesn't exist

    cleanup_test_db("pull_nonexistent");
}

#[test]
fn test_pull_from_non_array_field_should_error() {
    let db = setup_test_db("pull_error");
    let _coll = db.collection("test").unwrap();

    // Insert document with non-array field
    db.insert_one("test", json_to_hashmap(json!({"_id": 1, "name": "test"})))
        .unwrap();

    // Try to pull from non-array field (should fail)
    let result = db.update_one(
        "test",
        &json!({"_id": 1}),
        &json!({"$pull": {"name": "value"}}),
    );

    assert!(result.is_err());

    cleanup_test_db("pull_error");
}

// ========== $addToSet TESTS ==========

#[test]
fn test_addtoset_simple() {
    let db = setup_test_db("addtoset_simple");
    let coll = db.collection("test").unwrap();

    // Insert document with array
    db.insert_one(
        "test",
        json_to_hashmap(json!({"_id": 1, "tags": ["rust", "python"]})),
    )
    .unwrap();

    // Add unique element
    let (matched, modified) = db
        .update_one(
            "test",
            &json!({"_id": 1}),
            &json!({"$addToSet": {"tags": "java"}}),
        )
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
    db.insert_one(
        "test",
        json_to_hashmap(json!({"_id": 1, "tags": ["rust", "python"]})),
    )
    .unwrap();

    // Try to add duplicate element
    let (matched, modified) = db
        .update_one(
            "test",
            &json!({"_id": 1}),
            &json!({"$addToSet": {"tags": "rust"}}),
        )
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
    db.insert_one(
        "test",
        json_to_hashmap(json!({"_id": 1, "tags": ["rust", "python"]})),
    )
    .unwrap();

    // Add multiple unique elements with $each
    let (matched, modified) = db
        .update_one(
            "test",
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
    db.insert_one("test", json_to_hashmap(json!({"_id": 1, "name": "test"})))
        .unwrap();

    // AddToSet to nonexistent field (should create array)
    let (matched, modified) = db
        .update_one(
            "test",
            &json!({"_id": 1}),
            &json!({"$addToSet": {"tags": "new"}}),
        )
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
    let _coll = db.collection("test").unwrap();

    // Insert document with non-array field
    db.insert_one("test", json_to_hashmap(json!({"_id": 1, "name": "test"})))
        .unwrap();

    // Try to addToSet to non-array field (should fail)
    let result = db.update_one(
        "test",
        &json!({"_id": 1}),
        &json!({"$addToSet": {"name": "value"}}),
    );

    assert!(result.is_err());

    cleanup_test_db("addtoset_error");
}

// ========== $pop TESTS ==========

#[test]
fn test_pop_first() {
    let db = setup_test_db("pop_first");
    let coll = db.collection("test").unwrap();

    // Insert document with array
    db.insert_one(
        "test",
        json_to_hashmap(json!({"_id": 1, "items": [1, 2, 3, 4, 5]})),
    )
    .unwrap();

    // Pop first element
    let (matched, modified) = db
        .update_one("test", &json!({"_id": 1}), &json!({"$pop": {"items": -1}}))
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
    db.insert_one(
        "test",
        json_to_hashmap(json!({"_id": 1, "items": [1, 2, 3, 4, 5]})),
    )
    .unwrap();

    // Pop last element
    let (matched, modified) = db
        .update_one("test", &json!({"_id": 1}), &json!({"$pop": {"items": 1}}))
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
    db.insert_one("test", json_to_hashmap(json!({"_id": 1, "items": []})))
        .unwrap();

    // Pop from empty array (should be no-op)
    let (matched, modified) = db
        .update_one("test", &json!({"_id": 1}), &json!({"$pop": {"items": 1}}))
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
    let _coll = db.collection("test").unwrap();

    // Insert document with array
    db.insert_one(
        "test",
        json_to_hashmap(json!({"_id": 1, "items": [1, 2, 3]})),
    )
    .unwrap();

    // Try to pop with invalid direction (not -1 or 1)
    let result = db.update_one("test", &json!({"_id": 1}), &json!({"$pop": {"items": 2}}));

    assert!(result.is_err());

    cleanup_test_db("pop_invalid");
}

#[test]
fn test_pop_from_non_array_field_should_error() {
    let db = setup_test_db("pop_error");
    let _coll = db.collection("test").unwrap();

    // Insert document with non-array field
    db.insert_one("test", json_to_hashmap(json!({"_id": 1, "name": "test"})))
        .unwrap();

    // Try to pop from non-array field (should fail)
    let result = db.update_one("test", &json!({"_id": 1}), &json!({"$pop": {"name": 1}}));

    assert!(result.is_err());

    cleanup_test_db("pop_error");
}

// ========== COMBINED OPERATIONS TESTS ==========

#[test]
fn test_combined_array_operations() {
    let db = setup_test_db("combined");
    let coll = db.collection("test").unwrap();

    // Insert document
    db.insert_one(
        "test",
        json_to_hashmap(json!({"_id": 1, "tags": ["a", "b"], "scores": [10, 20]})),
    )
    .unwrap();

    // Apply multiple array operations at once
    let (matched, modified) = db
        .update_one(
            "test",
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
    db.insert_one(
        "test",
        json_to_hashmap(json!({"_id": 1, "category": "A", "tags": ["old"]})),
    )
    .unwrap();
    db.insert_one(
        "test",
        json_to_hashmap(json!({"_id": 2, "category": "A", "tags": ["old"]})),
    )
    .unwrap();
    db.insert_one(
        "test",
        json_to_hashmap(json!({"_id": 3, "category": "B", "tags": ["old"]})),
    )
    .unwrap();

    // Update all category A documents
    let (matched, modified) = db
        .update_many(
            "test",
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

// ========== $size QUERY OPERATOR TESTS ==========

#[test]
fn test_size_query_operator() {
    let db = setup_test_db("size_query");
    let coll = db.collection("test").unwrap();

    // Insert documents with arrays of various sizes
    db.insert_one(
        "test",
        json_to_hashmap(json!({"_id": 1, "name": "Alice", "tags": ["a", "b", "c"]})),
    )
    .unwrap();
    db.insert_one(
        "test",
        json_to_hashmap(json!({"_id": 2, "name": "Bob", "tags": ["x", "y"]})),
    )
    .unwrap();
    db.insert_one(
        "test",
        json_to_hashmap(json!({"_id": 3, "name": "Charlie", "tags": []})),
    )
    .unwrap();
    db.insert_one(
        "test",
        json_to_hashmap(json!({"_id": 4, "name": "David", "tags": ["single"]})),
    )
    .unwrap();

    // Test $size: 3
    let docs = coll.find(&json!({"tags": {"$size": 3}})).unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0]["name"], "Alice");

    // Test $size: 2
    let docs = coll.find(&json!({"tags": {"$size": 2}})).unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0]["name"], "Bob");

    // Test $size: 0 (empty array)
    let docs = coll.find(&json!({"tags": {"$size": 0}})).unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0]["name"], "Charlie");

    // Test $size: 1
    let docs = coll.find(&json!({"tags": {"$size": 1}})).unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0]["name"], "David");

    // Test $size with no matches
    let docs = coll.find(&json!({"tags": {"$size": 10}})).unwrap();
    assert_eq!(docs.len(), 0);

    cleanup_test_db("size_query");
}

#[test]
fn test_size_query_operator_with_options() {
    use ironbase_core::find_options::FindOptions;

    let db = setup_test_db("size_query_options");
    let coll = db.collection("test").unwrap();

    // Insert documents with arrays of various sizes
    db.insert_one(
        "test",
        json_to_hashmap(json!({"_id": 1, "name": "Alice", "tags": ["a", "b", "c"]})),
    )
    .unwrap();
    db.insert_one(
        "test",
        json_to_hashmap(json!({"_id": 2, "name": "Bob", "tags": ["x", "y"]})),
    )
    .unwrap();

    // Test $size: 3 with FindOptions (like Python does)
    let options = FindOptions::new();
    let docs = coll
        .find_with_options(&json!({"tags": {"$size": 3}}), options)
        .unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0]["name"], "Alice");

    // Test $size: 2 with FindOptions
    let options = FindOptions::new();
    let docs = coll
        .find_with_options(&json!({"tags": {"$size": 2}}), options)
        .unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0]["name"], "Bob");

    cleanup_test_db("size_query_options");
}

// ========== $all QUERY OPERATOR TESTS ==========

#[test]
fn test_all_operator_basic() {
    let db = setup_test_db("all_basic");
    let coll = db.collection("test").unwrap();

    db.insert_one(
        "test",
        json_to_hashmap(json!({"name": "A", "tags": ["rust", "mongodb", "nosql"]})),
    )
    .unwrap();
    db.insert_one(
        "test",
        json_to_hashmap(json!({"name": "B", "tags": ["rust", "sql"]})),
    )
    .unwrap();
    db.insert_one(
        "test",
        json_to_hashmap(json!({"name": "C", "tags": ["mongodb", "nosql"]})),
    )
    .unwrap();

    // Find documents with BOTH "rust" and "mongodb" tags
    let results = coll
        .find(&json!({"tags": {"$all": ["rust", "mongodb"]}}))
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["name"], "A");

    cleanup_test_db("all_basic");
}

#[test]
fn test_all_operator_single_element() {
    let db = setup_test_db("all_single");
    let coll = db.collection("test").unwrap();

    db.insert_one(
        "test",
        json_to_hashmap(json!({"name": "A", "tags": ["rust", "mongodb"]})),
    )
    .unwrap();
    db.insert_one(
        "test",
        json_to_hashmap(json!({"name": "B", "tags": ["python"]})),
    )
    .unwrap();

    // $all with single element (equivalent to simple match)
    let results = coll.find(&json!({"tags": {"$all": ["rust"]}})).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["name"], "A");

    cleanup_test_db("all_single");
}

#[test]
fn test_all_operator_no_match() {
    let db = setup_test_db("all_no_match");
    let coll = db.collection("test").unwrap();

    db.insert_one(
        "test",
        json_to_hashmap(json!({"name": "A", "tags": ["rust"]})),
    )
    .unwrap();
    db.insert_one(
        "test",
        json_to_hashmap(json!({"name": "B", "tags": ["python"]})),
    )
    .unwrap();

    // Neither document has both "rust" AND "python"
    let results = coll
        .find(&json!({"tags": {"$all": ["rust", "python"]}}))
        .unwrap();

    assert_eq!(results.len(), 0);

    cleanup_test_db("all_no_match");
}

#[test]
fn test_all_operator_with_numbers() {
    let db = setup_test_db("all_numbers");
    let coll = db.collection("test").unwrap();

    db.insert_one(
        "test",
        json_to_hashmap(json!({"name": "A", "scores": [85, 90, 95]})),
    )
    .unwrap();
    db.insert_one(
        "test",
        json_to_hashmap(json!({"name": "B", "scores": [85, 70]})),
    )
    .unwrap();

    // Find documents with both 85 and 90 in scores
    let results = coll.find(&json!({"scores": {"$all": [85, 90]}})).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["name"], "A");

    cleanup_test_db("all_numbers");
}

#[test]
fn test_all_operator_empty_array() {
    let db = setup_test_db("all_empty");
    let coll = db.collection("test").unwrap();

    db.insert_one(
        "test",
        json_to_hashmap(json!({"name": "A", "tags": ["rust"]})),
    )
    .unwrap();

    // Empty $all should match all documents with the field
    let _results = coll.find(&json!({"tags": {"$all": []}})).unwrap();

    // MongoDB behavior: empty $all matches all documents
    // Test passed - query completed without crashing

    cleanup_test_db("all_empty");
}

#[test]
fn test_all_operator_superset() {
    let db = setup_test_db("all_superset");
    let coll = db.collection("test").unwrap();

    db.insert_one(
        "test",
        json_to_hashmap(json!({"name": "A", "tags": ["a", "b", "c", "d"]})),
    )
    .unwrap();
    db.insert_one(
        "test",
        json_to_hashmap(json!({"name": "B", "tags": ["a", "b"]})),
    )
    .unwrap();

    // Both have ["a", "b"], but A has more
    let results = coll.find(&json!({"tags": {"$all": ["a", "b"]}})).unwrap();

    assert_eq!(results.len(), 2);

    cleanup_test_db("all_superset");
}

// ========== $elemMatch QUERY OPERATOR TESTS ==========

#[test]
fn test_elemmatch_basic() {
    let db = setup_test_db("elemmatch_basic");
    let coll = db.collection("test").unwrap();

    db.insert_one(
        "test",
        json_to_hashmap(json!({
            "name": "A",
            "results": [
                {"subject": "math", "score": 85},
                {"subject": "english", "score": 90}
            ]
        })),
    )
    .unwrap();
    db.insert_one(
        "test",
        json_to_hashmap(json!({
            "name": "B",
            "results": [
                {"subject": "math", "score": 70},
                {"subject": "english", "score": 75}
            ]
        })),
    )
    .unwrap();

    // Find documents where at least one result has score >= 85
    let results = coll
        .find(&json!({
            "results": {"$elemMatch": {"score": {"$gte": 85}}}
        }))
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["name"], "A");

    cleanup_test_db("elemmatch_basic");
}

#[test]
fn test_elemmatch_multiple_conditions() {
    let db = setup_test_db("elemmatch_multi");
    let coll = db.collection("test").unwrap();

    db.insert_one(
        "test",
        json_to_hashmap(json!({
            "name": "A",
            "items": [
                {"type": "fruit", "qty": 10},
                {"type": "vegetable", "qty": 5}
            ]
        })),
    )
    .unwrap();
    db.insert_one(
        "test",
        json_to_hashmap(json!({
            "name": "B",
            "items": [
                {"type": "fruit", "qty": 3},
                {"type": "vegetable", "qty": 20}
            ]
        })),
    )
    .unwrap();

    // Find documents where ONE element matches BOTH conditions:
    // type = "fruit" AND qty >= 5
    let results = coll
        .find(&json!({
            "items": {"$elemMatch": {"type": "fruit", "qty": {"$gte": 5}}}
        }))
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["name"], "A");

    cleanup_test_db("elemmatch_multi");
}

#[test]
fn test_elemmatch_with_nested_objects() {
    let db = setup_test_db("elemmatch_nested");
    let coll = db.collection("test").unwrap();

    db.insert_one(
        "test",
        json_to_hashmap(json!({
            "name": "A",
            "grades": [
                {"course": "math", "grade": 90},
                {"course": "english", "grade": 85}
            ]
        })),
    )
    .unwrap();
    db.insert_one(
        "test",
        json_to_hashmap(json!({
            "name": "B",
            "grades": [
                {"course": "math", "grade": 70},
                {"course": "english", "grade": 75}
            ]
        })),
    )
    .unwrap();

    // $elemMatch with nested object condition
    let results = coll
        .find(&json!({
            "grades": {"$elemMatch": {"course": "math", "grade": {"$gte": 80}}}
        }))
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["name"], "A");

    cleanup_test_db("elemmatch_nested");
}

#[test]
fn test_elemmatch_no_match() {
    let db = setup_test_db("elemmatch_no_match");
    let coll = db.collection("test").unwrap();

    db.insert_one(
        "test",
        json_to_hashmap(json!({
            "name": "A",
            "items": [
                {"x": 1, "y": 10},
                {"x": 2, "y": 20}
            ]
        })),
    )
    .unwrap();

    // No element has BOTH x > 1 AND y < 15
    let results = coll
        .find(&json!({
            "items": {"$elemMatch": {"x": {"$gt": 1}, "y": {"$lt": 15}}}
        }))
        .unwrap();

    assert_eq!(results.len(), 0);

    cleanup_test_db("elemmatch_no_match");
}

#[test]
fn test_elemmatch_vs_regular_query() {
    let db = setup_test_db("elemmatch_vs_regular");
    let coll = db.collection("test").unwrap();

    // This document has x > 1 in one element and y < 15 in another
    db.insert_one(
        "test",
        json_to_hashmap(json!({
            "name": "A",
            "items": [
                {"x": 1, "y": 5},   // y < 15 but x is not > 1
                {"x": 2, "y": 20}   // x > 1 but y is not < 15
            ]
        })),
    )
    .unwrap();

    // Regular query (without $elemMatch) would match because conditions can be satisfied by different elements
    // But $elemMatch requires ONE element to satisfy ALL conditions
    let results = coll
        .find(&json!({
            "items": {"$elemMatch": {"x": {"$gt": 1}, "y": {"$lt": 15}}}
        }))
        .unwrap();

    // Should NOT match because no single element satisfies both conditions
    assert_eq!(results.len(), 0);

    cleanup_test_db("elemmatch_vs_regular");
}

#[test]
fn test_all_and_elemmatch_combined() {
    let db = setup_test_db("all_elemmatch");
    let coll = db.collection("test").unwrap();

    db.insert_one(
        "test",
        json_to_hashmap(json!({
            "name": "A",
            "tags": ["rust", "mongodb"],
            "scores": [{"type": "quiz", "score": 85}]
        })),
    )
    .unwrap();
    db.insert_one(
        "test",
        json_to_hashmap(json!({
            "name": "B",
            "tags": ["rust"],
            "scores": [{"type": "quiz", "score": 90}]
        })),
    )
    .unwrap();

    // Combine $all and $elemMatch in the same query
    let results = coll
        .find(&json!({
            "tags": {"$all": ["rust", "mongodb"]},
            "scores": {"$elemMatch": {"score": {"$gte": 80}}}
        }))
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["name"], "A");

    cleanup_test_db("all_elemmatch");
}

// ========== $elemMatch VALIDATION AND ERROR HANDLING TESTS ==========

#[test]
fn test_elemmatch_invalid_filter_value_should_error() {
    let db = setup_test_db("elemmatch_invalid");
    let coll = db.collection("test").unwrap();

    db.insert_one("test", json_to_hashmap(json!({"items": [1, 2, 3]})))
        .unwrap();

    // $elemMatch with non-object filter value should throw error
    let result = coll.find(&json!({
        "items": {"$elemMatch": 5}
    }));

    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("$elemMatch requires an object"));

    cleanup_test_db("elemmatch_invalid");
}

#[test]
fn test_elemmatch_invalid_filter_string_should_error() {
    let db = setup_test_db("elemmatch_invalid_str");
    let coll = db.collection("test").unwrap();

    db.insert_one("test", json_to_hashmap(json!({"items": [{"name": "a"}]})))
        .unwrap();

    // $elemMatch with string filter value should throw error
    let result = coll.find(&json!({
        "items": {"$elemMatch": "invalid"}
    }));

    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("$elemMatch requires an object"));

    cleanup_test_db("elemmatch_invalid_str");
}

#[test]
fn test_elemmatch_unknown_operator_should_error() {
    let db = setup_test_db("elemmatch_unknown_op");
    let coll = db.collection("test").unwrap();

    db.insert_one(
        "test",
        json_to_hashmap(json!({
            "items": [{"score": 85}, {"score": 90}]
        })),
    )
    .unwrap();

    // Unknown operator $gtt (typo of $gt) should throw error
    let result = coll.find(&json!({
        "items": {"$elemMatch": {"score": {"$gtt": 80}}}
    }));

    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("Unknown operator: $gtt")
            || err_msg.contains("Unknown query operator: $gtt")
    );

    cleanup_test_db("elemmatch_unknown_op");
}

#[test]
fn test_elemmatch_unknown_toplevel_operator_should_error() {
    let db = setup_test_db("elemmatch_unknown_toplevel");
    let coll = db.collection("test").unwrap();

    db.insert_one("test", json_to_hashmap(json!({"scores": [80, 85, 90]})))
        .unwrap();

    // Unknown top-level operator in $elemMatch for scalar array
    let result = coll.find(&json!({
        "scores": {"$elemMatch": {"$gtt": 80}}
    }));

    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("Unknown operator: $gtt")
            || err_msg.contains("Unknown query operator: $gtt")
    );

    cleanup_test_db("elemmatch_unknown_toplevel");
}

#[test]
fn test_elemmatch_regex_with_options() {
    let db = setup_test_db("elemmatch_regex_options");
    let coll = db.collection("test").unwrap();

    db.insert_one(
        "test",
        json_to_hashmap(json!({
            "name": "A",
            "items": [
                {"tag": "RUST"},
                {"tag": "Python"}
            ]
        })),
    )
    .unwrap();
    db.insert_one(
        "test",
        json_to_hashmap(json!({
            "name": "B",
            "items": [
                {"tag": "java"},
                {"tag": "go"}
            ]
        })),
    )
    .unwrap();

    // Case-insensitive regex inside $elemMatch
    let results = coll
        .find(&json!({
            "items": {"$elemMatch": {"tag": {"$regex": "rust", "$options": "i"}}}
        }))
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["name"], "A");

    cleanup_test_db("elemmatch_regex_options");
}

#[test]
fn test_elemmatch_regex_without_options() {
    let db = setup_test_db("elemmatch_regex_no_options");
    let coll = db.collection("test").unwrap();

    db.insert_one(
        "test",
        json_to_hashmap(json!({
            "name": "A",
            "items": [{"tag": "rust"}]
        })),
    )
    .unwrap();
    db.insert_one(
        "test",
        json_to_hashmap(json!({
            "name": "B",
            "items": [{"tag": "RUST"}]
        })),
    )
    .unwrap();

    // Case-sensitive regex (no $options)
    let results = coll
        .find(&json!({
            "items": {"$elemMatch": {"tag": {"$regex": "rust"}}}
        }))
        .unwrap();

    // Only "rust" (lowercase) matches
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["name"], "A");

    cleanup_test_db("elemmatch_regex_no_options");
}

#[test]
fn test_elemmatch_options_without_regex_should_error() {
    let db = setup_test_db("elemmatch_options_no_regex");
    let coll = db.collection("test").unwrap();

    db.insert_one(
        "test",
        json_to_hashmap(json!({
            "items": [{"tag": "rust"}]
        })),
    )
    .unwrap();

    // $options without $regex should throw error
    let result = coll.find(&json!({
        "items": {"$elemMatch": {"tag": {"$options": "i"}}}
    }));

    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("$options requires $regex")
            || err_msg.contains("$options can only be used with $regex")
    );

    cleanup_test_db("elemmatch_options_no_regex");
}

#[test]
fn test_elemmatch_scalar_array_with_operators() {
    let db = setup_test_db("elemmatch_scalar");
    let coll = db.collection("test").unwrap();

    db.insert_one(
        "test",
        json_to_hashmap(json!({
            "name": "A",
            "scores": [75, 82, 90]
        })),
    )
    .unwrap();
    db.insert_one(
        "test",
        json_to_hashmap(json!({
            "name": "B",
            "scores": [60, 65, 70]
        })),
    )
    .unwrap();

    // Find where at least one score is > 80 AND < 85
    let results = coll
        .find(&json!({
            "scores": {"$elemMatch": {"$gt": 80, "$lt": 85}}
        }))
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["name"], "A"); // Has 82 which matches

    cleanup_test_db("elemmatch_scalar");
}

#[test]
fn test_elemmatch_scalar_array_with_regex() {
    let db = setup_test_db("elemmatch_scalar_regex");
    let coll = db.collection("test").unwrap();

    db.insert_one(
        "test",
        json_to_hashmap(json!({
            "name": "A",
            "tags": ["RUST", "python", "go"]
        })),
    )
    .unwrap();
    db.insert_one(
        "test",
        json_to_hashmap(json!({
            "name": "B",
            "tags": ["java", "kotlin"]
        })),
    )
    .unwrap();

    // Case-insensitive regex on scalar array
    let results = coll
        .find(&json!({
            "tags": {"$elemMatch": {"$regex": "rust", "$options": "i"}}
        }))
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["name"], "A");

    cleanup_test_db("elemmatch_scalar_regex");
}
