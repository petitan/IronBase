//! Run storage compaction to fix index inconsistency

use ironbase_core::storage::StorageEngine;
use ironbase_core::DatabaseCore;
use serde_json::json;
use std::time::Instant;

#[test]
#[ignore] // Run explicitly with --ignored (compaction can take time)
fn run_compaction() {
    let db_path = "/home/petitan/MongoLite/mcp-server/ironbase_data.mlite";

    println!("\n=== RUNNING COMPACTION ===\n");

    let db = DatabaseCore::<StorageEngine>::open(db_path).expect("Failed to open database");
    let coll = db
        .get_collection("emails")
        .expect("Failed to get collection");

    println!("Before compaction:");
    let count = coll.count_documents(&json!({})).unwrap();
    println!("  Total count: {}", count);

    let pipeline = json!([
        {"$group": {"_id": "$message_id", "count": {"$sum": 1}}},
        {"$match": {"count": {"$gt": 1}}}
    ]);
    let dups = coll.aggregate(&pipeline).unwrap();
    println!("  Duplicates reported by aggregation: {}", dups.len());

    // Run compaction
    println!("\nRunning compaction...");
    let start = Instant::now();
    let stats = db.compact().expect("Compaction failed");
    println!("Compaction completed in {:?}", start.elapsed());
    println!("Stats: {:?}", stats);

    // Close and reopen to reload indexes
    println!("\nClosing and reopening to reload indexes...");
    drop(coll);
    drop(db);

    let db2 = DatabaseCore::<StorageEngine>::open(db_path).expect("Failed to reopen database");
    let coll2 = db2
        .get_collection("emails")
        .expect("Failed to get collection");

    println!("\nAfter compaction:");
    let count = coll2.count_documents(&json!({})).unwrap();
    println!("  Total count: {}", count);

    let dups = coll2.aggregate(&pipeline).unwrap();
    println!("  Duplicates reported by aggregation: {}", dups.len());

    if dups.is_empty() {
        println!("\n=== SUCCESS! Index inconsistency fixed ===");
    } else {
        println!(
            "\n=== WARNING: Still {} duplicates reported ===",
            dups.len()
        );
    }
}
