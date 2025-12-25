// index_persistence_tests.rs
// Integration tests for index persistence across database reopens

use ironbase_core::DatabaseCore;
use serde_json::json;
use std::collections::HashMap;
use tempfile::TempDir;

/// Helper to create a user document
fn user_doc(id: i64, name: &str, age: i64, city: &str) -> HashMap<String, serde_json::Value> {
    let mut doc = HashMap::new();
    doc.insert("_id".to_string(), json!(id));
    doc.insert("name".to_string(), json!(name));
    doc.insert("age".to_string(), json!(age));
    doc.insert("city".to_string(), json!(city));
    doc
}

// ============================================================================
// SINGLE INDEX PERSISTENCE
// ============================================================================

#[test]
fn test_index_persists_after_reopen() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // Create index and insert data
    {
        let db = DatabaseCore::open(&db_path).unwrap();

        // Insert data
        for i in 1..=100 {
            db.insert_one(
                "users",
                user_doc(i, &format!("User{}", i), 20 + (i % 50), "City"),
            )
            .unwrap();
        }

        // Create index on age
        let coll = db.collection("users").unwrap();
        coll.create_index("age".to_string(), false).unwrap();

        db.close().unwrap();
    }

    // Verify index works after reopen
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let coll = db.collection("users").unwrap();

        // Query using indexed field should work
        let results = coll.find(&json!({"age": 25})).unwrap();
        assert!(!results.is_empty());

        // Range query should also work
        let range = coll.find(&json!({"age": {"$gte": 30, "$lt": 35}})).unwrap();
        assert!(!range.is_empty());

        db.close().unwrap();
    }
}

#[test]
fn test_index_on_string_field_persists() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // Create index on string field
    {
        let db = DatabaseCore::open(&db_path).unwrap();

        for i in 1..=50 {
            let city = match i % 5 {
                0 => "NYC",
                1 => "LA",
                2 => "Chicago",
                3 => "Houston",
                _ => "Phoenix",
            };
            db.insert_one("users", user_doc(i, &format!("User{}", i), 25, city))
                .unwrap();
        }

        let coll = db.collection("users").unwrap();
        coll.create_index("city".to_string(), false).unwrap();

        db.close().unwrap();
    }

    // Verify string index works after reopen
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let coll = db.collection("users").unwrap();

        let nyc_users = coll.find(&json!({"city": "NYC"})).unwrap();
        assert!(!nyc_users.is_empty(), "NYC query should return results");
        assert!(
            nyc_users.iter().all(|u| u["city"] == "NYC"),
            "All results should have city=NYC"
        );

        let la_users = coll.find(&json!({"city": "LA"})).unwrap();
        assert!(!la_users.is_empty(), "LA query should return results");
        assert!(
            la_users.iter().all(|u| u["city"] == "LA"),
            "All results should have city=LA"
        );

        db.close().unwrap();
    }
}

// ============================================================================
// MULTIPLE INDEXES PERSISTENCE
// ============================================================================

#[test]
fn test_multiple_indexes_persist() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // Create multiple indexes
    {
        let db = DatabaseCore::open(&db_path).unwrap();

        for i in 1..=100 {
            let mut doc = HashMap::new();
            doc.insert("_id".to_string(), json!(i));
            doc.insert("name".to_string(), json!(format!("Product{}", i)));
            doc.insert("price".to_string(), json!((i as f64) * 10.0));
            doc.insert(
                "category".to_string(),
                json!(if i % 3 == 0 {
                    "Electronics"
                } else if i % 3 == 1 {
                    "Clothing"
                } else {
                    "Food"
                }),
            );
            doc.insert("stock".to_string(), json!(i * 10));
            db.insert_one("products", doc).unwrap();
        }

        let coll = db.collection("products").unwrap();
        coll.create_index("price".to_string(), false).unwrap();
        coll.create_index("category".to_string(), false).unwrap();
        coll.create_index("stock".to_string(), false).unwrap();

        db.close().unwrap();
    }

    // Verify all indexes work after reopen
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let coll = db.collection("products").unwrap();

        // Query on price index
        let cheap = coll.find(&json!({"price": {"$lt": 200.0}})).unwrap();
        assert!(!cheap.is_empty());

        // Query on category index
        let electronics = coll.find(&json!({"category": "Electronics"})).unwrap();
        assert!(!electronics.is_empty());

        // Query on stock index
        let low_stock = coll.find(&json!({"stock": {"$lte": 100}})).unwrap();
        assert!(!low_stock.is_empty());

        db.close().unwrap();
    }
}

