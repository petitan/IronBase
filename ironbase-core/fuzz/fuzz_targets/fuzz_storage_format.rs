#![no_main]

use libfuzzer_sys::fuzz_target;
use std::io::Write;
use tempfile::TempDir;

// Fuzz target: Storage file format parsing
// Goal: Find panics when parsing corrupted/malformed .mlite files

/// Magic number for IronBase files
const MAGIC: &[u8; 8] = b"MONGOLTE";
const HEADER_SIZE: usize = 256;

fuzz_target!(|data: &[u8]| {
    // Skip very small inputs
    if data.len() < 32 {
        return;
    }

    let temp_dir = match TempDir::new() {
        Ok(dir) => dir,
        Err(_) => return,
    };
    let db_path = temp_dir.path().join("fuzz.mlite");

    // Strategy 1: Write raw fuzz data as database file
    {
        if let Ok(mut file) = std::fs::File::create(&db_path) {
            let _ = file.write_all(data);
            let _ = file.sync_all();
        }

        // Try to open - should handle corruption gracefully
        let result = ironbase_core::DatabaseCore::open(&db_path);
        // We don't care if it fails, just that it doesn't panic
        drop(result);
    }

    // Strategy 2: Create valid header with fuzz data as metadata
    if data.len() >= 8 {
        let mut header = vec![0u8; HEADER_SIZE];

        // Set magic number
        header[0..8].copy_from_slice(MAGIC);

        // Set version (fuzz the version number)
        let version = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        header[8..12].copy_from_slice(&version.to_le_bytes());

        // Set page size
        header[12..16].copy_from_slice(&4096u32.to_le_bytes());

        // Set collection count (from fuzz data)
        let collection_count = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
        header[16..20].copy_from_slice(&collection_count.to_le_bytes());

        // Append rest of fuzz data as "metadata"
        let mut file_data = header;
        file_data.extend_from_slice(&data[8..]);

        if let Ok(mut file) = std::fs::File::create(&db_path) {
            let _ = file.write_all(&file_data);
            let _ = file.sync_all();
        }

        let result = ironbase_core::DatabaseCore::open(&db_path);
        drop(result);
    }

    // Strategy 3: Valid header with garbage document data
    if data.len() >= 16 {
        let mut file_data = vec![0u8; HEADER_SIZE];

        // Valid header
        file_data[0..8].copy_from_slice(MAGIC);
        file_data[8..12].copy_from_slice(&3u32.to_le_bytes()); // Version 3
        file_data[12..16].copy_from_slice(&4096u32.to_le_bytes()); // Page size
        file_data[16..20].copy_from_slice(&0u32.to_le_bytes()); // No collections

        // Set metadata offset to point to our fuzz data
        let metadata_offset = (HEADER_SIZE + 10 * 1024 * 1024) as u64; // After reserved space
        file_data[48..56].copy_from_slice(&metadata_offset.to_le_bytes());

        // Pad to metadata offset and add fuzz data
        file_data.resize(metadata_offset as usize, 0);
        file_data.extend_from_slice(data);

        if let Ok(mut file) = std::fs::File::create(&db_path) {
            let _ = file.write_all(&file_data);
            let _ = file.sync_all();
        }

        let result = ironbase_core::DatabaseCore::open(&db_path);
        drop(result);
    }

    // Cleanup
    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_file(db_path.with_extension("mlite.lock"));
    let _ = std::fs::remove_file(db_path.with_extension("mlite.wal"));
});
