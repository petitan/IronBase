#![no_main]

use libfuzzer_sys::fuzz_target;
use ironbase_core::document::Document;
use ironbase_core::storage::MemoryStorage;
use ironbase_core::storage::Storage;
use ironbase_core::CollectionCore;
use parking_lot::RwLock;
use std::sync::Arc;
use std::collections::HashMap;

// Fuzz target: Document parsing and insertion
// Goal: Find panics when handling arbitrary document data

fuzz_target!(|data: &[u8]| {
    // Try to parse as JSON document
    if let Ok(doc_value) = serde_json::from_slice::<serde_json::Value>(data) {
        // Test Document::from_json
        if let Ok(json_str) = serde_json::to_string(&doc_value) {
            let _ = Document::from_json(&json_str);
        }

        // Test document insertion with arbitrary JSON
        if doc_value.is_object() {
            let storage = Arc::new(RwLock::new(MemoryStorage::new()));

            {
                let mut s = storage.write();
                let _ = s.create_collection("fuzz_doc");
            }

            if let Ok(collection) = CollectionCore::new("fuzz_doc".to_string(), Arc::clone(&storage)) {
                // Convert to HashMap for insertion
                if let Some(obj) = doc_value.as_object() {
                    let doc: HashMap<String, serde_json::Value> = obj
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect();

                    // Insert should not panic
                    let _ = collection.insert_one(doc);
                }
            }
        }
    }

    // Also test raw bytes as if they were a stored document
    // This simulates reading corrupted data from storage
    let _ = serde_json::from_slice::<serde_json::Value>(data);

    // Test UTF-8 handling
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = Document::from_json(s);
    }
});
