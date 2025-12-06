// Logical operator integration tests
// Tests: $and, $or, $nor, $not

use ironbase_core::StorageEngine;
use serde_json::json;
use std::collections::HashMap;
use tempfile::TempDir;

type DatabaseCore = ironbase_core::DatabaseCore<StorageEngine>;

// ============================================================================
// HELPER MACRO
// ============================================================================

fn insert_doc(db: &DatabaseCore, coll: &str, doc: serde_json::Value) {
    if let serde_json::Value::Object(map) = doc {
        let fields: HashMap<String, serde_json::Value> = map.into_iter().collect();
        db.insert_one(coll, fields).unwrap();
    }
}

// ============================================================================
// $AND OPERATOR TESTS
// ============================================================================

#[test]
fn test_and_operator_basic() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(
        &db,
        "users",
        json!({"name": "Alice", "age": 30, "city": "NYC"}),
    );
    insert_doc(
        &db,
        "users",
        json!({"name": "Bob", "age": 25, "city": "LA"}),
    );
    insert_doc(
        &db,
        "users",
        json!({"name": "Carol", "age": 30, "city": "LA"}),
    );

    let collection = db.collection("users").unwrap();

    // $and: age = 30 AND city = "LA"
    let results = collection
        .find(&json!({
            "$and": [
                {"age": 30},
                {"city": "LA"}
            ]
        }))
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].get("name").unwrap().as_str().unwrap(), "Carol");
}

#[test]
fn test_and_operator_multiple_conditions() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(
        &db,
        "products",
        json!({"name": "Laptop", "price": 1000, "stock": 5, "category": "electronics"}),
    );
    insert_doc(
        &db,
        "products",
        json!({"name": "Phone", "price": 500, "stock": 10, "category": "electronics"}),
    );
    insert_doc(
        &db,
        "products",
        json!({"name": "Book", "price": 20, "stock": 100, "category": "books"}),
    );

    let collection = db.collection("products").unwrap();

    // $and with 3 conditions
    let results = collection
        .find(&json!({
            "$and": [
                {"category": "electronics"},
                {"price": {"$gte": 500}},
                {"stock": {"$lte": 10}}
            ]
        }))
        .unwrap();

    assert_eq!(results.len(), 2); // Laptop and Phone
}

#[test]
fn test_and_operator_nested() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(
        &db,
        "users",
        json!({"name": "Alice", "profile": {"age": 30, "verified": true}}),
    );
    insert_doc(
        &db,
        "users",
        json!({"name": "Bob", "profile": {"age": 25, "verified": false}}),
    );
    insert_doc(
        &db,
        "users",
        json!({"name": "Carol", "profile": {"age": 30, "verified": false}}),
    );

    let collection = db.collection("users").unwrap();

    // $and with nested field access
    let results = collection
        .find(&json!({
            "$and": [
                {"profile.age": 30},
                {"profile.verified": true}
            ]
        }))
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].get("name").unwrap().as_str().unwrap(), "Alice");
}

#[test]
fn test_and_operator_implicit() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(
        &db,
        "users",
        json!({"name": "Alice", "age": 30, "city": "NYC"}),
    );
    insert_doc(
        &db,
        "users",
        json!({"name": "Bob", "age": 25, "city": "LA"}),
    );

    let collection = db.collection("users").unwrap();

    // Implicit $and (multiple conditions in object)
    let results = collection
        .find(&json!({
            "age": 30,
            "city": "NYC"
        }))
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].get("name").unwrap().as_str().unwrap(), "Alice");
}

#[test]
fn test_and_operator_no_match() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(&db, "users", json!({"name": "Alice", "age": 30}));
    insert_doc(&db, "users", json!({"name": "Bob", "age": 25}));

    let collection = db.collection("users").unwrap();

    // $and with impossible condition
    let results = collection
        .find(&json!({
            "$and": [
                {"age": 30},
                {"age": 25}
            ]
        }))
        .unwrap();

    assert_eq!(results.len(), 0);
}

// ============================================================================
// $OR OPERATOR TESTS
// ============================================================================

