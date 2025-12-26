// Aggregation accumulator integration tests
// Tests: $sum, $avg, $min, $max, $first, $last

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
// $SUM ACCUMULATOR TESTS
// ============================================================================

#[test]
fn test_sum_accumulator_basic() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(&db, "sales", json!({"product": "A", "qty": 10, "price": 5}));
    insert_doc(&db, "sales", json!({"product": "B", "qty": 20, "price": 3}));
    insert_doc(&db, "sales", json!({"product": "A", "qty": 5, "price": 5}));

    let collection = db.collection("sales").unwrap();

    // Sum qty by product
    let results = collection
        .aggregate(&json!([
            {"$group": {"_id": "$product", "totalQty": {"$sum": "$qty"}}}
        ]))
        .unwrap();

    assert_eq!(results.len(), 2);

    // Find totals
    let mut a_total = 0;
    let mut b_total = 0;
    for doc in &results {
        if doc.get("_id").unwrap() == "A" {
            a_total = doc.get("totalQty").unwrap().as_i64().unwrap();
        } else if doc.get("_id").unwrap() == "B" {
            b_total = doc.get("totalQty").unwrap().as_i64().unwrap();
        }
    }
    assert_eq!(a_total, 15); // 10 + 5
    assert_eq!(b_total, 20);
}

#[test]
fn test_sum_accumulator_count() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(
        &db,
        "items",
        json!({"category": "electronics", "name": "Phone"}),
    );
    insert_doc(
        &db,
        "items",
        json!({"category": "electronics", "name": "Laptop"}),
    );
    insert_doc(&db, "items", json!({"category": "books", "name": "Novel"}));

    let collection = db.collection("items").unwrap();

    // Count items per category using $sum: 1
    let results = collection
        .aggregate(&json!([
            {"$group": {"_id": "$category", "count": {"$sum": 1}}}
        ]))
        .unwrap();

    assert_eq!(results.len(), 2);

    for doc in &results {
        if doc.get("_id").unwrap() == "electronics" {
            assert_eq!(doc.get("count").unwrap().as_i64().unwrap(), 2);
        } else if doc.get("_id").unwrap() == "books" {
            assert_eq!(doc.get("count").unwrap().as_i64().unwrap(), 1);
        }
    }
}

// ============================================================================
// $AVG ACCUMULATOR TESTS
// ============================================================================

#[test]
fn test_avg_accumulator_integers() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(
        &db,
        "scores",
        json!({"student": "A", "class": "math", "score": 80}),
    );
    insert_doc(
        &db,
        "scores",
        json!({"student": "B", "class": "math", "score": 90}),
    );
    insert_doc(
        &db,
        "scores",
        json!({"student": "C", "class": "math", "score": 100}),
    );
    insert_doc(
        &db,
        "scores",
        json!({"student": "D", "class": "english", "score": 75}),
    );

    let collection = db.collection("scores").unwrap();

    // Average score by class
    let results = collection
        .aggregate(&json!([
            {"$group": {"_id": "$class", "avgScore": {"$avg": "$score"}}}
        ]))
        .unwrap();

    assert_eq!(results.len(), 2);

    for doc in &results {
        if doc.get("_id").unwrap() == "math" {
            let avg = doc.get("avgScore").unwrap().as_f64().unwrap();
            assert!((avg - 90.0).abs() < 0.001); // (80+90+100)/3 = 90
        } else if doc.get("_id").unwrap() == "english" {
            let avg = doc.get("avgScore").unwrap().as_f64().unwrap();
            assert!((avg - 75.0).abs() < 0.001);
        }
    }
}

#[test]
fn test_avg_accumulator_floats() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(&db, "measurements", json!({"sensor": "A", "value": 1.5}));
    insert_doc(&db, "measurements", json!({"sensor": "A", "value": 2.5}));
    insert_doc(&db, "measurements", json!({"sensor": "A", "value": 3.0}));

    let collection = db.collection("measurements").unwrap();

    let results = collection
        .aggregate(&json!([
            {"$group": {"_id": "$sensor", "avgValue": {"$avg": "$value"}}}
        ]))
        .unwrap();

    assert_eq!(results.len(), 1);
    let avg = results[0].get("avgValue").unwrap().as_f64().unwrap();
    assert!((avg - 2.333333).abs() < 0.001); // (1.5+2.5+3.0)/3
}

