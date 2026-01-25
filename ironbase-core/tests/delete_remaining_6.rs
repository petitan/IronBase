//! Delete remaining 6 duplicates

use ironbase_core::storage::StorageEngine;
use ironbase_core::DatabaseCore;
use serde_json::{json, Value};

#[test]
#[ignore] // Manual debug test - opens production database which may be locked
fn delete_remaining_6() {
    let db_path = "/home/petitan/MongoLite/mcp-server/ironbase_data.mlite";

    println!("\n=== DELETE REMAINING 6 DUPLICATES ===\n");

    let db = DatabaseCore::<StorageEngine>::open(db_path).expect("Failed to open database");
    let coll = db
        .get_collection("emails")
        .expect("Failed to get collection");

    // The 6 duplicate message_ids
    let dup_msg_ids = [
        "<176040000936.22970.7963212550205046816@Centrop-server>",
        "<175072321191.7308.13018124486954210673@Centrop-server>",
        "<174061801063.32643.13258083255032889037@Centrop-server>",
        "<174415681141.13922.10090660113443152550@Centrop-server>",
        "<175746241239.19441.5758483362459855679@Centrop-server>",
        "<175262400989.29652.12414580285633183664@Centrop-server>",
    ];

    let count_before = coll.count_documents(&json!({})).unwrap();
    println!("Count before: {}", count_before);

    let mut deleted_total = 0u64;

    for msg_id in &dup_msg_ids {
        // Find all docs with this message_id
        let docs = coll.find(&json!({"message_id": msg_id})).unwrap();
        println!("\nmessage_id: {}", msg_id);
        println!("  find() returned {} docs", docs.len());

        if docs.len() <= 1 {
            println!("  Skipping (only 1 or 0 docs)");
            continue;
        }

        // Get IDs, sort, keep first, delete rest
        let mut ids: Vec<Value> = docs.iter().filter_map(|d| d.get("_id").cloned()).collect();
        ids.sort_by_key(|a| a.to_string());

        println!("  _ids: {:?}", ids);

        // Delete all but first
        for id in ids.iter().skip(1) {
            let result = db
                .delete_one("emails", &json!({"_id": id}))
                .expect("Delete failed");
            println!("  Deleted _id={}: {}", id, result);
            deleted_total += result;
        }
    }

    println!("\n\nTotal deleted: {}", deleted_total);

    // Verify
    let count_after = coll.count_documents(&json!({})).unwrap();
    println!("Count after: {}", count_after);
    println!("Difference: {}", count_before as i64 - count_after as i64);

    // Check duplicates
    let pipeline = json!([
        {"$group": {"_id": "$message_id", "count": {"$sum": 1}}},
        {"$match": {"count": {"$gt": 1}}}
    ]);
    let remaining = coll.aggregate(&pipeline).unwrap();
    println!("\nRemaining duplicates: {}", remaining.len());

    if remaining.is_empty() {
        println!("\n=== SUCCESS! All duplicates deleted ===");
    } else {
        for dup in &remaining {
            println!("  {:?}", dup);
        }
    }
}