#[test]
fn test_or_operator_basic() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(&db, "users", json!({"name": "Alice", "city": "NYC"}));
    insert_doc(&db, "users", json!({"name": "Bob", "city": "LA"}));
    insert_doc(&db, "users", json!({"name": "Carol", "city": "Chicago"}));

    let collection = db.collection("users").unwrap();

    // $or: city = "NYC" OR city = "LA"
    let results = collection
        .find(&json!({
            "$or": [
                {"city": "NYC"},
                {"city": "LA"}
            ]
        }))
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
fn test_or_operator_multiple_conditions() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(
        &db,
        "products",
        json!({"name": "A", "price": 10, "category": "food"}),
    );
    insert_doc(
        &db,
        "products",
        json!({"name": "B", "price": 100, "category": "electronics"}),
    );
    insert_doc(
        &db,
        "products",
        json!({"name": "C", "price": 50, "category": "books"}),
    );
    insert_doc(
        &db,
        "products",
        json!({"name": "D", "price": 200, "category": "food"}),
    );

    let collection = db.collection("products").unwrap();

    // $or with 3 conditions
    let results = collection
        .find(&json!({
            "$or": [
                {"price": {"$lt": 20}},
                {"category": "electronics"},
                {"price": {"$gt": 150}}
            ]
        }))
        .unwrap();

    assert_eq!(results.len(), 3); // A (price < 20), B (electronics), D (price > 150)
}

#[test]
fn test_or_operator_with_comparison() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(&db, "users", json!({"name": "Young", "age": 18}));
    insert_doc(&db, "users", json!({"name": "Adult", "age": 35}));
    insert_doc(&db, "users", json!({"name": "Senior", "age": 65}));

    let collection = db.collection("users").unwrap();

    // $or: age < 20 OR age >= 60
    let results = collection
        .find(&json!({
            "$or": [
                {"age": {"$lt": 20}},
                {"age": {"$gte": 60}}
            ]
        }))
        .unwrap();

    assert_eq!(results.len(), 2);
    let names: Vec<&str> = results
        .iter()
        .map(|d| d.get("name").unwrap().as_str().unwrap())
        .collect();
    assert!(names.contains(&"Young"));
    assert!(names.contains(&"Senior"));
}

#[test]
fn test_or_operator_all_match() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(&db, "users", json!({"name": "Alice", "status": "active"}));
    insert_doc(&db, "users", json!({"name": "Bob", "status": "active"}));

    let collection = db.collection("users").unwrap();

    // All documents match at least one condition
    let results = collection
        .find(&json!({
            "$or": [
                {"status": "active"},
                {"status": "inactive"}
            ]
        }))
        .unwrap();

    assert_eq!(results.len(), 2);
}

// ============================================================================
// $NOR OPERATOR TESTS
// ============================================================================

#[test]
fn test_nor_operator_basic() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(
        &db,
        "users",
        json!({"name": "Alice", "deleted": false, "banned": false}),
    );
    insert_doc(
        &db,
        "users",
        json!({"name": "Bob", "deleted": true, "banned": false}),
    );
    insert_doc(
        &db,
        "users",
        json!({"name": "Carol", "deleted": false, "banned": true}),
    );
    insert_doc(
        &db,
        "users",
        json!({"name": "Dave", "deleted": true, "banned": true}),
    );

    let collection = db.collection("users").unwrap();

    // $nor: NOT (deleted = true) AND NOT (banned = true)
    // Should only return Alice
    let results = collection
        .find(&json!({
            "$nor": [
                {"deleted": true},
                {"banned": true}
            ]
        }))
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].get("name").unwrap().as_str().unwrap(), "Alice");
}

