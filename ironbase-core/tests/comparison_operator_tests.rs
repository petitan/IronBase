// Comparison operator integration tests
// Tests: $ne, $in, $nin (+ basic $eq, $gt, $gte, $lt, $lte for completeness)

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
// $NE (Not Equal) OPERATOR TESTS
// ============================================================================

#[test]
fn test_ne_operator_string() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(&db, "users", json!({"name": "Alice", "status": "active"}));
    insert_doc(&db, "users", json!({"name": "Bob", "status": "inactive"}));
    insert_doc(&db, "users", json!({"name": "Carol", "status": "pending"}));

    let collection = db.collection("users").unwrap();

    // Find all users where status != "active"
    let results = collection
        .find(&json!({"status": {"$ne": "active"}}))
        .unwrap();

    assert_eq!(results.len(), 2);
    let names: Vec<&str> = results
        .iter()
        .map(|d| d.get("name").unwrap().as_str().unwrap())
        .collect();
    assert!(names.contains(&"Bob"));
    assert!(names.contains(&"Carol"));
}

#[test]
fn test_ne_operator_number() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(&db, "products", json!({"name": "A", "price": 100}));
    insert_doc(&db, "products", json!({"name": "B", "price": 200}));
    insert_doc(&db, "products", json!({"name": "C", "price": 100}));

    let collection = db.collection("products").unwrap();

    // Find all products where price != 100
    let results = collection.find(&json!({"price": {"$ne": 100}})).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].get("name").unwrap().as_str().unwrap(), "B");
}

#[test]
fn test_ne_operator_boolean() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(&db, "users", json!({"name": "Alice", "verified": true}));
    insert_doc(&db, "users", json!({"name": "Bob", "verified": false}));
    insert_doc(&db, "users", json!({"name": "Carol", "verified": true}));

    let collection = db.collection("users").unwrap();

    // Find all users where verified != true
    let results = collection
        .find(&json!({"verified": {"$ne": true}}))
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].get("name").unwrap().as_str().unwrap(), "Bob");
}

#[test]
fn test_ne_operator_null() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(
        &db,
        "users",
        json!({"name": "Alice", "email": "alice@test.com"}),
    );
    insert_doc(&db, "users", json!({"name": "Bob", "email": null}));
    insert_doc(&db, "users", json!({"name": "Carol"})); // No email field

    let collection = db.collection("users").unwrap();

    // Find all users where email != null
    let results = collection.find(&json!({"email": {"$ne": null}})).unwrap();

    // Should return Alice (has email) - Bob has null, Carol has no field
    assert!(!results.is_empty());
}

#[test]
fn test_ne_operator_nested() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(
        &db,
        "users",
        json!({"name": "Alice", "address": {"city": "NYC"}}),
    );
    insert_doc(
        &db,
        "users",
        json!({"name": "Bob", "address": {"city": "LA"}}),
    );
    insert_doc(
        &db,
        "users",
        json!({"name": "Carol", "address": {"city": "NYC"}}),
    );

    let collection = db.collection("users").unwrap();

    // Find all users where address.city != "NYC"
    let results = collection
        .find(&json!({"address.city": {"$ne": "NYC"}}))
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].get("name").unwrap().as_str().unwrap(), "Bob");
}

// ============================================================================
// $IN OPERATOR TESTS
// ============================================================================

#[test]
fn test_in_operator_strings() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(&db, "users", json!({"name": "Alice", "city": "NYC"}));
    insert_doc(&db, "users", json!({"name": "Bob", "city": "LA"}));
    insert_doc(&db, "users", json!({"name": "Carol", "city": "Chicago"}));
    insert_doc(&db, "users", json!({"name": "Dave", "city": "Boston"}));

    let collection = db.collection("users").unwrap();

    // Find users in NYC or LA
    let results = collection
        .find(&json!({"city": {"$in": ["NYC", "LA"]}}))
        .unwrap();

    assert_eq!(results.len(), 2);
    let names: Vec<&str> = results
        .iter()
        .map(|d| d.get("name").unwrap().as_str().unwrap())
        .collect();
    assert!(names.contains(&"Alice"));
    assert!(names.contains(&"Bob"));
}

