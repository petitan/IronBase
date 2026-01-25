//! One-time fix: Rebuild indexes and mark clean shutdown
//!
//! This test will:
//! 1. Open the database
//! 2. Trigger index rebuild (first get_collection)
//! 3. Save .idx files and mark clean shutdown
//!
//! After this, subsequent opens will be FAST!

use ironbase_core::storage::StorageEngine;
use ironbase_core::DatabaseCore;
use std::time::Instant;

#[test]
#[ignore] // Run explicitly with --ignored
fn fix_clean_shutdown() {
    let db_path = "/home/petitan/MongoLite/mcp-server/ironbase_data.mlite";

    println!("\n=== ONE-TIME INDEX REBUILD AND CLEAN SHUTDOWN FIX ===\n");
    println!("This will take 2-3 minutes for 132K documents...\n");

    // Step 1: Open database
    let start = Instant::now();
    println!("1. Opening database...");
    let db = DatabaseCore::<StorageEngine>::open(db_path).expect("Failed to open database");
    println!("   Database opened in {:?}\n", start.elapsed());

    // Step 2: Trigger index rebuild by accessing collection
    let start = Instant::now();
    println!("2. Rebuilding indexes for 'emails' collection...");
    println!("   (This reads all 132K documents - be patient!)");
    let _coll = db
        .get_collection("emails")
        .expect("Failed to get collection");
    println!("   Index rebuild completed in {:?}\n", start.elapsed());

    // Step 3: Test that it works
    let start = Instant::now();
    println!("3. Testing count_documents...");
    let coll = db.get_collection("emails").unwrap();
    let count = coll.count_documents(&serde_json::json!({})).unwrap();
    println!(
        "   count_documents({{}}) = {} in {:?}\n",
        count,
        start.elapsed()
    );

    // Step 4: Close database (triggers mark_clean_shutdown via Drop)
    println!("4. Closing database (saves .idx files + clean shutdown flag)...");
    let start = Instant::now();
    drop(db);
    println!("   Database closed in {:?}\n", start.elapsed());

    // Step 5: Verify clean shutdown
    println!("5. Verifying clean shutdown...");
    let storage = StorageEngine::open(db_path).expect("Failed to reopen storage");
    println!("   was_clean_shutdown() = {}", storage.was_clean_shutdown());

    if storage.was_clean_shutdown() {
        println!("\n=== SUCCESS! ===");
        println!("Next database open will be FAST (indexes loaded from .idx files)");
    } else {
        println!("\n=== WARNING ===");
        println!("Clean shutdown flag not set - check Drop implementation");
    }

    // Check .idx file exists
    let idx_path = format!("{}.idx.emails_id", db_path);
    if std::path::Path::new(&idx_path).exists() {
        let size = std::fs::metadata(&idx_path).map(|m| m.len()).unwrap_or(0);
        println!("\n_id index file created: {} ({} bytes)", idx_path, size);
    }
}