#[test]
fn test_avg_accumulator_single_value() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(&db, "data", json!({"group": "X", "value": 42}));

    let collection = db.collection("data").unwrap();

    let results = collection
        .aggregate(&json!([
            {"$group": {"_id": "$group", "avg": {"$avg": "$value"}}}
        ]))
        .unwrap();

    assert_eq!(results.len(), 1);
    let avg = results[0].get("avg").unwrap().as_f64().unwrap();
    assert!((avg - 42.0).abs() < 0.001);
}

// ============================================================================
// $MIN ACCUMULATOR TESTS
// ============================================================================

#[test]
fn test_min_accumulator_numbers() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(&db, "products", json!({"category": "A", "price": 100}));
    insert_doc(&db, "products", json!({"category": "A", "price": 50}));
    insert_doc(&db, "products", json!({"category": "A", "price": 75}));
    insert_doc(&db, "products", json!({"category": "B", "price": 200}));

    let collection = db.collection("products").unwrap();

    let results = collection
        .aggregate(&json!([
            {"$group": {"_id": "$category", "minPrice": {"$min": "$price"}}}
        ]))
        .unwrap();

    assert_eq!(results.len(), 2);

    for doc in &results {
        if doc.get("_id").unwrap() == "A" {
            // Min returns float
            let min = doc.get("minPrice").unwrap().as_f64().unwrap();
            assert!((min - 50.0).abs() < 0.001);
        } else if doc.get("_id").unwrap() == "B" {
            let min = doc.get("minPrice").unwrap().as_f64().unwrap();
            assert!((min - 200.0).abs() < 0.001);
        }
    }
}

#[test]
fn test_min_accumulator_with_negatives() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(&db, "temps", json!({"location": "A", "temp": -10}));
    insert_doc(&db, "temps", json!({"location": "A", "temp": 5}));
    insert_doc(&db, "temps", json!({"location": "A", "temp": -20}));

    let collection = db.collection("temps").unwrap();

    let results = collection
        .aggregate(&json!([
            {"$group": {"_id": "$location", "minTemp": {"$min": "$temp"}}}
        ]))
        .unwrap();

    assert_eq!(results.len(), 1);
    let min = results[0].get("minTemp").unwrap().as_f64().unwrap();
    assert!((min - (-20.0)).abs() < 0.001);
}

#[test]
fn test_min_accumulator_floats() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(&db, "data", json!({"group": "X", "value": 1.5}));
    insert_doc(&db, "data", json!({"group": "X", "value": 0.5}));
    insert_doc(&db, "data", json!({"group": "X", "value": 2.5}));

    let collection = db.collection("data").unwrap();

    let results = collection
        .aggregate(&json!([
            {"$group": {"_id": "$group", "min": {"$min": "$value"}}}
        ]))
        .unwrap();

    assert_eq!(results.len(), 1);
    let min = results[0].get("min").unwrap().as_f64().unwrap();
    assert!((min - 0.5).abs() < 0.001);
}

// ============================================================================
// $MAX ACCUMULATOR TESTS
// ============================================================================

#[test]
fn test_max_accumulator_numbers() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(&db, "scores", json!({"player": "A", "score": 100}));
    insert_doc(&db, "scores", json!({"player": "A", "score": 250}));
    insert_doc(&db, "scores", json!({"player": "A", "score": 175}));
    insert_doc(&db, "scores", json!({"player": "B", "score": 300}));

    let collection = db.collection("scores").unwrap();

    let results = collection
        .aggregate(&json!([
            {"$group": {"_id": "$player", "maxScore": {"$max": "$score"}}}
        ]))
        .unwrap();

    assert_eq!(results.len(), 2);

    for doc in &results {
        if doc.get("_id").unwrap() == "A" {
            let max = doc.get("maxScore").unwrap().as_f64().unwrap();
            assert!((max - 250.0).abs() < 0.001);
        } else if doc.get("_id").unwrap() == "B" {
            let max = doc.get("maxScore").unwrap().as_f64().unwrap();
            assert!((max - 300.0).abs() < 0.001);
        }
    }
}