// ============================================================================
// INDEX WITH DATA CHANGES
// ============================================================================

#[test]
fn test_index_updated_after_insert() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // Create index, close, reopen, insert, verify
    {
        let db = DatabaseCore::open(&db_path).unwrap();

        for i in 1..=10 {
            db.insert_one("users", user_doc(i, &format!("User{}", i), 25, "City"))
                .unwrap();
        }

        let coll = db.collection("users").unwrap();
        coll.create_index("age".to_string(), false).unwrap();
        db.close().unwrap();
    }

    // Insert more data after reopen
    {
        let db = DatabaseCore::open(&db_path).unwrap();

        for i in 11..=20 {
            db.insert_one("users", user_doc(i, &format!("User{}", i), 30, "City"))
                .unwrap();
        }

        db.close().unwrap();
    }

    // Verify index includes new data
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let coll = db.collection("users").unwrap();

        let age_25 = coll.find(&json!({"age": 25})).unwrap();
        assert!(!age_25.is_empty(), "age=25 query should return results");
        assert!(
            age_25.iter().all(|u| u["age"] == 25),
            "All age_25 results should have age=25"
        );

        let age_30 = coll.find(&json!({"age": 30})).unwrap();
        assert!(!age_30.is_empty(), "age=30 query should return results");
        assert!(
            age_30.iter().all(|u| u["age"] == 30),
            "All age_30 results should have age=30"
        );

        db.close().unwrap();
    }
}

#[test]
fn test_index_updated_after_update() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // Create index with data
    {
        let db = DatabaseCore::open(&db_path).unwrap();

        for i in 1..=10 {
            db.insert_one("users", user_doc(i, &format!("User{}", i), 25, "OldCity"))
                .unwrap();
        }

        let coll = db.collection("users").unwrap();
        coll.create_index("city".to_string(), false).unwrap();
        db.close().unwrap();
    }

    // Update indexed field after reopen
    {
        let db = DatabaseCore::open(&db_path).unwrap();

        for i in 1..=5 {
            db.update_one(
                "users",
                &json!({"_id": i}),
                &json!({"$set": {"city": "NewCity"}}),
            )
            .unwrap();
        }

        db.close().unwrap();
    }

    // Verify index reflects updates
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let coll = db.collection("users").unwrap();

        let old_city = coll.find(&json!({"city": "OldCity"})).unwrap();
        assert!(!old_city.is_empty(), "OldCity query should return results");
        assert!(
            old_city.iter().all(|u| u["city"] == "OldCity"),
            "All old_city results should have city=OldCity"
        );

        let new_city = coll.find(&json!({"city": "NewCity"})).unwrap();
        assert!(!new_city.is_empty(), "NewCity query should return results");
        assert!(
            new_city.iter().all(|u| u["city"] == "NewCity"),
            "All new_city results should have city=NewCity"
        );

        db.close().unwrap();
    }
}

