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
fn test_count_with_and_no_index_fallback() {
    let (db, coll_name) = create_memory_db();
    let collection = db.collection(&coll_name).unwrap();

    for i in 0..10 {
        db.insert_one(
            &coll_name,
            HashMap::from([
                (
                    "status".to_string(),
                    json!(if i % 2 == 0 { "active" } else { "inactive" }),
                ),
                ("value".to_string(), json!(i)),
            ]),
        )
        .unwrap();
    }

    // Only index on status; the value clause is not indexed
    collection
        .create_index("status".to_string(), false, false)
        .unwrap();

    let query = json!({
        "$and": [
            {"status": "active"},
            {"value": {"$gt": 3}}
        ]
    });

    let count = collection.count_documents(&query).unwrap();
    assert_eq!(count, 3);
}
