//! Delete duplicate emails based on message_id
//!
//! This test:
//! 1. Uses aggregation to find duplicate message_ids
//! 2. For each duplicate, finds all docs and keeps the first _id
//! 3. Deletes the rest using delete_many with $in (fast path)

use ironbase_core::storage::StorageEngine;
use ironbase_core::DatabaseCore;
use serde_json::{json, Value};
use std::time::Instant;

#[test]
#[ignore] // Run explicitly with --ignored
fn delete_duplicate_emails() {
    let db_path = "/home/petitan/MongoLite/mcp-server/ironbase_data.mlite";

    println!("\n============================================================");
    println!("DUPLICATE EMAIL DELETION");
    println!("============================================================\n");

    // Open database
    let start = Instant::now();
    println!("Opening database...");
    let db = DatabaseCore::<StorageEngine>::open(db_path).expect("Failed to open database");
    println!("Database opened in {:?}\n", start.elapsed());

    // Get collection (this may take a while on first access)
    let start = Instant::now();
    println!("Getting collection (may rebuild indexes)...");
    let coll = db
        .get_collection("emails")
        .expect("Failed to get collection");
    println!("Collection ready in {:?}\n", start.elapsed());

    // Step 1: Count before
    let start = Instant::now();
    let count_before = coll.count_documents(&json!({})).unwrap();
    println!(
        "1. Count before: {} ({:?})\n",
        count_before,
        start.elapsed()
    );

    // Step 2: Find duplicate message_ids using aggregation
    println!("2. Finding duplicates via aggregation...");
    let start = Instant::now();
    let pipeline = json!([
        {"$group": {"_id": "$message_id", "count": {"$sum": 1}}},
        {"$match": {"count": {"$gt": 1}}}
    ]);
    let duplicates = coll.aggregate(&pipeline).expect("Aggregation failed");
    println!(
        "   Found {} message_ids with duplicates ({:?})\n",
        duplicates.len(),
        start.elapsed()
    );

    if duplicates.is_empty() {
        println!("No duplicates found!");
        return;
    }

    // Calculate expected deletions
    let total_to_delete: i64 = duplicates
        .iter()
        .filter_map(|d| d.get("count")?.as_i64())
        .map(|c| c - 1)
        .sum();
    println!("   Expected deletions: {}\n", total_to_delete);

    // Step 3: Collect _ids to delete
    println!("3. Collecting _ids to delete...");
    let start = Instant::now();
    let mut all_ids_to_delete: Vec<Value> = Vec::new();

    for (i, dup) in duplicates.iter().enumerate() {
        let msg_id = match dup.get("_id") {
            Some(v) => v.clone(),
            None => continue,
        };

        // Find all docs with this message_id
        let docs = coll
            .find(&json!({"message_id": msg_id}))
            .expect("Find failed");

        if docs.len() > 1 {
            // Get _ids, sort by string representation, keep first, delete rest
            let mut ids: Vec<Value> = docs.iter().filter_map(|d| d.get("_id").cloned()).collect();
            ids.sort_by_key(|a| a.to_string());
            all_ids_to_delete.extend(ids.into_iter().skip(1));
        }

        if (i + 1) % 500 == 0 {
            println!(
                "   Progress: {}/{}, collected {} ids",
                i + 1,
                duplicates.len(),
                all_ids_to_delete.len()
            );
        }
    }
    println!(
        "   Total _ids to delete: {} ({:?})\n",
        all_ids_to_delete.len(),
        start.elapsed()
    );

    if all_ids_to_delete.is_empty() {
        println!("Nothing to delete!");
        return;
    }

    // Step 4: Delete in batches
    println!("4. Deleting duplicates...");
    let start = Instant::now();
    const BATCH_SIZE: usize = 500;
    let mut total_deleted = 0u64;

    for (batch_num, chunk) in all_ids_to_delete.chunks(BATCH_SIZE).enumerate() {
        let ids_json: Vec<Value> = chunk.to_vec();

        let result = db
            .delete_many("emails", &json!({"_id": {"$in": ids_json}}))
            .expect("Delete failed");
        total_deleted += result;
        println!(
            "   Batch {}: deleted {} (total: {})",
            batch_num + 1,
            result,
            total_deleted
        );
    }
    println!(
        "   Total deleted: {} ({:?})\n",
        total_deleted,
        start.elapsed()
    );

    // Step 5: Verify
    println!("5. Verifying...");
    let count_after = coll.count_documents(&json!({})).unwrap();
    println!("   Count after: {}", count_after);
    println!(
        "   Difference: {}\n",
        count_before as i64 - count_after as i64
    );

    println!("============================================================");
    if count_before as i64 - count_after as i64 == total_to_delete {
        println!("SUCCESS! All duplicates deleted.");
    } else {
        println!(
            "WARNING: Expected {}, actual {}",
            total_to_delete,
            count_before as i64 - count_after as i64
        );
    }
    println!("============================================================");
}
