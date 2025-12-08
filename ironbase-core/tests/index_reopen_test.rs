// Test for index persistence after database reopen
use ironbase_core::{DatabaseCore, StorageEngine};
use serde_json::json;
use std::collections::HashMap;
use std::fs;

#[test]
fn test_index_reopen_with_documents() {
    let test_path = "/tmp/test_index_reopen.mlite";
    let wal_path = "/tmp/test_index_reopen.wal";

    // Clean up
    let _ = fs::remove_file(test_path);
    let _ = fs::remove_file(wal_path);
    // Remove any .idx files
    if let Ok(entries) = fs::read_dir("/tmp") {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if name.starts_with("test_index_reopen_") && name.ends_with(".idx") {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }
    }

    // Phase 1: Create index and insert documents
    {
        println!("Phase 1: Creating database and index...");
        let db = DatabaseCore::<StorageEngine>::open(test_path).unwrap();
        let coll = db.collection("test").unwrap();

        println!("Creating index on 'name' field...");
        coll.create_index("name".to_string(), false).unwrap();

        println!("Inserting 10 documents...");
        for i in 1..=10 {
            let mut fields: HashMap<String, serde_json::Value> = HashMap::new();
            fields.insert("_id".to_string(), json!(i));
            fields.insert("name".to_string(), json!(format!("doc_{}", i)));
            db.insert_one("test", fields).unwrap();
        }

        let count = coll.count_documents(&serde_json::json!({})).unwrap();
        println!("Documents before close: {}", count);
        assert_eq!(count, 10);

        println!("Dropping database (implicit close)...");
    }

    // Phase 2: Reopen and check
    println!("\nPhase 2: Reopening database...");
    {
        let result = DatabaseCore::<StorageEngine>::open(test_path);
        match &result {
            Ok(_) => println!("Database opened OK"),
            Err(e) => println!("ERROR opening database: {:?}", e),
        }

        let db = result.unwrap();

        println!("Accessing collection...");
        let coll = db.collection("test").unwrap();

        println!("Counting documents...");
        let count = coll.count_documents(&serde_json::json!({})).unwrap();
        println!("Documents after reopen: {}", count);

        assert_eq!(count, 10, "Document count mismatch after reopen!");
    }

    // Clean up
    let _ = fs::remove_file(test_path);
    let _ = fs::remove_file(wal_path);
    if let Ok(entries) = fs::read_dir("/tmp") {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if name.starts_with("test_index_reopen_") && name.ends_with(".idx") {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }
    }
}
