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
