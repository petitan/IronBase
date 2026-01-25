//! Investigate remaining duplicates

use ironbase_core::storage::StorageEngine;
use ironbase_core::DatabaseCore;
use serde_json::json;

#[test]
#[ignore] // Manual debug test - opens production database which may be locked
fn investigate_remaining_duplicates() {
    let db_path = "/home/petitan/MongoLite/mcp-server/ironbase_data.mlite";

    let db = DatabaseCore::<StorageEngine>::open(db_path).expect("Failed to open database");
    let coll = db
        .get_collection("emails")
        .expect("Failed to get collection");

    println!("\n=== INVESTIGATING REMAINING DUPLICATES ===\n");

    // Find remaining duplicates
    let pipeline = json!([
        {"$group": {"_id": "$message_id", "count": {"$sum": 1}}},
        {"$match": {"count": {"$gt": 1}}}
    ]);
    let dups = coll.aggregate(&pipeline).expect("Aggregation failed");

    println!("Found {} duplicate message_ids:\n", dups.len());

    for dup in &dups {
        let msg_id = dup.get("_id").unwrap();
        let count = dup.get("count").unwrap();
        println!("message_id: {} (count: {})", msg_id, count);

        // Find the actual documents
        let docs = coll.find(&json!({"message_id": msg_id})).unwrap();
        println!("  Documents:");
        for doc in &docs {
            let id = doc.get("_id").unwrap();
            let subject = doc.get("subject").and_then(|v| v.as_str()).unwrap_or("N/A");
            println!("    _id: {}, subject: {:.50}", id, subject);
        }
        println!();
    }
}