#[test]
fn test_in_operator_numbers() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(&db, "products", json!({"name": "A", "category_id": 1}));
    insert_doc(&db, "products", json!({"name": "B", "category_id": 2}));
    insert_doc(&db, "products", json!({"name": "C", "category_id": 3}));
    insert_doc(&db, "products", json!({"name": "D", "category_id": 1}));

    let collection = db.collection("products").unwrap();

    // Find products in categories 1 or 3
    let results = collection
        .find(&json!({"category_id": {"$in": [1, 3]}}))
        .unwrap();

    assert_eq!(results.len(), 3);
}

#[test]
fn test_in_operator_mixed_types() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(&db, "items", json!({"name": "A", "code": "ABC"}));
    insert_doc(&db, "items", json!({"name": "B", "code": 123}));
    insert_doc(&db, "items", json!({"name": "C", "code": "XYZ"}));

    let collection = db.collection("items").unwrap();

    // $in with mixed types
    let results = collection
        .find(&json!({"code": {"$in": ["ABC", 123]}}))
        .unwrap();

    assert_eq!(results.len(), 2);
}

#[test]
fn test_in_operator_empty_array() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(&db, "users", json!({"name": "Alice"}));

    let collection = db.collection("users").unwrap();

    // $in with empty array should match nothing
    let results = collection.find(&json!({"name": {"$in": []}})).unwrap();

    assert_eq!(results.len(), 0);
}

#[test]
fn test_in_operator_single_value() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(&db, "users", json!({"name": "Alice", "role": "admin"}));
    insert_doc(&db, "users", json!({"name": "Bob", "role": "user"}));

    let collection = db.collection("users").unwrap();

    // $in with single value (equivalent to $eq)
    let results = collection
        .find(&json!({"role": {"$in": ["admin"]}}))
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].get("name").unwrap().as_str().unwrap(), "Alice");
}

#[test]
fn test_in_operator_with_array_field() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(
        &db,
        "products",
        json!({"name": "A", "tags": ["electronics", "sale"]}),
    );
    insert_doc(
        &db,
        "products",
        json!({"name": "B", "tags": ["books", "new"]}),
    );
    insert_doc(
        &db,
        "products",
        json!({"name": "C", "tags": ["electronics", "new"]}),
    );

    let collection = db.collection("products").unwrap();

    // $in on array field - matches if any element is in the list
    let results = collection
        .find(&json!({"tags": {"$in": ["sale", "books"]}}))
        .unwrap();

    assert_eq!(results.len(), 2); // A (has "sale") and B (has "books")
}

#[test]
fn test_in_operator_nested_field() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(
        &db,
        "users",
        json!({"name": "Alice", "address": {"country": "USA"}}),
    );
    insert_doc(
        &db,
        "users",
        json!({"name": "Bob", "address": {"country": "UK"}}),
    );
    insert_doc(
        &db,
        "users",
        json!({"name": "Carol", "address": {"country": "Canada"}}),
    );

    let collection = db.collection("users").unwrap();

    // $in on nested field
    let results = collection
        .find(&json!({"address.country": {"$in": ["USA", "UK"]}}))
        .unwrap();

    assert_eq!(results.len(), 2);
}

// ============================================================================
// $NIN (Not In) OPERATOR TESTS
// ============================================================================

#[test]
fn test_nin_operator_strings() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(&db, "users", json!({"name": "Alice", "status": "active"}));
    insert_doc(&db, "users", json!({"name": "Bob", "status": "banned"}));
    insert_doc(
        &db,
        "users",
        json!({"name": "Carol", "status": "suspended"}),
    );
    insert_doc(&db, "users", json!({"name": "Dave", "status": "active"}));

    let collection = db.collection("users").unwrap();

    // Find users NOT banned or suspended
    let results = collection
        .find(&json!({"status": {"$nin": ["banned", "suspended"]}}))
        .unwrap();

    assert_eq!(results.len(), 2);
    let names: Vec<&str> = results
        .iter()
        .map(|d| d.get("name").unwrap().as_str().unwrap())
        .collect();
    assert!(names.contains(&"Alice"));
    assert!(names.contains(&"Dave"));
}

#[test]
fn test_nin_operator_numbers() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(&db, "products", json!({"name": "A", "category_id": 1}));
    insert_doc(&db, "products", json!({"name": "B", "category_id": 2}));
    insert_doc(&db, "products", json!({"name": "C", "category_id": 3}));
    insert_doc(&db, "products", json!({"name": "D", "category_id": 4}));

    let collection = db.collection("products").unwrap();

    // Exclude categories 1 and 2
    let results = collection
        .find(&json!({"category_id": {"$nin": [1, 2]}}))
        .unwrap();

    assert_eq!(results.len(), 2);
    let names: Vec<&str> = results
        .iter()
        .map(|d| d.get("name").unwrap().as_str().unwrap())
        .collect();
    assert!(names.contains(&"C"));
    assert!(names.contains(&"D"));
}