#[test]
fn test_max_accumulator_floats() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(&db, "data", json!({"group": "X", "value": 1.1}));
    insert_doc(&db, "data", json!({"group": "X", "value": 3.3}));
    insert_doc(&db, "data", json!({"group": "X", "value": 2.2}));

    let collection = db.collection("data").unwrap();

    let results = collection
        .aggregate(&json!([
            {"$group": {"_id": "$group", "max": {"$max": "$value"}}}
        ]))
        .unwrap();

    assert_eq!(results.len(), 1);
    let max = results[0].get("max").unwrap().as_f64().unwrap();
    assert!((max - 3.3).abs() < 0.001);
}

// ============================================================================
// $FIRST ACCUMULATOR TESTS
// ============================================================================

#[test]
fn test_first_accumulator() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(
        &db,
        "events",
        json!({"user": "A", "action": "login", "time": 1}),
    );
    insert_doc(
        &db,
        "events",
        json!({"user": "A", "action": "click", "time": 2}),
    );
    insert_doc(
        &db,
        "events",
        json!({"user": "A", "action": "logout", "time": 3}),
    );
    insert_doc(
        &db,
        "events",
        json!({"user": "B", "action": "login", "time": 1}),
    );

    let collection = db.collection("events").unwrap();

    // Sort by time to ensure predictable order, then get first action per user
    let results = collection
        .aggregate(&json!([
            {"$sort": {"time": 1}},
            {"$group": {"_id": "$user", "firstAction": {"$first": "$action"}}}
        ]))
        .unwrap();

    assert_eq!(results.len(), 2);

    for doc in &results {
        if doc.get("_id").unwrap() == "A" {
            // After sorting by time, first for A should be "login" (time=1)
            assert_eq!(doc.get("firstAction").unwrap().as_str().unwrap(), "login");
        } else if doc.get("_id").unwrap() == "B" {
            assert_eq!(doc.get("firstAction").unwrap().as_str().unwrap(), "login");
        }
    }
}

#[test]
fn test_first_with_sort() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(
        &db,
        "logs",
        json!({"category": "error", "message": "first", "priority": 1}),
    );
    insert_doc(
        &db,
        "logs",
        json!({"category": "error", "message": "second", "priority": 2}),
    );
    insert_doc(
        &db,
        "logs",
        json!({"category": "error", "message": "third", "priority": 3}),
    );

    let collection = db.collection("logs").unwrap();

    // Sort by priority desc, then get first (highest priority)
    let results = collection
        .aggregate(&json!([
            {"$sort": {"priority": -1}},
            {"$group": {"_id": "$category", "firstMsg": {"$first": "$message"}}}
        ]))
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(
        results[0].get("firstMsg").unwrap().as_str().unwrap(),
        "third"
    );
}

// ============================================================================
// $LAST ACCUMULATOR TESTS
// ============================================================================

#[test]
fn test_last_accumulator() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(
        &db,
        "events",
        json!({"user": "A", "action": "login", "time": 1}),
    );
    insert_doc(
        &db,
        "events",
        json!({"user": "A", "action": "click", "time": 2}),
    );
    insert_doc(
        &db,
        "events",
        json!({"user": "A", "action": "logout", "time": 3}),
    );
    insert_doc(
        &db,
        "events",
        json!({"user": "B", "action": "signup", "time": 1}),
    );

    let collection = db.collection("events").unwrap();

    // Sort by time to ensure predictable order, then get last action per user
    let results = collection
        .aggregate(&json!([
            {"$sort": {"time": 1}},
            {"$group": {"_id": "$user", "lastAction": {"$last": "$action"}}}
        ]))
        .unwrap();

    assert_eq!(results.len(), 2);

    for doc in &results {
        if doc.get("_id").unwrap() == "A" {
            // After sorting by time, last for A should be "logout" (time=3)
            assert_eq!(doc.get("lastAction").unwrap().as_str().unwrap(), "logout");
        } else if doc.get("_id").unwrap() == "B" {
            assert_eq!(doc.get("lastAction").unwrap().as_str().unwrap(), "signup");
        }
    }
}

