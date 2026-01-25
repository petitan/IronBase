//! Detailed duplicate check

use ironbase_core::storage::StorageEngine;
use ironbase_core::DatabaseCore;
use serde_json::json;

#[test]
#[ignore] // Manual debug test - opens production database which may be locked
fn detailed_duplicate_check() {
    let db_path = "/home/petitan/MongoLite/mcp-server/ironbase_data.mlite";

    let db = DatabaseCore::<StorageEngine>::open(db_path).expect("Failed to open database");
    let coll = db
        .get_collection("emails")
        .expect("Failed to get collection");

    println!("\n=== DETAILED DUPLICATE CHECK ===\n");

    // Test one specific message_id
    let msg_id = "<176040000936.22970.7963212550205046816@Centrop-server>";

    println!("Checking message_id: {}\n", msg_id);

    // Method 1: Direct find
    let docs = coll.find(&json!({"message_id": msg_id})).unwrap();
    println!("1. Direct find result: {} documents", docs.len());
    for doc in &docs {
        println!("   _id: {}", doc.get("_id").unwrap());
    }

    // Method 2: Count with filter
    let count = coll
        .count_documents(&json!({"message_id": msg_id}))
        .unwrap();
    println!("\n2. count_documents result: {}", count);

    // Method 3: Aggregation count
    let pipeline = json!([
        {"$match": {"message_id": msg_id}},
        {"$group": {"_id": null, "count": {"$sum": 1}}}
    ]);
    let agg_result = coll.aggregate(&pipeline).unwrap();
    println!("\n3. Aggregation count result: {:?}", agg_result);

    // Method 4: Full scan count (sanity check)
    println!(
        "\n4. Total emails: {}",
        coll.count_documents(&json!({})).unwrap()
    );

    // Method 5: Re-run duplicate detection
    println!("\n5. Re-running duplicate detection...");
    let pipeline = json!([
        {"$group": {"_id": "$message_id", "count": {"$sum": 1}}},
        {"$match": {"count": {"$gt": 1}}}
    ]);
    let dups = coll.aggregate(&pipeline).unwrap();
    println!("   Found {} duplicate message_ids", dups.len());

    for dup in dups.iter().take(3) {
        println!("   {:?}", dup);
    }
}
