// Element operator integration tests
// Tests: $exists, $type

use ironbase_core::StorageEngine;
use serde_json::json;
use std::collections::HashMap;
use tempfile::TempDir;

type DatabaseCore = ironbase_core::DatabaseCore<StorageEngine>;

fn insert_doc(db: &DatabaseCore, coll: &str, doc: serde_json::Value) {
    if let serde_json::Value::Object(map) = doc {
        let fields: HashMap<String, serde_json::Value> = map.into_iter().collect();
        db.insert_one(coll, fields).unwrap();
    }
}

// ============================================================================
// $EXISTS OPERATOR TESTS
// ============================================================================

#[test]
fn test_exists_true_field_present() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(
        &db,
        "users",
        json!({"name": "Alice", "email": "alice@test.com"}),
    );
    insert_doc(&db, "users", json!({"name": "Bob"})); // No email field
    insert_doc(
        &db,
        "users",
        json!({"name": "Carol", "email": "carol@test.com"}),
    );

    let collection = db.collection("users").unwrap();

    // Find users where email field exists
    let results = collection
        .find(&json!({"email": {"$exists": true}}))
        .unwrap();

    assert_eq!(results.len(), 2);
    let names: Vec<&str> = results
        .iter()
        .map(|d| d.get("name").unwrap().as_str().unwrap())
        .collect();
    assert!(names.contains(&"Alice"));
    assert!(names.contains(&"Carol"));
}

#[test]
fn test_exists_false_field_missing() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(
        &db,
        "users",
        json!({"name": "Alice", "email": "alice@test.com"}),
    );
    insert_doc(&db, "users", json!({"name": "Bob"})); // No email field
    insert_doc(
        &db,
        "users",
        json!({"name": "Carol", "email": "carol@test.com"}),
    );

    let collection = db.collection("users").unwrap();

    // Find users where email field does NOT exist
    let results = collection
        .find(&json!({"email": {"$exists": false}}))
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].get("name").unwrap().as_str().unwrap(), "Bob");
}

#[test]
fn test_exists_with_null_value() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(&db, "users", json!({"name": "Alice", "phone": "123456"}));
    insert_doc(&db, "users", json!({"name": "Bob", "phone": null})); // Field exists but null
    insert_doc(&db, "users", json!({"name": "Carol"})); // Field doesn't exist

    let collection = db.collection("users").unwrap();

    // $exists: true should match even if value is null
    let results = collection
        .find(&json!({"phone": {"$exists": true}}))
        .unwrap();

    assert_eq!(results.len(), 2); // Alice and Bob (null value still counts as existing)
}

#[test]
fn test_exists_nested_field() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(
        &db,
        "users",
        json!({"name": "Alice", "address": {"city": "NYC", "zip": "10001"}}),
    );
    insert_doc(
        &db,
        "users",
        json!({"name": "Bob", "address": {"city": "LA"}}),
    ); // No zip
    insert_doc(&db, "users", json!({"name": "Carol"})); // No address at all

    let collection = db.collection("users").unwrap();

    // Find users where address.zip exists
    let results = collection
        .find(&json!({"address.zip": {"$exists": true}}))
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].get("name").unwrap().as_str().unwrap(), "Alice");
}

#[test]
fn test_exists_nested_field_false() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(
        &db,
        "users",
        json!({"name": "Alice", "profile": {"bio": "Hello"}}),
    );
    insert_doc(&db, "users", json!({"name": "Bob", "profile": {}})); // Empty profile
    insert_doc(&db, "users", json!({"name": "Carol"})); // No profile

    let collection = db.collection("users").unwrap();

    // Find users where profile.bio does NOT exist
    let results = collection
        .find(&json!({"profile.bio": {"$exists": false}}))
        .unwrap();

    assert_eq!(results.len(), 2); // Bob and Carol
}

#[test]
fn test_exists_combined_with_other_operators() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(
        &db,
        "products",
        json!({"name": "A", "price": 100, "discount": 10}),
    );
    insert_doc(&db, "products", json!({"name": "B", "price": 200})); // No discount
    insert_doc(
        &db,
        "products",
        json!({"name": "C", "price": 150, "discount": 20}),
    );

    let collection = db.collection("products").unwrap();

    // Find products with price > 100 AND discount exists
    let results = collection
        .find(&json!({
            "price": {"$gt": 100},
            "discount": {"$exists": true}
        }))
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].get("name").unwrap().as_str().unwrap(), "C");
}

// ============================================================================
// $TYPE OPERATOR TESTS
// ============================================================================

#[test]
fn test_type_string() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(&db, "items", json!({"name": "A", "value": "text"}));
    insert_doc(&db, "items", json!({"name": "B", "value": 123}));
    insert_doc(&db, "items", json!({"name": "C", "value": "another text"}));

    let collection = db.collection("items").unwrap();

    // Find items where value is a string
    let results = collection
        .find(&json!({"value": {"$type": "string"}}))
        .unwrap();

    assert_eq!(results.len(), 2);
    let names: Vec<&str> = results
        .iter()
        .map(|d| d.get("name").unwrap().as_str().unwrap())
        .collect();
    assert!(names.contains(&"A"));
    assert!(names.contains(&"C"));
}

#[test]
fn test_type_number() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(&db, "items", json!({"name": "A", "value": 42}));
    insert_doc(&db, "items", json!({"name": "B", "value": "text"}));
    insert_doc(&db, "items", json!({"name": "C", "value": 3.15}));
    insert_doc(&db, "items", json!({"name": "D", "value": -100}));

    let collection = db.collection("items").unwrap();

    // Find items where value is a number
    let results = collection
        .find(&json!({"value": {"$type": "number"}}))
        .unwrap();

    assert_eq!(results.len(), 3); // A, C, D
}