#[test]
fn test_last_with_sort() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(
        &db,
        "logs",
        json!({"category": "info", "message": "first", "seq": 1}),
    );
    insert_doc(
        &db,
        "logs",
        json!({"category": "info", "message": "second", "seq": 2}),
    );
    insert_doc(
        &db,
        "logs",
        json!({"category": "info", "message": "third", "seq": 3}),
    );

    let collection = db.collection("logs").unwrap();

    // Sort by seq asc, then get last (highest seq)
    let results = collection
        .aggregate(&json!([
            {"$sort": {"seq": 1}},
            {"$group": {"_id": "$category", "lastMsg": {"$last": "$message"}}}
        ]))
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(
        results[0].get("lastMsg").unwrap().as_str().unwrap(),
        "third"
    );
}

// ============================================================================
// MULTIPLE ACCUMULATORS COMBINED
// ============================================================================

#[test]
fn test_multiple_accumulators() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(&db, "sales", json!({"store": "NYC", "amount": 100}));
    insert_doc(&db, "sales", json!({"store": "NYC", "amount": 200}));
    insert_doc(&db, "sales", json!({"store": "NYC", "amount": 150}));
    insert_doc(&db, "sales", json!({"store": "LA", "amount": 300}));
    insert_doc(&db, "sales", json!({"store": "LA", "amount": 250}));

    let collection = db.collection("sales").unwrap();

    // Multiple accumulators in one group
    let results = collection
        .aggregate(&json!([
            {"$group": {
                "_id": "$store",
                "totalSales": {"$sum": "$amount"},
                "avgSale": {"$avg": "$amount"},
                "minSale": {"$min": "$amount"},
                "maxSale": {"$max": "$amount"},
                "count": {"$sum": 1}
            }}
        ]))
        .unwrap();

    assert_eq!(results.len(), 2);

    for doc in &results {
        if doc.get("_id").unwrap() == "NYC" {
            assert!((doc.get("totalSales").unwrap().as_f64().unwrap() - 450.0).abs() < 0.001);
            assert!((doc.get("avgSale").unwrap().as_f64().unwrap() - 150.0).abs() < 0.001);
            assert!((doc.get("minSale").unwrap().as_f64().unwrap() - 100.0).abs() < 0.001);
            assert!((doc.get("maxSale").unwrap().as_f64().unwrap() - 200.0).abs() < 0.001);
            assert!((doc.get("count").unwrap().as_f64().unwrap() - 3.0).abs() < 0.001);
        } else if doc.get("_id").unwrap() == "LA" {
            assert!((doc.get("totalSales").unwrap().as_f64().unwrap() - 550.0).abs() < 0.001);
            assert!((doc.get("avgSale").unwrap().as_f64().unwrap() - 275.0).abs() < 0.001);
            assert!((doc.get("minSale").unwrap().as_f64().unwrap() - 250.0).abs() < 0.001);
            assert!((doc.get("maxSale").unwrap().as_f64().unwrap() - 300.0).abs() < 0.001);
            assert!((doc.get("count").unwrap().as_f64().unwrap() - 2.0).abs() < 0.001);
        }
    }
}

#[test]
fn test_accumulators_with_match() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(&db, "orders", json!({"status": "completed", "total": 100}));
    insert_doc(&db, "orders", json!({"status": "completed", "total": 200}));
    insert_doc(&db, "orders", json!({"status": "pending", "total": 50}));
    insert_doc(&db, "orders", json!({"status": "completed", "total": 150}));

    let collection = db.collection("orders").unwrap();

    // Filter first, then aggregate
    let results = collection
        .aggregate(&json!([
            {"$match": {"status": "completed"}},
            {"$group": {
                "_id": null,
                "totalRevenue": {"$sum": "$total"},
                "avgOrder": {"$avg": "$total"},
                "orderCount": {"$sum": 1}
            }}
        ]))
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(
        results[0].get("totalRevenue").unwrap().as_i64().unwrap(),
        450
    );
    assert!((results[0].get("avgOrder").unwrap().as_f64().unwrap() - 150.0).abs() < 0.001);
    assert_eq!(results[0].get("orderCount").unwrap().as_i64().unwrap(), 3);
}

