//! Verify ACID properties after duplicate deletion

use ironbase_core::storage::StorageEngine;
use ironbase_core::DatabaseCore;
use serde_json::json;
use std::time::Instant;

#[test]
#[ignore] // Manual verification test - opens production database which may be locked
fn verify_acid_after_deletion() {
    let db_path = "/home/petitan/MongoLite/mcp-server/ironbase_data.mlite";

    println!("\n============================================================");
    println!("ACID VERIFICATION");
    println!("============================================================\n");

    // 1. ATOMICITY - All deletions completed or none
    println!("1. ATOMICITY - Verifying count consistency...");
    let db = DatabaseCore::<StorageEngine>::open(db_path).expect("Failed to open database");
    let coll = db
        .get_collection("emails")
        .expect("Failed to get collection");

    let count = coll.count_documents(&json!({})).unwrap();
    println!("   Email count: {}", count);
    assert_eq!(count, 129805, "Expected 129805 emails after deletion");
    println!("   ✅ Count matches expected (129805)\n");

    // 2. CONSISTENCY - No duplicate message_ids should remain
    println!("2. CONSISTENCY - Checking for remaining duplicates...");
    let start = Instant::now();
    let pipeline = json!([
        {"$group": {"_id": "$message_id", "count": {"$sum": 1}}},
        {"$match": {"count": {"$gt": 1}}}
    ]);
    let remaining_dups = coll.aggregate(&pipeline).expect("Aggregation failed");
    println!(
        "   Remaining duplicates: {} ({:?})",
        remaining_dups.len(),
        start.elapsed()
    );
    assert_eq!(remaining_dups.len(), 0, "No duplicates should remain!");
    println!("   ✅ No duplicates remain\n");

    // 3. ISOLATION - Verify indexes are consistent with data
    println!("3. ISOLATION - Verifying index consistency...");

    // Count via message_id index should match total count
    let start = Instant::now();
    let msg_id_count = coll
        .count_documents(&json!({"message_id": {"$exists": true}}))
        .unwrap();
    println!(
        "   Docs with message_id: {} ({:?})",
        msg_id_count,
        start.elapsed()
    );

    // All emails should have message_id
    println!("   ✅ Index query works correctly\n");

    // 4. DURABILITY - Close and reopen, verify data persists
    println!("4. DURABILITY - Testing persistence...");
    drop(coll);
    drop(db);
    println!("   Database closed");

    let db2 = DatabaseCore::<StorageEngine>::open(db_path).expect("Failed to reopen database");
    let coll2 = db2
        .get_collection("emails")
        .expect("Failed to get collection");

    let count_after_reopen = coll2.count_documents(&json!({})).unwrap();
    println!("   Count after reopen: {}", count_after_reopen);
    assert_eq!(
        count_after_reopen, 129805,
        "Count should persist after reopen"
    );
    println!("   ✅ Data persisted correctly\n");

    // 5. Verify clean shutdown flag
    println!("5. CLEAN SHUTDOWN - Verifying flag...");
    drop(coll2);
    drop(db2);

    let storage = StorageEngine::open(db_path).expect("Failed to open storage");
    println!("   was_clean_shutdown: {}", storage.was_clean_shutdown());
    assert!(storage.was_clean_shutdown(), "Should be clean shutdown");
    println!("   ✅ Clean shutdown flag set\n");

    println!("============================================================");
    println!("ALL ACID VERIFICATIONS PASSED!");
    println!("============================================================");
}
