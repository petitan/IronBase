//! Diagnostic test: Check why get_collection is slow
//!
//! This test verifies the root cause hypothesis:
//! - was_clean_shutdown=false causes index rebuild from catalog
//! - Index rebuild loads ALL documents (132K) which is very slow

use ironbase_core::storage::StorageEngine;
use std::time::Instant;

#[test]
#[ignore] // Manual debug test - opens production database which may be locked
fn diagnose_startup_state() {
    let db_path = "/home/petitan/MongoLite/mcp-server/ironbase_data.mlite";

    println!("\n=== Diagnostic: Storage Engine State ===\n");

    // Step 1: Open storage directly (without DatabaseCore)
    let start = Instant::now();
    let storage = StorageEngine::open(db_path).expect("Failed to open storage");
    println!("1. StorageEngine::open() took: {:?}", start.elapsed());

    // Step 2: Check clean shutdown flag
    println!(
        "\n2. was_clean_shutdown() = {}",
        storage.was_clean_shutdown()
    );

    if !storage.was_clean_shutdown() {
        println!("\n   ⚠️  DIRTY SHUTDOWN DETECTED!");
        println!("   → Index files (.idx) will NOT be trusted");
        println!("   → Indexes will be REBUILT from catalog");
        println!("   → This reads ALL documents from disk!");
    }

    // Step 3: Check collection metadata
    if let Some(meta) = storage.get_collection_meta("emails") {
        println!("\n3. emails collection metadata:");
        println!(
            "   - document_catalog.len() = {}",
            meta.document_catalog.len()
        );
        println!("   - indexes.len() = {}", meta.indexes.len());
        println!("   - live_document_count = {}", meta.live_document_count);

        // List indexes
        println!("\n   Persisted indexes:");
        for idx in &meta.indexes {
            println!(
                "   - {} (field: {}, unique: {})",
                idx.name, idx.field, idx.unique
            );
        }
    } else {
        println!("\n3. No 'emails' collection found!");
    }

    // Step 4: Check if .idx files exist
    use std::path::Path;
    let idx_path = format!("{}.idx.emails_id", db_path);
    let idx_exists = Path::new(&idx_path).exists();
    println!("\n4. _id index file exists: {} ({})", idx_exists, idx_path);

    // Other index files
    for suffix in ["emails_message_id", "emails_date"] {
        let path = format!("{}.idx.{}", db_path, suffix);
        if Path::new(&path).exists() {
            let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            println!("   - {}: {} bytes", suffix, size);
        }
    }

    println!("\n=== Diagnosis Complete ===\n");

    // The test passes - we just want to see the output
}
