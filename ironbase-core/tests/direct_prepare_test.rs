//! Direct test for delete_many_prepare

use ironbase_core::storage::StorageEngine;
use ironbase_core::DatabaseCore;
use serde_json::json;
use std::time::Instant;

#[test]
#[ignore] // Manual debug test - opens production database which may be locked
fn test_direct_prepare() {
    println!("\n=== Opening database... ===");
    let start = Instant::now();
    let db = DatabaseCore::<StorageEngine>::open(
        "/home/petitan/MongoLite/mcp-server/ironbase_data.mlite",
    )
    .expect("Failed to open database");
    println!("Database opened in {:?}", start.elapsed());

    // Get collection directly
    println!("\n=== Getting collection... ===");
    let start = Instant::now();
    let _coll = db
        .get_collection("emails")
        .expect("Failed to get collection");
    println!("Got collection in {:?}", start.elapsed());

    // Check if fast path detection works
    let query = json!({"_id": {"$in": [888888888_i64, 888888889_i64]}});
    println!("\n=== Query: {} ===", query);

    // Test the underlying method
    println!("\n=== Calling internal prepare... ===");
    let _start = Instant::now();

    // We can't call delete_many_prepare directly (it's a trait method)
    // Let's just call delete_many and see timing
    println!("Calling delete_many on collection...");
    // Note: We can't call delete_many directly on collection - needs to go through database

    println!("\n=== Calling db.delete_many... ===");
    let start = Instant::now();
    let result = db.delete_many("emails", &query);
    let elapsed = start.elapsed();
    println!("delete_many took: {:?}", elapsed);
    println!("Result: {:?}", result);

    if elapsed.as_secs() > 5 {
        println!("\n!!! SLOW: Fast path may not be working !!!");
    } else {
        println!("\n=== FAST: Fast path is working! ===");
    }
}
