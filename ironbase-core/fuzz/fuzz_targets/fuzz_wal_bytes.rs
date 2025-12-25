#![no_main]

use libfuzzer_sys::fuzz_target;
use std::collections::HashMap;
use serde_json::json;
use std::io::Write;
use tempfile::TempDir;

// Fuzz target: WAL recovery with corrupted database files
// Goal: Find panics when recovering from corrupted storage
// Note: WAL module is private, so we test via DatabaseCore with file storage

fuzz_target!(|data: &[u8]| {
    // Skip empty inputs
    if data.is_empty() || data.len() < 32 {
        return;
    }

    let temp_dir = match TempDir::new() {
        Ok(dir) => dir,
        Err(_) => return,
    };
    let db_path = temp_dir.path().join("fuzz.mlite");

    // Create a valid database first, then corrupt it
    {
        // Create normal database
        if let Ok(db) = ironbase_core::DatabaseCore::open(&db_path) {
            // Insert some documents
            for i in 0..3 {
                let doc = HashMap::from([
                    ("_id".to_string(), json!(i)),
                    ("data".to_string(), json!(format!("test_{}", i))),
                ]);
                let _ = db.insert_one("test", doc);
            }
            let _ = db.close();
        }
    }

    // Now corrupt the file with fuzz data
    if let Ok(mut file) = std::fs::OpenOptions::new().write(true).open(&db_path) {
        // Seek to random position based on fuzz data and write garbage
        let offset = u64::from_le_bytes([
            data[0], data[1], data[2], data[3],
            data[4], data[5], data[6], data[7],
        ]) % 10000; // Limit offset to reasonable range

        use std::io::Seek;
        let _ = file.seek(std::io::SeekFrom::Start(offset));
        let _ = file.write_all(&data[8..]);
        let _ = file.sync_all();
    }

    // Try to reopen corrupted database - should handle errors gracefully
    let result = ironbase_core::DatabaseCore::open(&db_path);
    if let Ok(db) = result {
        // If it opened, try some operations
        let _ = db.collection("test");
        let _ = db.close();
    }

    // Cleanup
    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_file(db_path.with_extension("mlite.lock"));
    let _ = std::fs::remove_file(db_path.with_extension("mlite.wal"));
});
