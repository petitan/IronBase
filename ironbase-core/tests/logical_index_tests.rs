use std::collections::HashMap;

use ironbase_core::storage::MemoryStorage;
use ironbase_core::DatabaseCore;
use serde_json::json;

fn create_memory_db() -> (DatabaseCore<MemoryStorage>, String) {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    let coll_name = "test".to_string();
    let _ = db.collection(&coll_name).unwrap();
    (db, coll_name)
}

#[test]
fn test_and_with_exists_and_equality() {
    let (db, coll_name) = create_memory_db();
    let collection = db.collection(&coll_name).unwrap();

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
    db.insert_one(&coll_name, HashMap::from([("other".to_string(), json!(1))]))
        .unwrap();

    collection
        .create_index("status".to_string(), false, true)
        .unwrap();

    let query = json!({
        "$and": [
            {"status": {"$exists": true}},
            {"status": "active"}
        ]
    });

    let results = collection.find(&query).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["status"], "active");
}

#[test]
fn test_or_with_indexed_clauses() {
    let (db, coll_name) = create_memory_db();
    let collection = db.collection(&coll_name).unwrap();

    for status in ["active", "inactive", "pending"] {
        db.insert_one(
            &coll_name,
            HashMap::from([("status".to_string(), json!(status))]),
        )
        .unwrap();
    }

    collection
        .create_index("status".to_string(), false, false)
        .unwrap();

    let query = json!({
        "$or": [
            {"status": "active"},
            {"status": "pending"}
        ]
    });

    let results = collection.find(&query).unwrap();
    let statuses: Vec<_> = results
        .iter()
        .map(|doc| doc["status"].as_str().unwrap())
        .collect();
    assert_eq!(statuses.len(), 2);
    assert!(statuses.contains(&"active"));
    assert!(statuses.contains(&"pending"));
}

#[test]
fn test_nor_with_indexed_clause() {
    let (db, coll_name) = create_memory_db();
    let collection = db.collection(&coll_name).unwrap();

    for status in ["active", "inactive", "pending"] {
        db.insert_one(
            &coll_name,
            HashMap::from([("status".to_string(), json!(status))]),
        )
        .unwrap();
    }

    collection
        .create_index("status".to_string(), false, false)
        .unwrap();

    let query = json!({"$nor": [{"status": "inactive"}]});

    let results = collection.find(&query).unwrap();
    let statuses: Vec<_> = results
        .iter()
        .map(|doc| doc["status"].as_str().unwrap())
        .collect();
    assert_eq!(statuses.len(), 2);
    assert!(statuses.contains(&"active"));
    assert!(statuses.contains(&"pending"));
}

// --- Clause merge optimization integration tests ---

#[test]
fn test_and_clause_merge_same_field_range() {
    let (db, coll_name) = create_memory_db();
    let collection = db.collection(&coll_name).unwrap();

    for year in 2020..=2030 {
        for i in 0..5 {
            db.insert_one(
                &coll_name,
                HashMap::from([
                    ("year".to_string(), json!(year)),
                    ("idx".to_string(), json!(i)),
                ]),
            )
            .unwrap();
        }
    }

    collection
        .create_index("year".to_string(), false, false)
        .unwrap();

    let and_query = json!({
        "$and": [
            {"year": {"$gte": 2024}},
            {"year": {"$lte": 2025}}
        ]
    });
    let implicit_query = json!({"year": {"$gte": 2024, "$lte": 2025}});

    let and_results = collection.find(&and_query).unwrap();
    let implicit_results = collection.find(&implicit_query).unwrap();

    assert_eq!(and_results.len(), implicit_results.len());
    assert_eq!(and_results.len(), 10); // 2024 + 2025, 5 docs each
}

#[test]
fn test_and_clause_merge_count() {
    let (db, coll_name) = create_memory_db();
    let collection = db.collection(&coll_name).unwrap();

    for year in 2020..=2030 {
        for i in 0..3 {
            db.insert_one(
                &coll_name,
                HashMap::from([
                    ("year".to_string(), json!(year)),
                    ("i".to_string(), json!(i)),
                ]),
            )
            .unwrap();
        }
    }

    collection
        .create_index("year".to_string(), false, false)
        .unwrap();

    let and_query = json!({
        "$and": [
            {"year": {"$gte": 2024}},
            {"year": {"$lte": 2026}}
        ]
    });

    let count = collection.count_documents(&and_query).unwrap();
    assert_eq!(count, 9);

    let implicit_count = collection
        .count_documents(&json!({"year": {"$gte": 2024, "$lte": 2026}}))
        .unwrap();
    assert_eq!(count, implicit_count);
}

