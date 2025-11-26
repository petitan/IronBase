#![no_main]

use libfuzzer_sys::fuzz_target;
use ironbase_core::storage::MemoryStorage;
use ironbase_core::storage::Storage;
use ironbase_core::CollectionCore;
use parking_lot::RwLock;
use std::sync::Arc;
use std::collections::HashMap;
use serde_json::json;

// Fuzz target: Query parser with arbitrary JSON
// Goal: Find panics or crashes when parsing arbitrary query JSON

fuzz_target!(|data: &[u8]| {
    // Try to parse as JSON
    if let Ok(query) = serde_json::from_slice::<serde_json::Value>(data) {
        // Create a minimal collection to test queries against
        let storage = Arc::new(RwLock::new(MemoryStorage::new()));

        {
            let mut s = storage.write();
            let _ = s.create_collection("fuzz");
        }

        if let Ok(collection) = CollectionCore::new("fuzz".to_string(), Arc::clone(&storage)) {
            // Insert some documents to query against
            for i in 0..5 {
                let doc = HashMap::from([
                    ("_id".to_string(), json!(i)),
                    ("value".to_string(), json!(i * 10)),
                    ("name".to_string(), json!(format!("item_{}", i))),
                ]);
                let _ = collection.insert_one(doc);
            }

            // Try various query operations - should NEVER panic
            let _ = collection.find(&query);
            let _ = collection.find_one(&query);
            let _ = collection.count_documents(&query);
            let _ = collection.delete_one(&query);
            let _ = collection.delete_many(&query);

            // Try as update query
            let _ = collection.update_one(&query, &json!({"$set": {"fuzzed": true}}));
            let _ = collection.update_many(&query, &json!({"$inc": {"count": 1}}));
        }
    }
});