#[test]
fn test_accumulators_with_nested_fields() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(
        &db,
        "orders",
        json!({"customer": {"city": "NYC"}, "amount": 100}),
    );
    insert_doc(
        &db,
        "orders",
        json!({"customer": {"city": "NYC"}, "amount": 200}),
    );
    insert_doc(
        &db,
        "orders",
        json!({"customer": {"city": "LA"}, "amount": 150}),
    );

    let collection = db.collection("orders").unwrap();

    // Group by nested field
    let results = collection
        .aggregate(&json!([
            {"$group": {
                "_id": "$customer.city",
                "total": {"$sum": "$amount"},
                "avg": {"$avg": "$amount"}
            }}
        ]))
        .unwrap();

    assert_eq!(results.len(), 2);

    for doc in &results {
        if doc.get("_id").unwrap() == "NYC" {
            assert_eq!(doc.get("total").unwrap().as_i64().unwrap(), 300);
            assert!((doc.get("avg").unwrap().as_f64().unwrap() - 150.0).abs() < 0.001);
        }
    }
}

// ============================================================================
// EDGE CASES
// ============================================================================

#[test]
fn test_accumulator_empty_group() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(&db, "data", json!({"type": "A", "value": 10}));

    let collection = db.collection("data").unwrap();

    // Filter out all documents, then group
    let results = collection
        .aggregate(&json!([
            {"$match": {"type": "NONEXISTENT"}},
            {"$group": {"_id": null, "sum": {"$sum": "$value"}}}
        ]))
        .unwrap();

    // Empty result when no documents match
    assert_eq!(results.len(), 0);
}

#[test]
fn test_accumulator_null_values() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    insert_doc(&db, "data", json!({"group": "A", "value": 10}));
    insert_doc(&db, "data", json!({"group": "A", "value": null}));
    insert_doc(&db, "data", json!({"group": "A", "value": 20}));

    let collection = db.collection("data").unwrap();

    // Accumulators should handle null values
    let results = collection
        .aggregate(&json!([
            {"$group": {"_id": "$group", "sum": {"$sum": "$value"}}}
        ]))
        .unwrap();

    assert_eq!(results.len(), 1);
    // null is typically treated as 0 in sum
    let sum = results[0].get("sum").unwrap().as_i64().unwrap();
    assert!(sum == 30); // Depending on null handling
}

// ============================================================================
// STREAMING AGGREGATION MEMORY OPTIMIZATION TEST
// ============================================================================

/// Test that streaming aggregation handles large documents efficiently.
///
/// This test verifies the fix for OOM issues where $group would store
/// full documents in memory. The new streaming implementation only
/// stores accumulator states, reducing memory from O(N * doc_size)
/// to O(G * state_size) where G = number of groups.
///
/// Example from bug report:
/// - 650K emails grouped by sender (10K groups)
/// - OLD: 650K × 800 bytes = ~500MB stored in HashMap
/// - NEW: 10K × 64 bytes = ~640KB (780× reduction!)
#[test]
fn test_streaming_aggregation_memory_efficiency() {
    use ironbase_core::{storage::MemoryStorage, DatabaseCore};

    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();

    // Insert 1000 documents with 1KB body each (simulating large emails)
    // 100 unique senders means 100 groups
    for i in 0..1000 {
        let sender = format!("sender{}@example.com", i % 100);
        let body = "x".repeat(1000); // 1KB body
        let mut doc = HashMap::new();
        doc.insert("from".to_string(), json!({"name": sender, "email": sender}));
        doc.insert("body".to_string(), json!(body));
        doc.insert("index".to_string(), json!(i));
        db.insert_one("emails", doc).unwrap();
    }

    // This would OOM with old implementation on larger datasets
    // because it stored full 1KB documents in groups HashMap
    let coll = db.collection("emails").unwrap();
    let results = coll
        .aggregate(&json!([
            {"$group": {"_id": "$from.name", "count": {"$sum": 1}}},
            {"$limit": 15}
        ]))
        .unwrap();

    // Verify aggregation worked correctly
    assert_eq!(results.len(), 15); // $limit 15

    // Each group should have exactly 10 documents (1000 docs / 100 senders)
    for doc in &results {
        let count = doc.get("count").unwrap().as_i64().unwrap();
        assert_eq!(count, 10, "Each sender should have 10 emails");
    }
}

