//! Simple delete test

use ironbase_core::storage::StorageEngine;
use ironbase_core::DatabaseCore;
use serde_json::json;
use std::time::Instant;

#[test]
#[ignore] // Manual debug test - opens production database which may be locked
fn test_delete_many_fast_path() {
    println!("\n=== Opening database... ===");
    let db = DatabaseCore::<StorageEngine>::open(
        "/home/petitan/MongoLite/mcp-server/ironbase_data.mlite",
    )
    .expect("Failed to open database");
    println!("Database opened");

    // Test delete_many with $in (fast path)
    println!("\n=== Test: delete_many with $in [2 non-existent ids] ===");
    let query = json!({"_id": {"$in": [888888888_i64, 888888889_i64]}});
    println!("Query: {}", query);

    let start = Instant::now();
    let result = db.delete_many("emails", &query);
    println!("delete_many took: {:?}", start.elapsed());
    println!("Result: {:?}", result);

    // Verify fast path was used (should be <100ms for non-existent ids)
    let elapsed = start.elapsed();
    assert!(
        elapsed.as_millis() < 1000,
        "delete_many took too long ({:?}), fast path may not be working",
        elapsed
    );

    println!("\n=== Test passed! ===");
}
