//! Debug test for delete_many performance issue

use ironbase_core::storage::StorageEngine;
use ironbase_core::DatabaseCore;
use serde_json::json;
use std::time::Instant;

#[test]
#[ignore] // Manual debug test - opens production database which may be locked
fn test_delete_many_with_in() {
    // Open database with timing
    println!("\n=== Opening database... ===");
    let open_start = Instant::now();
    let db = DatabaseCore::<StorageEngine>::open(
        "/home/petitan/MongoLite/mcp-server/ironbase_data.mlite",
    )
    .expect("Failed to open database");
    println!("Database opened in {:?}", open_start.elapsed());

    // Test 1: delete_one with exact _id (should be fast)
    println!("\n=== Test 1: delete_one with exact _id ===");
    let start = Instant::now();
    let _ = db.delete_one("emails", &json!({"_id": 999999999})); // Non-existent
    println!("delete_one took {:?}", start.elapsed());

    // Test 2: delete_many with $in containing single non-existent _id
    println!("\n=== Test 2: delete_many with $in [1 id] ===");
    let start = Instant::now();
    let query = json!({"_id": {"$in": [888888888]}});
    println!("Query: {}", query);
    println!("Starting delete_many...");
    let result = db.delete_many("emails", &query);
    println!("delete_many took {:?}", start.elapsed());
    println!("Result: {:?}", result);

    // Test 3: delete_many with empty filter (just to see if it returns)
    println!("\n=== Test 3: count_documents (baseline) ===");
    let start = Instant::now();
    let coll = db.collection("emails").unwrap();
    let count = coll.count_documents(&json!({})).unwrap();
    println!(
        "count_documents returned {} in {:?}",
        count,
        start.elapsed()
    );

    println!("\n=== All tests completed ===");
}