#[test]
fn test_or_clause_merge_to_in() {
    let (db, coll_name) = create_memory_db();
    let collection = db.collection(&coll_name).unwrap();

    for year in 2020..=2030 {
        db.insert_one(
            &coll_name,
            HashMap::from([("year".to_string(), json!(year))]),
        )
        .unwrap();
    }

    collection
        .create_index("year".to_string(), false, false)
        .unwrap();

    let or_query = json!({"$or": [{"year": 2024}, {"year": 2025}, {"year": 2026}]});
    let in_query = json!({"year": {"$in": [2024, 2025, 2026]}});

    let or_results = collection.find(&or_query).unwrap();
    let in_results = collection.find(&in_query).unwrap();

    assert_eq!(or_results.len(), in_results.len());
    assert_eq!(or_results.len(), 3);
}

#[test]
fn test_or_clause_merge_count() {
    let (db, coll_name) = create_memory_db();
    let collection = db.collection(&coll_name).unwrap();

    for year in 2020..=2030 {
        for _ in 0..2 {
            db.insert_one(
                &coll_name,
                HashMap::from([("year".to_string(), json!(year))]),
            )
            .unwrap();
        }
    }

    collection
        .create_index("year".to_string(), false, false)
        .unwrap();

    let or_query = json!({"$or": [{"year": 2024}, {"year": 2025}]});
    let count = collection.count_documents(&or_query).unwrap();
    assert_eq!(count, 4);

    let in_count = collection
        .count_documents(&json!({"year": {"$in": [2024, 2025]}}))
        .unwrap();
    assert_eq!(count, in_count);
}

#[test]
fn test_and_clause_merge_different_fields() {
    let (db, coll_name) = create_memory_db();
    let collection = db.collection(&coll_name).unwrap();

    db.insert_one(
        &coll_name,
        HashMap::from([
            ("year".to_string(), json!(2024)),
            ("status".to_string(), json!("active")),
        ]),
    )
    .unwrap();
    db.insert_one(
        &coll_name,
        HashMap::from([
            ("year".to_string(), json!(2024)),
            ("status".to_string(), json!("inactive")),
        ]),
    )
    .unwrap();
    db.insert_one(
        &coll_name,
        HashMap::from([
            ("year".to_string(), json!(2025)),
            ("status".to_string(), json!("active")),
        ]),
    )
    .unwrap();

    collection
        .create_index("year".to_string(), false, false)
        .unwrap();

    let and_query = json!({
        "$and": [
            {"year": {"$gte": 2024}},
            {"status": "active"}
        ]
    });

    let results = collection.find(&and_query).unwrap();
    assert_eq!(results.len(), 2);
}

#[test]
fn test_and_clause_merge_explain_shows_optimization() {
    let (db, coll_name) = create_memory_db();
    let collection = db.collection(&coll_name).unwrap();

    collection
        .create_index("year".to_string(), false, false)
        .unwrap();

    let and_query = json!({
        "$and": [
            {"year": {"$gte": 2024}},
            {"year": {"$lte": 2025}}
        ]
    });

    let explain = collection.explain(&and_query).unwrap();
    assert_eq!(
        explain.get("optimization").and_then(|v| v.as_str()),
        Some("clause_merge"),
        "explain should show clause_merge optimization: {:?}",
        explain
    );
    assert_eq!(
        explain.get("originalOperator").and_then(|v| v.as_str()),
        Some("$and")
    );
}

#[test]
fn test_and_conflicting_gte_tighter_bound() {
    let (db, coll_name) = create_memory_db();
    let collection = db.collection(&coll_name).unwrap();

    for year in 2020..=2030 {
        db.insert_one(
            &coll_name,
            HashMap::from([("year".to_string(), json!(year))]),
        )
        .unwrap();
    }

    collection
        .create_index("year".to_string(), false, false)
        .unwrap();

    let query = json!({
        "$and": [
            {"year": {"$gte": 2020}},
            {"year": {"$gte": 2028}},
            {"year": {"$lte": 2030}}
        ]
    });

    let results = collection.find(&query).unwrap();
    assert_eq!(results.len(), 3); // 2028, 2029, 2030
}

