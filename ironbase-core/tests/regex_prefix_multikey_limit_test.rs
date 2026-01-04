use ironbase_core::{storage::MemoryStorage, DatabaseCore, FindOptions};
use serde_json::{json, Value};
use std::collections::HashMap;

fn json_to_hashmap(v: Value) -> HashMap<String, Value> {
    match v {
        Value::Object(map) => map.into_iter().collect(),
        _ => panic!("Expected object"),
    }
}

#[test]
fn test_regex_prefix_multikey_limit_returns_full_page() {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    let coll = db.collection("emails").unwrap();

    coll.create_index("tags".to_string(), false, false).unwrap();

    db.insert_one(
        "emails",
        json_to_hashmap(json!({"tags": ["apple", "apricot"]})),
    )
    .unwrap();
    db.insert_one(
        "emails",
        json_to_hashmap(json!({"tags": ["apex", "april"]})),
    )
    .unwrap();
    db.insert_one("emails", json_to_hashmap(json!({"tags": ["banana"]})))
        .unwrap();

    let options = FindOptions::new().with_limit(2);
    let results = coll
        .find_with_options(&json!({"tags": {"$regex": "^ap"}}), options)
        .unwrap();

    assert_eq!(results.len(), 2);
}
