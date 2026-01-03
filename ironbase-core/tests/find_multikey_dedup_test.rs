use ironbase_core::{storage::MemoryStorage, DatabaseCore};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

fn json_to_hashmap(v: Value) -> HashMap<String, Value> {
    match v {
        Value::Object(map) => map.into_iter().collect(),
        _ => panic!("Expected object"),
    }
}

#[test]
fn test_find_deduplicates_multikey_index_hits() {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    let coll = db.collection("emails").unwrap();

    // Create multikey index before inserts so array fields are indexed.
    coll.create_index("to.email".to_string(), false, false)
        .unwrap();

    // Insert documents with array field to create multikey index entries.
    // Use db.insert_one() which is the public API (coll.insert_one_raw is pub(crate))
    db.insert_one(
        "emails",
        json_to_hashmap(json!({
            "from": {"email": "a@example.com"},
            "to": [
                {"email": "b@example.com"},
                {"email": "c@example.com"}
            ],
            "date": 1
        })),
    )
    .unwrap();

    db.insert_one(
        "emails",
        json_to_hashmap(json!({
            "from": {"email": "d@example.com"},
            "to": [
                {"email": "e@example.com"},
                {"email": "f@example.com"},
                {"email": "g@example.com"}
            ],
            "date": 2
        })),
    )
    .unwrap();

    db.insert_one(
        "emails",
        json_to_hashmap(json!({
            "from": {"email": "h@example.com"},
            "to": [
                {"email": "i@example.com"}
            ],
            "date": 3
        })),
    )
    .unwrap();

    // Ensure the planner uses the multikey index for a range scan.
    let plan = coll.explain(&json!({"to.email": {"$gte": "a"}})).unwrap();
    let plan_type = plan.get("queryPlan").and_then(|v| v.as_str()).unwrap_or("");
    assert_ne!(plan_type, "CollectionScan");

    let results = coll.find(&json!({"to.email": {"$gte": "a"}})).unwrap();
    assert_eq!(results.len(), 3);

    let mut seen_ids = HashSet::new();
    for doc in results {
        let id = doc.get("_id").cloned().unwrap_or(Value::Null);
        assert!(seen_ids.insert(id));
    }
}
