#![no_main]

use libfuzzer_sys::fuzz_target;
use ironbase_core::DatabaseCore;
use ironbase_core::storage::MemoryStorage;
use std::collections::HashMap;
use serde_json::json;

// Fuzz target: Query parser with arbitrary JSON
// Goal: Find panics or crashes when parsing arbitrary query JSON

fuzz_target!(|data: &[u8]| {
    // Try to parse as JSON
    if let Ok(query) = serde_json::from_slice::<serde_json::Value>(data) {
        // Create in-memory database
        let db = match DatabaseCore::<MemoryStorage>::open_memory() {
            Ok(db) => db,
            Err(_) => return,
        };

        // Insert some documents to query against
        for i in 0..5 {
            let doc = HashMap::from([
                ("_id".to_string(), json!(i)),
                ("value".to_string(), json!(i * 10)),
                ("name".to_string(), json!(format!("item_{}", i))),
            ]);
            let _ = db.insert_one("fuzz", doc);
        }

        // Get collection for query operations
        let collection = match db.collection("fuzz") {
            Ok(c) => c,
            Err(_) => return,
        };

        // Try various query operations - should NEVER panic
        let _ = collection.find(&query);
        let _ = collection.find_one(&query);
        let _ = collection.count_documents(&query);

        // Try delete operations via db (these exist on MemoryStorage impl)
        let _ = db.delete_one("fuzz", &query);
        let _ = db.delete_many("fuzz", &query);

        // Try as update query
        let _ = db.update_one("fuzz", &query, &json!({"$set": {"fuzzed": true}}));
        let _ = db.update_many("fuzz", &query, &json!({"$inc": {"count": 1}}));
    }
});
