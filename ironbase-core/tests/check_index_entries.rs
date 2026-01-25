//! Check index entries directly

use ironbase_core::storage::StorageEngine;
use ironbase_core::DatabaseCore;
use serde_json::json;

#[test]
#[ignore] // Manual debug test - opens production database which may be locked
fn check_index_entries() {
    let db_path = "/home/petitan/MongoLite/mcp-server/ironbase_data.mlite";

    println!("\n=== CHECK INDEX ENTRIES ===\n");

    let db = DatabaseCore::<StorageEngine>::open(db_path).expect("Failed to open database");
    let coll = db
        .get_collection("emails")
        .expect("Failed to get collection");

    let msg_id = "<176040000936.22970.7963212550205046816@Centrop-server>";
    println!("Checking message_id: {}\n", msg_id);

    // Find via collection
    let docs = coll.find(&json!({"message_id": msg_id})).unwrap();
    println!("1. find() returned {} documents:", docs.len());
    for doc in &docs {
        println!("   _id: {}", doc.get("_id").unwrap());
    }

    // Get the index manager directly
    // Note: This requires internal access which might not be directly exposed
    // Let's check what explain tells us
    println!("\n2. explain() for message_id query:");
    let explain = coll.explain(&json!({"message_id": msg_id})).unwrap();
    println!("{}", serde_json::to_string_pretty(&explain).unwrap());

    // Check by reading documents directly
    println!("\n3. Checking raw storage for this message_id:");

    // Scan for docs with this message_id (brute force)
    // This will show if there are tombstones
    let all_docs = coll.find(&json!({})).unwrap();
    let mut found_live = 0;
    for doc in &all_docs {
        if doc.get("message_id").and_then(|v| v.as_str()) == Some(msg_id) {
            found_live += 1;
            let is_tombstone = doc
                .get("_tombstone")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            println!(
                "   Found doc _id={}, tombstone={}",
                doc.get("_id").unwrap(),
                is_tombstone
            );
        }
    }
    println!("   Total live docs with this message_id: {}", found_live);
}