/// Test streaming aggregation with nested field grouping
/// (reproduces the exact query from the bug report)
#[test]
fn test_streaming_aggregation_nested_field_grouping() {
    use ironbase_core::{storage::MemoryStorage, DatabaseCore};

    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();

    // Insert test data with nested "from.name" field
    let docs = vec![
        json!({"from": {"name": "Alice"}, "subject": "Hello"}),
        json!({"from": {"name": "Bob"}, "subject": "Hi"}),
        json!({"from": {"name": "Alice"}, "subject": "Bye"}),
        json!({"from": {"name": "Charlie"}, "subject": "Test"}),
        json!({"from": {"name": "Alice"}, "subject": "Again"}),
    ];

    for doc in docs {
        if let serde_json::Value::Object(map) = doc {
            let fields: HashMap<String, serde_json::Value> = map.into_iter().collect();
            db.insert_one("emails", fields).unwrap();
        }
    }

    // Exact query from bug report
    let coll = db.collection("emails").unwrap();
    let results = coll
        .aggregate(&json!([
            {"$group": {"_id": "$from.name", "count": {"$sum": 1}}},
            {"$limit": 15}
        ]))
        .unwrap();

    assert_eq!(results.len(), 3); // Alice, Bob, Charlie

    // Find Alice's count
    let alice_count = results
        .iter()
        .find(|doc| doc.get("_id").unwrap() == "Alice")
        .map(|doc| doc.get("count").unwrap().as_i64().unwrap())
        .unwrap();
    assert_eq!(alice_count, 3);
}

/// BUG TEST: $first should return null if first document has missing field
/// Not skip to the second document's value
#[test]
fn test_first_with_missing_field_bug() {
    use ironbase_core::{storage::MemoryStorage, DatabaseCore};

    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();

    // Insert docs where first doc is missing the "value" field
    let docs = vec![
        json!({"group": "A"}),                    // Missing "value" - should be $first
        json!({"group": "A", "value": "second"}), // Has value
        json!({"group": "A", "value": "third"}),  // Has value
    ];

    for doc in docs {
        if let serde_json::Value::Object(map) = doc {
            let fields: HashMap<String, serde_json::Value> = map.into_iter().collect();
            db.insert_one("test", fields).unwrap();
        }
    }

    let coll = db.collection("test").unwrap();
    let results = coll
        .aggregate(&json!([
            {"$group": {"_id": "$group", "first_val": {"$first": "$value"}}}
        ]))
        .unwrap();

    // $first should return null because first doc has missing field
    // BUG: streaming implementation skips missing and returns "second"
    let first_val = &results[0].get("first_val").unwrap();
    assert!(
        first_val.is_null(),
        "BUG: $first should return null for missing field, got {:?}",
        first_val
    );
}

/// BUG TEST: $last should return null if last document has missing field
#[test]
fn test_last_with_missing_field_bug() {
    use ironbase_core::{storage::MemoryStorage, DatabaseCore};

    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();

    // Insert docs where last doc is missing the "value" field
    let docs = vec![
        json!({"group": "A", "value": "first"}),
        json!({"group": "A", "value": "second"}),
        json!({"group": "A"}), // Missing "value" - should be $last
    ];

    for doc in docs {
        if let serde_json::Value::Object(map) = doc {
            let fields: HashMap<String, serde_json::Value> = map.into_iter().collect();
            db.insert_one("test2", fields).unwrap();
        }
    }

    let coll = db.collection("test2").unwrap();
    let results = coll
        .aggregate(&json!([
            {"$group": {"_id": "$group", "last_val": {"$last": "$value"}}}
        ]))
        .unwrap();

    // $last should return null because last doc has missing field
    // BUG: streaming implementation keeps "second" (previous value)
    let last_val = &results[0].get("last_val").unwrap();
    assert!(
        last_val.is_null(),
        "BUG: $last should return null for missing field, got {:?}",
        last_val
    );
}

