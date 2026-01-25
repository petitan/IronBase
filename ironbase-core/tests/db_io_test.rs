//! Test basic DB I/O timing

use ironbase_core::storage::StorageEngine;
use ironbase_core::DatabaseCore;
use serde_json::json;
use std::time::Instant;

#[test]
#[ignore] // Manual debug test - opens production database which may be locked
fn test_basic_io() {
    println!("\n=== Opening database... ===");
    let start = Instant::now();
    let db = DatabaseCore::<StorageEngine>::open(
        "/home/petitan/MongoLite/mcp-server/ironbase_data.mlite",
    )
    .expect("Failed to open database");
    println!("Database opened in {:?}", start.elapsed());

    // Basic operations
    println!("\n=== Test 1: count_documents (should be <1s) ===");
    let start = Instant::now();
    let coll = db.get_collection("emails").unwrap();
    let count = coll.count_documents(&json!({})).unwrap();
    println!("count_documents: {} in {:?}", count, start.elapsed());

    println!("\n=== Test 2: delete_one non-existent (should be <100ms) ===");
    let start = Instant::now();
    let result = db.delete_one("emails", &json!({"_id": 999999999999_i64}));
    println!("delete_one result: {:?} in {:?}", result, start.elapsed());

    println!("\n=== Test 3: delete_many $in non-existent (should be <100ms) ===");
    let start = Instant::now();
    let result = db.delete_many(
        "emails",
        &json!({"_id": {"$in": [888888888_i64, 888888889_i64]}}),
    );
    println!("delete_many result: {:?} in {:?}", result, start.elapsed());

    println!("\n=== All tests complete ===");
}
