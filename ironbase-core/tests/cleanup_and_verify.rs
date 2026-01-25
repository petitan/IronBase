//! Cleanup and verify - run compaction then check

use ironbase_core::storage::StorageEngine;
use ironbase_core::DatabaseCore;
use serde_json::json;
use std::time::Instant;

#[test]
#[ignore] // Manual debug test - opens production database which may be locked
fn cleanup_and_verify() {
    let db_path = "/home/petitan/MongoLite/mcp-server/ironbase_data.mlite";

    println!("\n=== CLEANUP AND VERIFY ===\n");

    // Open database
    let db = DatabaseCore::<StorageEngine>::open(db_path).expect("Failed to open database");

    // First check the current state
    let coll = db
        .get_collection("emails")
        .expect("Failed to get collection");

    println!("1. Before cleanup:");
    let count = coll.count_documents(&json!({})).unwrap();
    println!("   Total count: {}", count);

    let msg_id = "<176040000936.22970.7963212550205046816@Centrop-server>";
    let find_count = coll.find(&json!({"message_id": msg_id})).unwrap().len();
    let count_count = coll
        .count_documents(&json!({"message_id": msg_id}))
        .unwrap();
    println!("   find() for test message_id: {}", find_count);
    println!("   count_documents() for test message_id: {}", count_count);

    // Try to run compaction
    println!("\n2. Running flush...");
    let start = Instant::now();
    db.flush().expect("Flush failed");
    println!("   Flush took {:?}", start.elapsed());

    // Drop and reopen to reload indexes
    println!("\n3. Closing and reopening database to reload indexes...");
    drop(coll);
    drop(db);

    let db2 = DatabaseCore::<StorageEngine>::open(db_path).expect("Failed to reopen database");
    let coll2 = db2
        .get_collection("emails")
        .expect("Failed to get collection");

    println!("\n4. After reopen:");
    let count = coll2.count_documents(&json!({})).unwrap();
    println!("   Total count: {}", count);

    let find_count = coll2.find(&json!({"message_id": msg_id})).unwrap().len();
    let count_count = coll2
        .count_documents(&json!({"message_id": msg_id}))
        .unwrap();
    println!("   find() for test message_id: {}", find_count);
    println!("   count_documents() for test message_id: {}", count_count);

    // Check duplicates
    println!("\n5. Checking duplicates:");
    let pipeline = json!([
        {"$group": {"_id": "$message_id", "count": {"$sum": 1}}},
        {"$match": {"count": {"$gt": 1}}}
    ]);
    let dups = coll2.aggregate(&pipeline).expect("Aggregation failed");
    println!("   Remaining duplicates: {}", dups.len());

    if dups.is_empty() {
        println!("\n=== SUCCESS! No duplicates remain ===");
    } else {
        println!("\n=== WARNING: Still {} duplicates ===", dups.len());
        for dup in &dups {
            println!("   {:?}", dup);
        }
    }
}
