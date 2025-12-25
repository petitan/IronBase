#![no_main]

use libfuzzer_sys::fuzz_target;
use ironbase_core::DatabaseCore;
use ironbase_core::storage::MemoryStorage;
use std::collections::HashMap;
use serde_json::json;

// Fuzz target: Aggregation pipeline with arbitrary JSON
// Goal: Find panics or crashes when parsing arbitrary aggregation pipelines

fuzz_target!(|data: &[u8]| {
    // Try to parse as JSON array (pipeline)
    if let Ok(pipeline) = serde_json::from_slice::<serde_json::Value>(data) {
        // Must be an array for aggregation
        if !pipeline.is_array() {
            return;
        }

        let db = match DatabaseCore::<MemoryStorage>::open_memory() {
            Ok(db) => db,
            Err(_) => return,
        };

        // Insert diverse test documents for aggregation
        let docs = vec![
            json!({"_id": 1, "category": "A", "value": 100, "name": "doc1", "nested": {"x": 10}}),
            json!({"_id": 2, "category": "B", "value": 200, "name": "doc2", "nested": {"x": 20}}),
            json!({"_id": 3, "category": "A", "value": 150, "name": "doc3", "nested": {"x": 30}}),
            json!({"_id": 4, "category": "C", "value": 50, "name": "doc4", "nested": {"x": 40}}),
            json!({"_id": 5, "category": "B", "value": 300, "name": "doc5", "nested": {"x": 50}}),
        ];

        for doc in docs {
            if let Some(obj) = doc.as_object() {
                let doc_map: HashMap<String, serde_json::Value> = obj
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                let _ = db.insert_one("fuzz_agg", doc_map);
            }
        }

        // Get collection for aggregation
        let collection = match db.collection("fuzz_agg") {
            Ok(c) => c,
            Err(_) => return,
        };

        // Execute aggregation pipeline - should NEVER panic
        let _ = collection.aggregate(&pipeline);

        // Test with various malformed pipelines
        let _ = collection.aggregate(&json!([pipeline])); // Nested array
        let _ = collection.aggregate(&json!([{"$match": pipeline}])); // Pipeline as match
        let _ = collection.aggregate(&json!([{"$group": {"_id": pipeline}}])); // Pipeline as group id
    }
});