#[test]
fn test_index_updated_after_delete() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // Create index with data
    {
        let db = DatabaseCore::open(&db_path).unwrap();

        for i in 1..=20 {
            db.insert_one(
                "users",
                user_doc(
                    i,
                    &format!("User{}", i),
                    if i <= 10 { 25 } else { 30 },
                    "City",
                ),
            )
            .unwrap();
        }

        let coll = db.collection("users").unwrap();
        coll.create_index("age".to_string(), false).unwrap();
        db.close().unwrap();
    }

    // Delete some documents after reopen
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        db.delete_many("users", &json!({"age": 25})).unwrap();
        db.close().unwrap();
    }

    // Verify index reflects deletes
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let coll = db.collection("users").unwrap();

        // Deleted documents should not appear
        let age_25 = coll.find(&json!({"age": 25})).unwrap();
        assert!(age_25.is_empty(), "age=25 documents should be deleted");

        // Remaining documents should still be queryable
        let age_30 = coll.find(&json!({"age": 30})).unwrap();
        assert!(!age_30.is_empty(), "age=30 query should return results");
        assert!(
            age_30.iter().all(|u| u["age"] == 30),
            "All age_30 results should have age=30"
        );

        db.close().unwrap();
    }
}

// ============================================================================
// INDEX ON NESTED FIELDS
// ============================================================================

#[test]
fn test_index_on_nested_field_persists() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // Create index on nested field
    {
        let db = DatabaseCore::open(&db_path).unwrap();

        for i in 1..=50 {
            let mut doc = HashMap::new();
            doc.insert("_id".to_string(), json!(i));
            doc.insert(
                "address".to_string(),
                json!({
                    "city": if i % 2 == 0 { "NYC" } else { "LA" },
                    "zip": format!("1000{}", i % 100)
                }),
            );
            db.insert_one("users", doc).unwrap();
        }

        let coll = db.collection("users").unwrap();
        coll.create_index("address.city".to_string(), false)
            .unwrap();
        db.close().unwrap();
    }

    // Verify nested index works after reopen
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let coll = db.collection("users").unwrap();

        let nyc = coll.find(&json!({"address.city": "NYC"})).unwrap();
        assert!(!nyc.is_empty(), "NYC nested query should return results");
        assert!(
            nyc.iter().all(|u| u["address"]["city"] == "NYC"),
            "All NYC results should have address.city=NYC"
        );

        let la = coll.find(&json!({"address.city": "LA"})).unwrap();
        assert!(!la.is_empty(), "LA nested query should return results");
        assert!(
            la.iter().all(|u| u["address"]["city"] == "LA"),
            "All LA results should have address.city=LA"
        );

        db.close().unwrap();
    }
}

// ============================================================================
// INDEX ACROSS MULTIPLE COLLECTIONS
// ============================================================================

#[test]
fn test_indexes_on_multiple_collections_persist() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // Create indexes on multiple collections
    {
        let db = DatabaseCore::open(&db_path).unwrap();

        // Users collection
        for i in 1..=30 {
            db.insert_one("users", user_doc(i, &format!("User{}", i), 20 + i, "City"))
                .unwrap();
        }
        let users = db.collection("users").unwrap();
        users.create_index("age".to_string(), false).unwrap();

        // Products collection
        for i in 1..=30 {
            let mut doc = HashMap::new();
            doc.insert("_id".to_string(), json!(i));
            doc.insert("name".to_string(), json!(format!("Prod{}", i)));
            doc.insert("price".to_string(), json!(i as f64 * 10.0));
            db.insert_one("products", doc).unwrap();
        }
        let products = db.collection("products").unwrap();
        products.create_index("price".to_string(), false).unwrap();

        // Orders collection
        for i in 1..=30 {
            let mut doc = HashMap::new();
            doc.insert("_id".to_string(), json!(i));
            doc.insert("total".to_string(), json!(i * 100));
            db.insert_one("orders", doc).unwrap();
        }
        let orders = db.collection("orders").unwrap();
        orders.create_index("total".to_string(), false).unwrap();

        db.close().unwrap();
    }

    // Verify all indexes work after reopen
    {
        let db = DatabaseCore::open(&db_path).unwrap();

        // Check users index
        let users = db.collection("users").unwrap();
        let young = users.find(&json!({"age": {"$lt": 30}})).unwrap();
        assert!(!young.is_empty());

        // Check products index
        let products = db.collection("products").unwrap();
        let cheap = products.find(&json!({"price": {"$lt": 100.0}})).unwrap();
        assert!(!cheap.is_empty());

        // Check orders index
        let orders = db.collection("orders").unwrap();
        let small_orders = orders.find(&json!({"total": {"$lte": 500}})).unwrap();
        assert!(!small_orders.is_empty());

        db.close().unwrap();
    }
}

