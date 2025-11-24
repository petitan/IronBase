// Tests for explain() and hint() functionality
use ironbase_core::DatabaseCore;
use serde_json::json;
use tempfile::TempDir;

#[test]
fn test_explain_with_index() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    let db = DatabaseCore::open(&db_path).unwrap();
    let collection = db.collection("users").unwrap();

    // Create index on age
    collection.create_index("age".to_string(), false).unwrap();

    // Explain equality query - should use index
    let plan = collection.explain(&json!({"age": 25})).unwrap();

    assert_eq!(plan.get("queryPlan").unwrap(), "IndexScan");
    assert_eq!(plan.get("indexUsed").unwrap(), "users_age");
    assert_eq!(plan.get("stage").unwrap(), "FETCH_WITH_INDEX");
    assert_eq!(plan.get("indexType").unwrap(), "equality");
}

#[test]
fn test_explain_range_query() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    let db = DatabaseCore::open(&db_path).unwrap();
    let collection = db.collection("products").unwrap();

    // Create index on price
    collection.create_index("price".to_string(), false).unwrap();

    // Explain range query - should use IndexRangeScan
    let plan = collection
        .explain(&json!({
            "price": {
                "$gte": 100,
                "$lt": 500
            }
        }))
        .unwrap();

    assert_eq!(plan.get("queryPlan").unwrap(), "IndexRangeScan");
    assert_eq!(plan.get("indexUsed").unwrap(), "products_price");
    assert_eq!(plan.get("stage").unwrap(), "FETCH_WITH_INDEX");
    assert_eq!(plan.get("indexType").unwrap(), "range");

    // Verify range details
    let range = plan.get("range").unwrap();
    assert_eq!(range.get("inclusiveStart").unwrap(), true);
    assert_eq!(range.get("inclusiveEnd").unwrap(), false);
}

#[test]
fn test_explain_without_index() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    let db = DatabaseCore::open(&db_path).unwrap();
    let collection = db.collection("users").unwrap();

    // No index on name field
    // Explain query - should use CollectionScan
    let plan = collection.explain(&json!({"name": "Alice"})).unwrap();

    assert_eq!(plan.get("queryPlan").unwrap(), "CollectionScan");
    assert_eq!(plan.get("indexUsed").unwrap(), &json!(null));
    assert_eq!(plan.get("stage").unwrap(), "FULL_SCAN");
    assert_eq!(plan.get("estimatedCost").unwrap(), "O(n)");
}

#[test]
fn test_hint_forces_index_usage() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    let db = DatabaseCore::open(&db_path).unwrap();
    let collection = db.collection("users").unwrap();

    // Create index on age
    collection.create_index("age".to_string(), false).unwrap();

    // Insert test data
    for i in 0..50 {
        let mut fields = std::collections::HashMap::new();
        fields.insert("name".to_string(), json!(format!("User{}", i)));
        fields.insert("age".to_string(), json!(i % 10)); // Ages 0-9, 5 docs each
        collection.insert_one(fields).unwrap();
    }

    // Query with hint
    let results = collection
        .find_with_hint(&json!({"age": 5}), "users_age")
        .unwrap();

    assert_eq!(results.len(), 5); // Should find all 5 docs with age=5
}

#[test]
fn test_hint_with_range_query() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    let db = DatabaseCore::open(&db_path).unwrap();
    let collection = db.collection("products").unwrap();

    // Create index on price
    collection.create_index("price".to_string(), false).unwrap();

    // Insert test products
    for i in 0..20 {
        let mut fields = std::collections::HashMap::new();
        fields.insert("name".to_string(), json!(format!("Product{}", i)));
        fields.insert("price".to_string(), json!(i * 10));
        collection.insert_one(fields).unwrap();
    }

    // Range query with hint
    let results = collection
        .find_with_hint(
            &json!({
                "price": {
                    "$gte": 50,
                    "$lt": 150
                }
            }),
            "products_price",
        )
        .unwrap();

    assert_eq!(results.len(), 10); // Prices 50-140 (10 products)
}

#[test]
fn test_hint_invalid_index() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    let db = DatabaseCore::open(&db_path).unwrap();
    let collection = db.collection("users").unwrap();

    // Try to use non-existent index
    let result = collection.find_with_hint(&json!({"age": 25}), "nonexistent_index");

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not found"));
}

#[test]
fn test_hint_wrong_field() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    let db = DatabaseCore::open(&db_path).unwrap();
    let collection = db.collection("users").unwrap();

    // Create index on age
    collection.create_index("age".to_string(), false).unwrap();

    // Try to use age index for name query
    let result = collection.find_with_hint(&json!({"name": "Alice"}), "users_age");

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Cannot use index"));
}

#[test]
fn test_explain_and_hint_consistency() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    let db = DatabaseCore::open(&db_path).unwrap();
    let collection = db.collection("users").unwrap();

    // Create index
    collection.create_index("age".to_string(), false).unwrap();

    // Insert data
    for i in 0..10 {
        let mut fields = std::collections::HashMap::new();
        fields.insert("age".to_string(), json!(i));
        collection.insert_one(fields).unwrap();
    }

    let query = json!({"age": 5});

    // 1. Explain should show it will use index
    let plan = collection.explain(&query).unwrap();
    assert_eq!(plan.get("queryPlan").unwrap(), "IndexScan");
    assert_eq!(plan.get("indexUsed").unwrap(), "users_age");

    // 2. Normal find (auto index selection)
    let results_auto = collection.find(&query).unwrap();

    // 3. Hinted find (manual index selection)
    let results_hint = collection.find_with_hint(&query, "users_age").unwrap();

    // Both should return same results
    assert_eq!(results_auto.len(), results_hint.len());
    assert_eq!(results_auto.len(), 1);
}