#[test]
fn test_nor_operator_with_comparison() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(
        &db,
        "products",
        json!({"name": "A", "price": 50, "stock": 100}),
    );
    insert_doc(
        &db,
        "products",
        json!({"name": "B", "price": 10, "stock": 50}),
    ); // price < 20
    insert_doc(
        &db,
        "products",
        json!({"name": "C", "price": 200, "stock": 30}),
    ); // price > 150
    insert_doc(
        &db,
        "products",
        json!({"name": "D", "price": 100, "stock": 5}),
    ); // stock < 10

    let collection = db.collection("products").unwrap();

    // $nor: NOT (price < 20) AND NOT (price > 150) AND NOT (stock < 10)
    // Should only return A (price=50, stock=100)
    let results = collection
        .find(&json!({
            "$nor": [
                {"price": {"$lt": 20}},
                {"price": {"$gt": 150}},
                {"stock": {"$lt": 10}}
            ]
        }))
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].get("name").unwrap().as_str().unwrap(), "A");
}

#[test]
fn test_nor_operator_all_excluded() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(&db, "users", json!({"name": "Alice", "status": "active"}));
    insert_doc(&db, "users", json!({"name": "Bob", "status": "inactive"}));

    let collection = db.collection("users").unwrap();

    // $nor with conditions that exclude all
    let results = collection
        .find(&json!({
            "$nor": [
                {"status": "active"},
                {"status": "inactive"}
            ]
        }))
        .unwrap();

    assert_eq!(results.len(), 0);
}

// ============================================================================
// $NOT OPERATOR TESTS
// ============================================================================

#[test]
fn test_not_operator_basic() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(&db, "users", json!({"name": "Alice", "age": 30}));
    insert_doc(&db, "users", json!({"name": "Bob", "age": 25}));
    insert_doc(&db, "users", json!({"name": "Carol", "age": 35}));

    let collection = db.collection("users").unwrap();

    // $not: age NOT > 30 (i.e., age <= 30)
    let results = collection
        .find(&json!({
            "age": {"$not": {"$gt": 30}}
        }))
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
fn test_not_operator_with_regex() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(
        &db,
        "users",
        json!({"name": "Alice", "email": "alice@gmail.com"}),
    );
    insert_doc(
        &db,
        "users",
        json!({"name": "Bob", "email": "bob@yahoo.com"}),
    );
    insert_doc(
        &db,
        "users",
        json!({"name": "Carol", "email": "carol@gmail.com"}),
    );

    let collection = db.collection("users").unwrap();

    // $not with regex: email NOT matching gmail
    let results = collection
        .find(&json!({
            "email": {"$not": {"$regex": "gmail"}}
        }))
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].get("name").unwrap().as_str().unwrap(), "Bob");
}

#[test]
fn test_not_operator_with_eq() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(&db, "users", json!({"name": "Alice", "status": "active"}));
    insert_doc(&db, "users", json!({"name": "Bob", "status": "inactive"}));
    insert_doc(&db, "users", json!({"name": "Carol", "status": "pending"}));

    let collection = db.collection("users").unwrap();

    // $not: status NOT = "active"
    let results = collection
        .find(&json!({
            "status": {"$not": {"$eq": "active"}}
        }))
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
fn test_not_operator_with_range() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(&db, "products", json!({"name": "Cheap", "price": 10}));
    insert_doc(&db, "products", json!({"name": "Medium", "price": 50}));
    insert_doc(&db, "products", json!({"name": "Expensive", "price": 100}));

    let collection = db.collection("products").unwrap();

    // $not with range: price NOT between 20 and 80
    let results = collection
        .find(&json!({
            "price": {"$not": {"$gte": 20, "$lte": 80}}
        }))
        .unwrap();

    // Should return Cheap (10) and Expensive (100)
    assert_eq!(results.len(), 2);
}

// ============================================================================
// COMBINED LOGICAL OPERATORS
// ============================================================================

#[test]
fn test_and_or_combined() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(
        &db,
        "users",
        json!({"name": "Alice", "age": 30, "city": "NYC", "premium": true}),
    );
    insert_doc(
        &db,
        "users",
        json!({"name": "Bob", "age": 25, "city": "LA", "premium": false}),
    );
    insert_doc(
        &db,
        "users",
        json!({"name": "Carol", "age": 35, "city": "NYC", "premium": false}),
    );
    insert_doc(
        &db,
        "users",
        json!({"name": "Dave", "age": 40, "city": "LA", "premium": true}),
    );

    let collection = db.collection("users").unwrap();

    // (age >= 30) AND (city = "NYC" OR premium = true)
    let results = collection
        .find(&json!({
            "$and": [
                {"age": {"$gte": 30}},
                {"$or": [
                    {"city": "NYC"},
                    {"premium": true}
                ]}
            ]
        }))
        .unwrap();

    assert_eq!(results.len(), 3); // Alice (NYC+premium), Carol (NYC), Dave (premium)
}