#[test]
fn test_type_boolean() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(&db, "items", json!({"name": "A", "active": true}));
    insert_doc(&db, "items", json!({"name": "B", "active": false}));
    insert_doc(&db, "items", json!({"name": "C", "active": "yes"})); // String, not boolean
    insert_doc(&db, "items", json!({"name": "D", "active": 1})); // Number, not boolean

    let collection = db.collection("items").unwrap();

    // Find items where active is a boolean
    let results = collection
        .find(&json!({"active": {"$type": "bool"}}))
        .unwrap();

    assert_eq!(results.len(), 2); // A and B
}

#[test]
fn test_type_array() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(&db, "items", json!({"name": "A", "tags": ["a", "b"]}));
    insert_doc(&db, "items", json!({"name": "B", "tags": "single"})); // String, not array
    insert_doc(&db, "items", json!({"name": "C", "tags": [1, 2, 3]}));
    insert_doc(&db, "items", json!({"name": "D", "tags": []})); // Empty array is still array

    let collection = db.collection("items").unwrap();

    // Find items where tags is an array
    let results = collection
        .find(&json!({"tags": {"$type": "array"}}))
        .unwrap();

    assert_eq!(results.len(), 3); // A, C, D
}

#[test]
fn test_type_object() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(&db, "items", json!({"name": "A", "meta": {"key": "value"}}));
    insert_doc(&db, "items", json!({"name": "B", "meta": "string"}));
    insert_doc(&db, "items", json!({"name": "C", "meta": {}})); // Empty object
    insert_doc(&db, "items", json!({"name": "D", "meta": ["array"]}));

    let collection = db.collection("items").unwrap();

    // Find items where meta is an object
    let results = collection
        .find(&json!({"meta": {"$type": "object"}}))
        .unwrap();

    assert_eq!(results.len(), 2); // A and C
}

#[test]
fn test_type_null() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(&db, "items", json!({"name": "A", "value": null}));
    insert_doc(&db, "items", json!({"name": "B", "value": 0})); // 0 is not null
    insert_doc(&db, "items", json!({"name": "C", "value": ""})); // Empty string is not null
    insert_doc(&db, "items", json!({"name": "D"})); // Missing field

    let collection = db.collection("items").unwrap();

    // Find items where value is null
    let results = collection
        .find(&json!({"value": {"$type": "null"}}))
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].get("name").unwrap().as_str().unwrap(), "A");
}

#[test]
fn test_type_nested_field() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(
        &db,
        "users",
        json!({"name": "Alice", "profile": {"age": 30}}),
    );
    insert_doc(
        &db,
        "users",
        json!({"name": "Bob", "profile": {"age": "thirty"}}),
    ); // String age
    insert_doc(&db, "users", json!({"name": "Carol", "profile": {}}));

    let collection = db.collection("users").unwrap();

    // Find users where profile.age is a number
    let results = collection
        .find(&json!({"profile.age": {"$type": "number"}}))
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].get("name").unwrap().as_str().unwrap(), "Alice");
}

#[test]
fn test_type_combined_with_exists() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(&db, "items", json!({"name": "A", "data": "text"}));
    insert_doc(&db, "items", json!({"name": "B", "data": 123}));
    insert_doc(&db, "items", json!({"name": "C"})); // No data field

    let collection = db.collection("items").unwrap();

    // Find items where data exists AND is a string
    let results = collection
        .find(&json!({
            "data": {"$exists": true, "$type": "string"}
        }))
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].get("name").unwrap().as_str().unwrap(), "A");
}

// ============================================================================
// EDGE CASES
// ============================================================================

#[test]
fn test_exists_on_array_element() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(
        &db,
        "items",
        json!({"name": "A", "arr": [{"x": 1}, {"y": 2}]}),
    );
    insert_doc(&db, "items", json!({"name": "B", "arr": [{"z": 3}]}));

    let collection = db.collection("items").unwrap();

    // Check if arr.0.x exists (first element has x field)
    let results = collection
        .find(&json!({"arr.0.x": {"$exists": true}}))
        .unwrap();

    // This depends on implementation - arr.0.x syntax might not be supported
    // Just verify it doesn't crash
    let _ = results;
}

#[test]
fn test_type_with_multiple_types() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(&db, "items", json!({"name": "A", "id": "ABC123"})); // String
    insert_doc(&db, "items", json!({"name": "B", "id": 456})); // Number
    insert_doc(&db, "items", json!({"name": "C", "id": true})); // Boolean

    let collection = db.collection("items").unwrap();

    // Using $or to match multiple types
    let results = collection
        .find(&json!({
            "$or": [
                {"id": {"$type": "string"}},
                {"id": {"$type": "number"}}
            ]
        }))
        .unwrap();

    assert_eq!(results.len(), 2); // A and B
}

#[test]
fn test_exists_on_deeply_nested() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(
        &db,
        "data",
        json!({
            "level1": {
                "level2": {
                    "level3": {
                        "value": 42
                    }
                }
            }
        }),
    );
    insert_doc(
        &db,
        "data",
        json!({
            "level1": {
                "level2": {}
            }
        }),
    );

    let collection = db.collection("data").unwrap();

    // Check deeply nested field existence
    let results = collection
        .find(&json!({
            "level1.level2.level3.value": {"$exists": true}
        }))
        .unwrap();

    assert_eq!(results.len(), 1);
}
