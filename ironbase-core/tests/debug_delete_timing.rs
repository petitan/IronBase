//! Debug test for delete_one timing breakdown

use ironbase_core::storage::StorageEngine;
use ironbase_core::DatabaseCore;
use serde_json::json;
use std::time::Instant;

#[test]
#[ignore] // Manual debug test - opens production database which may be locked
fn test_delete_timing_breakdown() {
    println!("\n=== Opening database... ===");
    let start = Instant::now();
    let db = DatabaseCore::<StorageEngine>::open(
        "/home/petitan/MongoLite/mcp-server/ironbase_data.mlite",
    )
    .expect("Failed to open database");
    println!("Database opened in {:?}", start.elapsed());

    // Check for active write transaction
    println!("\n=== Checking write transaction status ===");
    let has_active = db.has_active_write_transaction();
    println!("Has active write transaction: {}", has_active);

    if has_active {
        println!("WARNING: There is an active write transaction blocking operations!");
        let holder = db.get_write_lock_holder();
        println!("Lock holder: {:?}", holder);
    }

    // Get a real _id from the collection
    println!("\n=== Getting a real document ID ===");
    let coll = db.collection("emails").unwrap();
    let docs = coll
        .find_with_options(&json!({}), ironbase_core::FindOptions::new().with_limit(1))
        .unwrap();
    let real_id = docs[0].get("_id").unwrap().as_i64().unwrap();
    println!("Real _id: {}", real_id);

    // Test delete_one with non-existent ID (fast path test)
    println!("\n=== Test 1: delete_one with NON-EXISTENT _id ===");
    let query = json!({"_id": 999999999999_i64}); // Definitely doesn't exist
    println!("Query: {}", query);

    let start = Instant::now();
    let result = db.delete_one("emails", &query);
    println!("delete_one took: {:?}", start.elapsed());
    println!("Result: {:?}", result);

    // Test delete_one with existing ID (should also be fast)
    println!("\n=== Test 2: delete_one with EXISTING _id ===");
    let query2 = json!({"_id": real_id});
    println!("Query: {}", query2);

    let start = Instant::now();
    let result2 = db.delete_one("emails", &query2);
    println!("delete_one took: {:?}", start.elapsed());
    println!("Result: {:?}", result2);

    // Test delete_many with $in
    println!("\n=== Test 3: delete_many with $in [2 non-existent ids] ===");
    let query3 = json!({"_id": {"$in": [888888888_i64, 888888889_i64]}});
    println!("Query: {}", query3);

    let start = Instant::now();
    let result3 = db.delete_many("emails", &query3);
    println!("delete_many took: {:?}", start.elapsed());
    println!("Result: {:?}", result3);

    println!("\n=== Tests complete ===");
}
