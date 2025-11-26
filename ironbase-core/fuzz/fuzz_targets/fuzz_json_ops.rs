#![no_main]

use libfuzzer_sys::fuzz_target;
use arbitrary::Arbitrary;
use ironbase_core::storage::MemoryStorage;
use ironbase_core::storage::Storage;
use ironbase_core::CollectionCore;
use parking_lot::RwLock;
use std::sync::Arc;
use std::collections::HashMap;
use serde_json::json;

// Structured fuzzing input for more targeted testing
#[derive(Debug, Arbitrary)]
struct FuzzOp {
    op_type: u8,
    field_name: String,
    int_value: i64,
    float_value: f64,
    string_value: String,
    nested_depth: u8,
}

// Fuzz target: Update operators, aggregation, and complex operations
// Goal: Find panics in operator handling

fuzz_target!(|ops: Vec<FuzzOp>| {
    if ops.is_empty() || ops.len() > 100 {
        return;
    }

    let storage = Arc::new(RwLock::new(MemoryStorage::new()));

    {
        let mut s = storage.write();
        let _ = s.create_collection("fuzz_ops");
    }

    let collection = match CollectionCore::new("fuzz_ops".to_string(), Arc::clone(&storage)) {
        Ok(c) => c,
        Err(_) => return,
    };

    // Insert base documents
    for i in 0..10 {
        let doc = HashMap::from([
            ("_id".to_string(), json!(i)),
            ("counter".to_string(), json!(0)),
            ("value".to_string(), json!(i * 10)),
            ("name".to_string(), json!(format!("doc_{}", i))),
            ("tags".to_string(), json!(["a", "b", "c"])),
        ]);
        let _ = collection.insert_one(doc);
    }

    // Apply fuzzed operations
    for op in ops {
        let field = if op.field_name.is_empty() {
            "value".to_string()
        } else {
            // Sanitize field name (limit length, no dots at start)
            let cleaned: String = op.field_name
                .chars()
                .take(50)
                .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '.')
                .collect();
            if cleaned.is_empty() { "value".to_string() } else { cleaned }
        };

        let query = json!({"_id": {"$gte": 0}});

        match op.op_type % 15 {
            // $set
            0 => {
                let update = json!({"$set": {&field: op.int_value}});
                let _ = collection.update_one(&query, &update);
            }
            // $inc
            1 => {
                let update = json!({"$inc": {&field: op.int_value}});
                let _ = collection.update_one(&query, &update);
            }
            // $unset
            2 => {
                let update = json!({"$unset": {&field: ""}});
                let _ = collection.update_one(&query, &update);
            }
            // $push
            3 => {
                let update = json!({"$push": {"tags": op.string_value}});
                let _ = collection.update_one(&query, &update);
            }
            // $pull
            4 => {
                let update = json!({"$pull": {"tags": op.string_value}});
                let _ = collection.update_one(&query, &update);
            }
            // $addToSet
            5 => {
                let update = json!({"$addToSet": {"tags": op.string_value}});
                let _ = collection.update_one(&query, &update);
            }
            // $pop
            6 => {
                let update = json!({"$pop": {"tags": if op.int_value > 0 { 1 } else { -1 }}});
                let _ = collection.update_one(&query, &update);
            }
            // Query operators
            7 => {
                let q = json!({&field: {"$gt": op.int_value}});
                let _ = collection.find(&q);
            }
            8 => {
                let q = json!({&field: {"$in": [op.int_value, op.int_value.saturating_add(1)]}});
                let _ = collection.find(&q);
            }
            9 => {
                let q = json!({"$or": [{&field: op.int_value}, {"name": op.string_value}]});
                let _ = collection.find(&q);
            }
            10 => {
                let q = json!({"$and": [{&field: {"$gte": op.int_value}}, {&field: {"$lte": op.int_value.saturating_add(100)}}]});
                let _ = collection.find(&q);
            }
            // Aggregation
            11 => {
                let pipeline = json!([
                    {"$match": {&field: {"$exists": true}}},
                    {"$group": {"_id": null, "sum": {"$sum": format!("${}", field)}}}
                ]);
                let _ = collection.aggregate(&pipeline);
            }
            12 => {
                let pipeline = json!([
                    {"$sort": {&field: if op.int_value > 0 { 1 } else { -1 }}},
                    {"$limit": (op.int_value.abs() % 100) + 1}
                ]);
                let _ = collection.aggregate(&pipeline);
            }
            // Delete
            13 => {
                let q = json!({&field: op.int_value});
                let _ = collection.delete_one(&q);
            }
            // Insert with fuzzed data
            14 => {
                let doc = HashMap::from([
                    (field.clone(), json!(op.int_value)),
                    ("fuzz_str".to_string(), json!(op.string_value)),
                    ("fuzz_float".to_string(), json!(op.float_value)),
                ]);
                let _ = collection.insert_one(doc);
            }
            _ => {}
        }
    }

    // Final verification - should not panic
    let _ = collection.find(&json!({}));
    let _ = collection.count_documents(&json!({}));
});