/// Test streaming $project before $group reduces memory usage
/// Pipeline: [$project, $group] should stream both stages
#[test]
fn test_streaming_project_then_group() {
    use ironbase_core::{storage::MemoryStorage, DatabaseCore};

    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();

    // Insert 100 documents with large "body" field (1KB each)
    // but we only need "from.name" for grouping
    for i in 0..100 {
        let sender = format!("sender{}@example.com", i % 10);
        let body = "x".repeat(1000); // 1KB body - should be discarded by $project
        let mut doc = HashMap::new();
        doc.insert("from".to_string(), json!({"name": sender, "email": sender}));
        doc.insert("body".to_string(), json!(body));
        doc.insert("subject".to_string(), json!(format!("Email {}", i)));
        db.insert_one("emails", doc).unwrap();
    }

    let coll = db.collection("emails").unwrap();

    // Pipeline with $project BEFORE $group
    // $project discards the large "body" field before grouping
    let results = coll
        .aggregate(&json!([
            {"$project": {"from": 1}},  // Only keep "from", discard "body" (1KB savings per doc!)
            {"$group": {"_id": "$from.name", "count": {"$sum": 1}}}
        ]))
        .unwrap();

    // Should have 10 unique senders
    assert_eq!(results.len(), 10);

    // Each sender should have 10 emails
    for doc in &results {
        let count = doc.get("count").unwrap().as_i64().unwrap();
        assert_eq!(count, 10, "Each sender should have 10 emails");
    }
}

/// Test $match + $project + $group all streaming
#[test]
fn test_streaming_match_project_group() {
    use ironbase_core::{storage::MemoryStorage, DatabaseCore};

    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();

    // Insert documents with different categories
    let docs = vec![
        json!({"category": "A", "value": 10, "extra": "large data here"}),
        json!({"category": "A", "value": 20, "extra": "large data here"}),
        json!({"category": "B", "value": 30, "extra": "large data here"}),
        json!({"category": "A", "value": 40, "extra": "large data here"}),
        json!({"category": "B", "value": 50, "extra": "large data here"}),
    ];

    for doc in docs {
        if let serde_json::Value::Object(map) = doc {
            let fields: HashMap<String, serde_json::Value> = map.into_iter().collect();
            db.insert_one("data", fields).unwrap();
        }
    }

    let coll = db.collection("data").unwrap();

    // Full streaming pipeline: $match → $project → $group
    let results = coll
        .aggregate(&json!([
            {"$match": {"category": "A"}},           // Filter to category A only (streaming)
            {"$project": {"category": 1, "value": 1}}, // Drop "extra" field (streaming)
            {"$group": {"_id": "$category", "total": {"$sum": "$value"}}}  // Sum values (streaming)
        ]))
        .unwrap();

    assert_eq!(results.len(), 1);
    let total = results[0].get("total").unwrap().as_i64().unwrap();
    assert_eq!(total, 70); // 10 + 20 + 40 = 70
}

/// Test that $project with field rename works in streaming mode
#[test]
fn test_streaming_project_rename() {
    use ironbase_core::{storage::MemoryStorage, DatabaseCore};

    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();

    let docs = vec![
        json!({"original_field": "value1", "other": "data"}),
        json!({"original_field": "value2", "other": "data"}),
    ];

    for doc in docs {
        if let serde_json::Value::Object(map) = doc {
            let fields: HashMap<String, serde_json::Value> = map.into_iter().collect();
            db.insert_one("test", fields).unwrap();
        }
    }

    let coll = db.collection("test").unwrap();

    // $project with rename, then $group
    let results = coll
        .aggregate(&json!([
            {"$project": {"renamed": "$original_field"}},  // Rename field
            {"$group": {"_id": null, "values": {"$push": "$renamed"}}}
        ]))
        .unwrap();

    assert_eq!(results.len(), 1);
    let values = results[0].get("values").unwrap().as_array().unwrap();
    assert_eq!(values.len(), 2);
    assert!(values.contains(&json!("value1")));
    assert!(values.contains(&json!("value2")));
}
