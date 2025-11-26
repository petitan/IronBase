#![no_main]

use libfuzzer_sys::fuzz_target;
use ironbase_core::wal::{WALEntry, WALEntryType, WriteAheadLog};
use std::io::Write;
use tempfile::TempDir;

// Fuzz target: WAL parsing with arbitrary bytes
// Goal: Find panics when parsing corrupted/malformed WAL data

fuzz_target!(|data: &[u8]| {
    // Skip empty inputs
    if data.is_empty() {
        return;
    }

    let temp_dir = TempDir::new().unwrap();
    let wal_path = temp_dir.path().join("fuzz.wal");

    // Write fuzzed data directly to WAL file
    {
        let mut file = std::fs::File::create(&wal_path).unwrap();
        file.write_all(data).unwrap();
        file.sync_all().unwrap();
    }

    // Try to open and recover - should NEVER panic
    if let Ok(mut wal) = WriteAheadLog::open(&wal_path) {
        // Recovery should handle any garbage gracefully
        let _ = wal.recover();
    }

    // Also test WALEntry::deserialize directly
    let _ = WALEntry::deserialize(data);

    // Test with data as multiple concatenated entries
    if data.len() >= 17 {
        // Minimum WAL entry size
        let _ = WALEntry::deserialize(&data[..17]);

        if data.len() >= 34 {
            let _ = WALEntry::deserialize(&data[17..34]);
        }
    }
});
