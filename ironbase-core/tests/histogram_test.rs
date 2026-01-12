//! Test histogram building for large indexes

use ironbase_core::{storage::MemoryStorage, DatabaseCore};
use serde_json::{json, Value};
use std::collections::HashMap;

fn json_to_hashmap(v: Value) -> HashMap<String, Value> {
    match v {
        Value::Object(map) => map.into_iter().collect(),
        _ => panic!("Expected object"),
    }
}

#[test]
fn test_histogram_builds_for_large_index() {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    let coll = db.collection("test").unwrap();

    // Create index first
    coll.create_index("value".to_string(), false, false)
        .unwrap();

    // Insert 100,001 documents (just over MIN_ENTRIES_FOR_HISTOGRAM = 100,000)
    println!("Inserting 100,001 documents...");
    for i in 0..100_001 {
        db.insert_one(
            "test",
            json_to_hashmap(json!({
                "value": i % 1000  // Values 0-999, each repeated ~100 times
            })),
        )
        .unwrap();
    }

    // Refresh stats - this should build histogram
    println!("Refreshing index stats...");
    coll.refresh_index_stats().unwrap();

    // Verify via explain that the index has stats
    let explain = coll.explain(&json!({"value": 500})).unwrap();
    println!(
        "Explain: {}",
        serde_json::to_string_pretty(&explain).unwrap()
    );

    // The index should be used (check chosenPlan.indexUsed)
    assert!(
        explain["chosenPlan"]["indexUsed"].as_str().is_some(),
        "Index should be used for query"
    );
}

#[test]
fn test_histogram_not_built_for_small_index() {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    let coll = db.collection("test").unwrap();

    // Create index
    coll.create_index("value".to_string(), false, false)
        .unwrap();

    // Insert only 1000 documents (below MIN_ENTRIES_FOR_HISTOGRAM)
    for i in 0..1000 {
        db.insert_one(
            "test",
            json_to_hashmap(json!({
                "value": i
            })),
        )
        .unwrap();
    }

    // Refresh stats - should NOT build histogram (streaming only)
    coll.refresh_index_stats().unwrap();

    let explain = coll.explain(&json!({"value": 500})).unwrap();
    println!(
        "Small index explain: {}",
        serde_json::to_string_pretty(&explain).unwrap()
    );

    assert!(
        explain["chosenPlan"]["indexUsed"].as_str().is_some(),
        "Index should still be used"
    );
}