#[test]
fn test_and_clause_merge_gt_lt_exclusive() {
    // $gt/$lt (exclusive boundaries) should also merge
    let (db, coll_name) = create_memory_db();
    let collection = db.collection(&coll_name).unwrap();

    for year in 2020..=2030 {
        db.insert_one(
            &coll_name,
            HashMap::from([("year".to_string(), json!(year))]),
        )
        .unwrap();
    }

    collection
        .create_index("year".to_string(), false, false)
        .unwrap();

    // $gt: 2024 means > 2024 (exclusive), $lt: 2028 means < 2028 (exclusive)
    let query = json!({"$and": [{"year": {"$gt": 2024}}, {"year": {"$lt": 2028}}]});
    let results = collection.find(&query).unwrap();
    assert_eq!(results.len(), 3); // 2025, 2026, 2027

    // Same with implicit form
    let implicit = collection
        .find(&json!({"year": {"$gt": 2024, "$lt": 2028}}))
        .unwrap();
    assert_eq!(results.len(), implicit.len());
}

#[test]
fn test_and_clause_merge_string_range() {
    let (db, coll_name) = create_memory_db();
    let collection = db.collection(&coll_name).unwrap();

    for name in ["Alice", "Bob", "Charlie", "Dave", "Eve"] {
        db.insert_one(
            &coll_name,
            HashMap::from([("name".to_string(), json!(name))]),
        )
        .unwrap();
    }

    collection
        .create_index("name".to_string(), false, false)
        .unwrap();

    let query = json!({"$and": [{"name": {"$gte": "Bob"}}, {"name": {"$lte": "Dave"}}]});
    let results = collection.find(&query).unwrap();
    assert_eq!(results.len(), 3); // Bob, Charlie, Dave
}

#[test]
fn test_or_clause_merge_boolean() {
    let (db, coll_name) = create_memory_db();
    let collection = db.collection(&coll_name).unwrap();

    db.insert_one(
        &coll_name,
        HashMap::from([("active".to_string(), json!(true))]),
    )
    .unwrap();
    db.insert_one(
        &coll_name,
        HashMap::from([("active".to_string(), json!(false))]),
    )
    .unwrap();

    collection
        .create_index("active".to_string(), false, false)
        .unwrap();

    let query = json!({"$or": [{"active": true}, {"active": false}]});
    let results = collection.find(&query).unwrap();
    assert_eq!(results.len(), 2);
}

#[test]
fn test_and_regex_fallback_still_works() {
    // $and with $regex should NOT merge but still return correct results via fallback
    let (db, coll_name) = create_memory_db();
    let collection = db.collection(&coll_name).unwrap();

    db.insert_one(
        &coll_name,
        HashMap::from([("name".to_string(), json!("Alice"))]),
    )
    .unwrap();
    db.insert_one(
        &coll_name,
        HashMap::from([("name".to_string(), json!("Bob"))]),
    )
    .unwrap();
    db.insert_one(
        &coll_name,
        HashMap::from([("name".to_string(), json!("Charlie"))]),
    )
    .unwrap();

    collection
        .create_index("name".to_string(), false, false)
        .unwrap();

    let query = json!({"$and": [{"name": {"$regex": "^A"}}, {"name": {"$regex": ".*ce$"}}]});
    let results = collection.find(&query).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["name"], "Alice");
}

#[test]
fn test_and_count_non_fully_covered() {
    // $and where merged query has both indexed and non-indexed fields
    let (db, coll_name) = create_memory_db();
    let collection = db.collection(&coll_name).unwrap();

    for year in 2020..=2025 {
        for status in ["active", "inactive"] {
            db.insert_one(
                &coll_name,
                HashMap::from([
                    ("year".to_string(), json!(year)),
                    ("status".to_string(), json!(status)),
                ]),
            )
            .unwrap();
        }
    }

    collection
        .create_index("year".to_string(), false, false)
        .unwrap();

    // year is indexed, status is not → non-fully-covered
    let query = json!({
        "$and": [
            {"year": {"$gte": 2024}},
            {"status": "active"}
        ]
    });

    let count = collection.count_documents(&query).unwrap();
    assert_eq!(count, 2); // 2024+active, 2025+active
}

#[test]
fn test_and_single_clause_merge() {
    // $and with single clause should merge and work
    let (db, coll_name) = create_memory_db();
    let collection = db.collection(&coll_name).unwrap();

    for year in 2020..=2025 {
        db.insert_one(
            &coll_name,
            HashMap::from([("year".to_string(), json!(year))]),
        )
        .unwrap();
    }

    collection
        .create_index("year".to_string(), false, false)
        .unwrap();

    let query = json!({"$and": [{"year": {"$gte": 2024}}]});
    let results = collection.find(&query).unwrap();
    assert_eq!(results.len(), 2); // 2024, 2025
}