#[test]
fn test_or_and_combined() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(
        &db,
        "products",
        json!({"name": "A", "category": "electronics", "price": 500, "stock": 10}),
    );
    insert_doc(
        &db,
        "products",
        json!({"name": "B", "category": "electronics", "price": 100, "stock": 5}),
    );
    insert_doc(
        &db,
        "products",
        json!({"name": "C", "category": "books", "price": 20, "stock": 100}),
    );
    insert_doc(
        &db,
        "products",
        json!({"name": "D", "category": "books", "price": 50, "stock": 0}),
    );

    let collection = db.collection("products").unwrap();

    // (category = "electronics" AND price > 200) OR (category = "books" AND stock > 50)
    let results = collection
        .find(&json!({
            "$or": [
                {"$and": [{"category": "electronics"}, {"price": {"$gt": 200}}]},
                {"$and": [{"category": "books"}, {"stock": {"$gt": 50}}]}
            ]
        }))
        .unwrap();

    assert_eq!(results.len(), 2); // A (electronics, price=500) and C (books, stock=100)
}

#[test]
fn test_not_with_or() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(&db, "users", json!({"name": "Alice", "role": "admin"}));
    insert_doc(&db, "users", json!({"name": "Bob", "role": "moderator"}));
    insert_doc(&db, "users", json!({"name": "Carol", "role": "user"}));

    let collection = db.collection("users").unwrap();

    // Find users who are neither admin nor moderator
    let results = collection
        .find(&json!({
            "role": {"$not": {"$in": ["admin", "moderator"]}}
        }))
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].get("name").unwrap().as_str().unwrap(), "Carol");
}

#[test]
fn test_nested_logical_operators() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(
        &db,
        "orders",
        json!({"id": 1, "status": "pending", "total": 100, "priority": "high"}),
    );
    insert_doc(
        &db,
        "orders",
        json!({"id": 2, "status": "shipped", "total": 50, "priority": "low"}),
    );
    insert_doc(
        &db,
        "orders",
        json!({"id": 3, "status": "pending", "total": 200, "priority": "low"}),
    );
    insert_doc(
        &db,
        "orders",
        json!({"id": 4, "status": "delivered", "total": 150, "priority": "high"}),
    );

    let collection = db.collection("orders").unwrap();

    // Complex nested: (status = "pending" AND (total > 150 OR priority = "high"))
    let results = collection
        .find(&json!({
            "$and": [
                {"status": "pending"},
                {"$or": [
                    {"total": {"$gt": 150}},
                    {"priority": "high"}
                ]}
            ]
        }))
        .unwrap();

    assert_eq!(results.len(), 2); // Order 1 (pending+high) and Order 3 (pending+total>150)
}

// ============================================================================
// EDGE CASES
// ============================================================================

#[test]
fn test_empty_and() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(&db, "users", json!({"name": "Alice"}));

    let collection = db.collection("users").unwrap();

    // Empty $and should match all documents
    let results = collection.find(&json!({"$and": []})).unwrap();
    assert_eq!(results.len(), 1);
}

#[test]
fn test_empty_or() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(&db, "users", json!({"name": "Alice"}));

    let collection = db.collection("users").unwrap();

    // Empty $or should match no documents
    let results = collection.find(&json!({"$or": []})).unwrap();
    assert_eq!(results.len(), 0);
}

#[test]
fn test_single_condition_and() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(&db, "users", json!({"name": "Alice", "age": 30}));
    insert_doc(&db, "users", json!({"name": "Bob", "age": 25}));

    let collection = db.collection("users").unwrap();

    // Single condition in $and
    let results = collection
        .find(&json!({
            "$and": [{"age": 30}]
        }))
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].get("name").unwrap().as_str().unwrap(), "Alice");
}