#[test]
fn test_nin_operator_empty_array() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(&db, "users", json!({"name": "Alice"}));
    insert_doc(&db, "users", json!({"name": "Bob"}));

    let collection = db.collection("users").unwrap();

    // $nin with empty array should match all
    let results = collection.find(&json!({"name": {"$nin": []}})).unwrap();

    assert_eq!(results.len(), 2);
}

#[test]
fn test_nin_operator_with_array_field() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(
        &db,
        "products",
        json!({"name": "A", "tags": ["electronics", "sale"]}),
    );
    insert_doc(&db, "products", json!({"name": "B", "tags": ["books"]}));
    insert_doc(
        &db,
        "products",
        json!({"name": "C", "tags": ["clothing", "new"]}),
    );

    let collection = db.collection("products").unwrap();

    // $nin on array field - excludes if ANY element is in the list
    let results = collection
        .find(&json!({"tags": {"$nin": ["electronics", "books"]}}))
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].get("name").unwrap().as_str().unwrap(), "C");
}

// ============================================================================
// COMPARISON OPERATORS WITH EDGE CASES
// ============================================================================

#[test]
fn test_comparison_with_null() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(&db, "items", json!({"name": "A", "value": 10}));
    insert_doc(&db, "items", json!({"name": "B", "value": null}));
    insert_doc(&db, "items", json!({"name": "C"})); // No value field

    let collection = db.collection("items").unwrap();

    // $gt should not match null or missing
    let results = collection.find(&json!({"value": {"$gt": 5}})).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].get("name").unwrap().as_str().unwrap(), "A");
}

#[test]
fn test_eq_with_null() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(&db, "items", json!({"name": "A", "value": 10}));
    insert_doc(&db, "items", json!({"name": "B", "value": null}));

    let collection = db.collection("items").unwrap();

    // $eq null should match documents with null value
    let results = collection.find(&json!({"value": {"$eq": null}})).unwrap();
    assert!(!results.is_empty());
}

#[test]
fn test_comparison_operators_combined() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(
        &db,
        "products",
        json!({"name": "A", "price": 50, "category": "electronics"}),
    );
    insert_doc(
        &db,
        "products",
        json!({"name": "B", "price": 150, "category": "electronics"}),
    );
    insert_doc(
        &db,
        "products",
        json!({"name": "C", "price": 75, "category": "books"}),
    );
    insert_doc(
        &db,
        "products",
        json!({"name": "D", "price": 200, "category": "clothing"}),
    );

    let collection = db.collection("products").unwrap();

    // Combine: price between 60-160 AND category not "clothing"
    let results = collection
        .find(&json!({
            "price": {"$gte": 60, "$lte": 160},
            "category": {"$ne": "clothing"}
        }))
        .unwrap();

    assert_eq!(results.len(), 2); // B (150, electronics) and C (75, books)
}

#[test]
fn test_ne_combined_with_in() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(
        &db,
        "users",
        json!({"name": "Alice", "role": "admin", "dept": "IT"}),
    );
    insert_doc(
        &db,
        "users",
        json!({"name": "Bob", "role": "user", "dept": "HR"}),
    );
    insert_doc(
        &db,
        "users",
        json!({"name": "Carol", "role": "manager", "dept": "IT"}),
    );
    insert_doc(
        &db,
        "users",
        json!({"name": "Dave", "role": "user", "dept": "IT"}),
    );

    let collection = db.collection("users").unwrap();

    // role in ["admin", "manager"] AND dept != "HR"
    let results = collection
        .find(&json!({
            "role": {"$in": ["admin", "manager"]},
            "dept": {"$ne": "HR"}
        }))
        .unwrap();

    assert_eq!(results.len(), 2); // Alice and Carol
}

#[test]
fn test_comparison_operators_on_strings() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(&db, "items", json!({"code": "A001"}));
    insert_doc(&db, "items", json!({"code": "B002"}));
    insert_doc(&db, "items", json!({"code": "C003"}));

    let collection = db.collection("items").unwrap();

    // String comparison (lexicographic)
    let results = collection.find(&json!({"code": {"$gte": "B000"}})).unwrap();

    assert_eq!(results.len(), 2); // B002 and C003
}