// ============================================================================
// LIST INDEXES
// ============================================================================

#[test]
fn test_list_indexes_after_reopen() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // Create indexes
    {
        let db = DatabaseCore::open(&db_path).unwrap();

        for i in 1..=10 {
            db.insert_one("test", user_doc(i, &format!("User{}", i), 25, "City"))
                .unwrap();
        }

        let coll = db.collection("test").unwrap();
        coll.create_index("age".to_string(), false).unwrap();
        coll.create_index("city".to_string(), false).unwrap();

        db.close().unwrap();
    }

    // Verify list_indexes after reopen
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let coll = db.collection("test").unwrap();

        let indexes = coll.list_indexes().unwrap();

        // Should have both indexes
        assert!(indexes.iter().any(|idx| idx.contains("age")));
        assert!(indexes.iter().any(|idx| idx.contains("city")));

        db.close().unwrap();
    }
}

// ============================================================================
// INDEX MULTIPLE REOPEN CYCLES
// ============================================================================

#[test]
fn test_index_multiple_reopen_cycles() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // Cycle 1: Create index
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        for i in 1..=10 {
            db.insert_one("data", user_doc(i, &format!("User{}", i), i, "City"))
                .unwrap();
        }
        let coll = db.collection("data").unwrap();
        coll.create_index("age".to_string(), false).unwrap();
        db.close().unwrap();
    }

    // Cycle 2: Add data
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        for i in 11..=20 {
            db.insert_one("data", user_doc(i, &format!("User{}", i), i, "City"))
                .unwrap();
        }
        db.close().unwrap();
    }

    // Cycle 3: Update data
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        db.update_many(
            "data",
            &json!({"age": {"$lte": 5}}),
            &json!({"$set": {"age": 100}}),
        )
        .unwrap();
        db.close().unwrap();
    }

    // Cycle 4: Delete data
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        db.delete_many("data", &json!({"age": 100})).unwrap();
        db.close().unwrap();
    }

    // Cycle 5: Verify final state
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let coll = db.collection("data").unwrap();

        // Should have 15 documents (20 - 5 deleted)
        assert_eq!(coll.count_documents(&json!({})).unwrap(), 15);

        // Query should use index and return results
        let results = coll.find(&json!({"age": 10})).unwrap();
        assert!(!results.is_empty(), "age=10 query should return results");
        assert!(
            results.iter().all(|r| r["age"] == 10),
            "All results should have age=10"
        );

        db.close().unwrap();
    }
}

// ============================================================================
// INDEX PERFORMANCE AFTER REOPEN
// ============================================================================

#[test]
fn test_index_query_performance_after_reopen() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // Create large dataset with index
    {
        let db = DatabaseCore::open(&db_path).unwrap();

        for i in 1..=1000 {
            db.insert_one("large", user_doc(i, &format!("User{}", i), i % 100, "City"))
                .unwrap();
        }

        let coll = db.collection("large").unwrap();
        coll.create_index("age".to_string(), false).unwrap();

        db.close().unwrap();
    }

    // Verify index query is efficient after reopen
    {
        let db = DatabaseCore::open(&db_path).unwrap();
        let coll = db.collection("large").unwrap();

        // These queries should be efficient due to index
        let age_50 = coll.find(&json!({"age": 50})).unwrap();
        assert!(!age_50.is_empty(), "age=50 query should return results");
        assert!(
            age_50.iter().all(|r| r["age"] == 50),
            "All results should have age=50"
        );

        let range = coll.find(&json!({"age": {"$gte": 90, "$lt": 95}})).unwrap();
        assert!(!range.is_empty(), "Range query should return results");
        assert!(
            range.iter().all(|r| {
                let age = r["age"].as_i64().unwrap();
                age >= 90 && age < 95
            }),
            "All range results should have 90 <= age < 95"
        );

        db.close().unwrap();
    }
}
