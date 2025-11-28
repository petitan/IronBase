// chaos_corruption_tests.rs
// Phase 2: Data Corruption Chaos Tests
//
// These tests inject various forms of data corruption and verify that:
// 1. IronBase NEVER panics - always returns Result::Err
// 2. Error messages are meaningful and include context
// 3. Graceful degradation where possible

#![allow(clippy::single_match)]

mod chaos_helpers;

use chaos_helpers::*;
use ironbase_core::error::MongoLiteError;
use ironbase_core::storage::StorageEngine;
use ironbase_core::wal::{WALEntry, WALEntryType, WriteAheadLog};
use serde_json::json;
use tempfile::TempDir;

// =============================================================================
// HEADER CORRUPTION TESTS
// =============================================================================

/// Test: Invalid magic number
/// Expected: Corruption error with "Invalid magic number" message
#[test]
fn test_corrupted_magic_number() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // Create valid database
    {
        let mut storage = StorageEngine::open(&db_path).unwrap();
        storage.create_collection("test").unwrap();
        storage.flush().unwrap();
    }

    // Corrupt magic number at offset 0
    corrupt_bytes_at(&db_path, 0, b"BADMAGIC").unwrap();

    // Open should fail with Corruption error
    let result = StorageEngine::open(&db_path);
    assert!(result.is_err(), "Should fail with corrupted magic");

    match result {
        Err(MongoLiteError::Corruption(msg)) => {
            assert!(
                msg.contains("magic") || msg.contains("Invalid"),
                "Error should mention magic: {}",
                msg
            );
        }
        Err(e) => {
            // Other errors are also acceptable (e.g., deserialization)
            println!("Got different error (acceptable): {:?}", e);
        }
        Ok(_) => panic!("Should have failed to open"),
    }
}

/// Test: Magic number with partial corruption
#[test]
fn test_magic_partial_corruption() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    {
        let mut storage = StorageEngine::open(&db_path).unwrap();
        storage.flush().unwrap();
    }

    // Corrupt just one byte of magic
    corrupt_bytes_at(&db_path, 3, &[0x00]).unwrap();

    let result = StorageEngine::open(&db_path);
    assert!(result.is_err());
}

/// Test: Zero-filled header
#[test]
fn test_zero_filled_header() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    {
        let mut storage = StorageEngine::open(&db_path).unwrap();
        storage.flush().unwrap();
    }

    // Zero out first 100 bytes of header
    corrupt_bytes_at(&db_path, 0, &[0u8; 100]).unwrap();

    let result = StorageEngine::open(&db_path);
    assert!(result.is_err());
}

/// Test: Truncated header (file too short)
#[test]
fn test_truncated_header() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    {
        let mut storage = StorageEngine::open(&db_path).unwrap();
        storage.flush().unwrap();
    }

    // Truncate to 100 bytes (header needs 256)
    truncate_file(&db_path, 100).unwrap();

    let result = StorageEngine::open(&db_path);
    assert!(result.is_err(), "Should fail with truncated header");
}

/// Test: Empty file (0 bytes)
#[test]
fn test_empty_file() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // Create empty file
    std::fs::File::create(&db_path).unwrap();

    // Opening empty file should initialize it (not crash)
    let result = StorageEngine::open(&db_path);
    assert!(result.is_ok(), "Empty file should be initialized");
}

// =============================================================================
// DOCUMENT CORRUPTION TESTS
// =============================================================================

/// Test: Document with zero length
/// Expected: Corruption error mentioning "zero length"
#[test]
fn test_document_zero_length() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    {
        let mut storage = StorageEngine::open(&db_path).unwrap();
        storage.create_collection("test").unwrap();
        storage.flush().unwrap();
    }

    // Write zero-length document marker
    write_length_header_only(&db_path, 0).unwrap();

    // Re-read should detect corruption
    let mut storage = StorageEngine::open(&db_path).unwrap();
    let file_len = storage.file_len().unwrap();

    // Try to read the corrupt document
    // The offset of our corrupt data is at the end of the file minus the header we wrote
    let corrupt_offset = file_len - 4; // We wrote 4 bytes (u32)
    let result = storage.read_data(corrupt_offset);

    assert!(result.is_err());
    match result {
        Err(MongoLiteError::Corruption(msg)) => {
            assert!(msg.contains("zero length"), "Should mention zero: {}", msg);
        }
        Err(e) => println!("Different error (ok): {:?}", e),
        Ok(_) => panic!("Should have failed"),
    }
}

/// Test: Document length exceeds file boundary
#[test]
fn test_document_length_overflow() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    {
        let mut storage = StorageEngine::open(&db_path).unwrap();
        storage.create_collection("test").unwrap();
        storage.flush().unwrap();
    }

    // Write length header claiming huge size
    write_length_header_only(&db_path, u32::MAX).unwrap();

    let mut storage = StorageEngine::open(&db_path).unwrap();
    let file_len = storage.file_len().unwrap();
    let corrupt_offset = file_len - 4;

    let result = storage.read_data(corrupt_offset);
    assert!(result.is_err());

    match result {
        Err(MongoLiteError::Corruption(msg)) => {
            assert!(
                msg.contains("exceed") || msg.contains("boundary"),
                "Should mention boundary: {}",
                msg
            );
        }
        Err(_) => {} // Other errors acceptable
        Ok(_) => panic!("Should fail"),
    }
}

/// Test: Document with invalid JSON
#[test]
fn test_document_invalid_json() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    {
        let mut storage = StorageEngine::open(&db_path).unwrap();
        storage.create_collection("test").unwrap();
        storage.flush().unwrap();
    }

    // Write invalid JSON document
    let invalid_json = b"{ this is not valid json }}}";
    let offset = {
        use std::fs::OpenOptions;
        use std::io::{Seek, SeekFrom, Write};
        let mut file = OpenOptions::new().append(true).open(&db_path).unwrap();
        let off = file.seek(SeekFrom::End(0)).unwrap();
        let len = (invalid_json.len() as u32).to_le_bytes();
        file.write_all(&len).unwrap();
        file.write_all(invalid_json).unwrap();
        file.sync_all().unwrap();
        off
    };

    // Read should succeed (raw bytes), parse should fail
    let mut storage = StorageEngine::open(&db_path).unwrap();
    let data = storage.read_data(offset).unwrap();

    // Parsing as JSON should fail
    let parse_result: Result<serde_json::Value, _> = serde_json::from_slice(&data);
    assert!(parse_result.is_err(), "Invalid JSON should fail to parse");
}

/// Test: Document with binary garbage
#[test]
fn test_document_binary_garbage() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    {
        let mut storage = StorageEngine::open(&db_path).unwrap();
        storage.create_collection("test").unwrap();
        storage.flush().unwrap();
    }

    // Write binary garbage
    let garbage = [0xFF, 0xFE, 0x00, 0x01, 0xAB, 0xCD];
    let offset = {
        use std::fs::OpenOptions;
        use std::io::{Seek, SeekFrom, Write};
        let mut file = OpenOptions::new().append(true).open(&db_path).unwrap();
        let off = file.seek(SeekFrom::End(0)).unwrap();
        let len = (garbage.len() as u32).to_le_bytes();
        file.write_all(&len).unwrap();
        file.write_all(&garbage).unwrap();
        file.sync_all().unwrap();
        off
    };

    let mut storage = StorageEngine::open(&db_path).unwrap();
    let data = storage.read_data(offset).unwrap();

    // Parsing as JSON should fail
    let parse_result: Result<serde_json::Value, _> = serde_json::from_slice(&data);
    assert!(parse_result.is_err());
}

/// Test: Read at offset beyond file
#[test]
fn test_read_beyond_file() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    let mut storage = StorageEngine::open(&db_path).unwrap();
    let file_len = storage.file_len().unwrap();

    let result = storage.read_data(file_len + 1000);
    assert!(result.is_err());

    match result {
        Err(MongoLiteError::Corruption(msg)) => {
            assert!(
                msg.contains("offset") || msg.contains("file"),
                "Should mention offset: {}",
                msg
            );
        }
        Err(_) => {}
        Ok(_) => panic!("Should fail"),
    }
}

// =============================================================================
// WAL CORRUPTION TESTS (more comprehensive)
// =============================================================================

/// Test: WAL with all invalid entry type bytes
#[test]
fn test_wal_all_invalid_entry_types() {
    let temp_dir = TempDir::new().unwrap();
    let wal_path = temp_dir.path().join("test.wal");

    // Create WAL with some entries
    {
        let mut wal = WriteAheadLog::open(&wal_path).unwrap();
        wal.append(&WALEntry::new(1, WALEntryType::Begin, vec![]))
            .unwrap();
        wal.flush().unwrap();
    }

    // Test all invalid type bytes
    let invalid_types = [0x00, 0x06, 0x10, 0x80, 0xFF];

    for invalid_type in invalid_types {
        // Corrupt entry type
        corrupt_bytes_at(&wal_path, 8, &[invalid_type]).unwrap();

        let mut wal = WriteAheadLog::open(&wal_path).unwrap();
        let result = wal.recover();

        assert!(
            result.is_err(),
            "Invalid type 0x{:02X} should fail",
            invalid_type
        );

        // Restore valid type for next iteration
        corrupt_bytes_at(&wal_path, 8, &[format::WAL_BEGIN]).unwrap();
    }
}

/// Test: WAL entry with data length mismatch
#[test]
fn test_wal_data_length_mismatch() {
    let temp_dir = TempDir::new().unwrap();
    let wal_path = temp_dir.path().join("test.wal");

    {
        let mut wal = WriteAheadLog::open(&wal_path).unwrap();
        wal.append(&WALEntry::new(
            1,
            WALEntryType::Operation,
            b"testdata".to_vec(),
        ))
        .unwrap();
        wal.flush().unwrap();
    }

    // Corrupt data length field (offset 9-12) to claim more data
    corrupt_bytes_at(&wal_path, 9, &[0xFF, 0xFF, 0x00, 0x00]).unwrap();

    let mut wal = WriteAheadLog::open(&wal_path).unwrap();
    let result = wal.recover();

    // Should either fail with error OR return empty (EOF during read is graceful)
    // The key is NO PANIC
    match result {
        Err(_) => {} // Error is expected
        Ok(recovered) => {
            // If it succeeds, should be empty (corrupted entry was skipped)
            assert!(
                recovered.is_empty(),
                "If recovery succeeds, should be empty due to corruption"
            );
        }
    }
}

/// Test: WAL completely filled with zeros
#[test]
fn test_wal_zero_filled() {
    let temp_dir = TempDir::new().unwrap();
    let wal_path = temp_dir.path().join("test.wal");

    // Create and zero out WAL
    {
        WriteAheadLog::open(&wal_path).unwrap();
        corrupt_bytes_at(&wal_path, 0, &[0u8; 100]).unwrap();
    }

    // Recovery should handle this gracefully
    let mut wal = WriteAheadLog::open(&wal_path).unwrap();
    let result = wal.recover();

    // Either empty result or error - both acceptable
    match result {
        Ok(recovered) => assert_eq!(recovered.len(), 0),
        Err(_) => {} // Error is also acceptable
    }
}

/// Test: WAL with flip of transaction_id
#[test]
fn test_wal_corrupted_transaction_id() {
    let temp_dir = TempDir::new().unwrap();
    let wal_path = temp_dir.path().join("test.wal");

    {
        let mut wal = WriteAheadLog::open(&wal_path).unwrap();
        wal.append(&WALEntry::new(1, WALEntryType::Begin, vec![]))
            .unwrap();
        wal.append(&WALEntry::new(1, WALEntryType::Commit, vec![]))
            .unwrap();
        wal.flush().unwrap();
    }

    // Corrupt transaction_id in first entry
    corrupt_bit(&wal_path, 0, 7).unwrap();

    let mut wal = WriteAheadLog::open(&wal_path).unwrap();
    let result = wal.recover();

    // CRC should catch this
    assert!(result.is_err());
}

// =============================================================================
// METADATA CORRUPTION TESTS
// =============================================================================

/// Test: Corrupted metadata offset in header
#[test]
fn test_corrupted_metadata_offset() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    {
        let mut storage = StorageEngine::open(&db_path).unwrap();
        storage.create_collection("test").unwrap();
        storage.flush().unwrap();
    }

    // Get file length
    let file_len = file_len(&db_path).unwrap();

    // Corrupt metadata_offset to point beyond file
    // metadata_offset is at a specific offset in the bincode-serialized header
    // This is tricky because bincode format varies, so we'll corrupt the end of header area
    let corrupt_offset = 100; // Somewhere in header
    let huge_offset = (file_len + 10000u64).to_le_bytes();
    corrupt_bytes_at(&db_path, corrupt_offset, &huge_offset).unwrap();

    // Open might fail or might initialize a new database
    let result = StorageEngine::open(&db_path);
    // Either is acceptable - key is no panic
    assert!(result.is_ok() || result.is_err());
}

/// Test: Corrupted collection count
#[test]
fn test_corrupted_collection_count() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    {
        let mut storage = StorageEngine::open(&db_path).unwrap();
        storage.create_collection("test").unwrap();
        storage.flush().unwrap();
    }

    // The collection_count is in the header - corrupt it
    // In bincode, after magic(8) + version(4) + page_size(4) = offset 16
    corrupt_bytes_at(&db_path, 16, &[0xFF, 0xFF, 0xFF, 0x7F]).unwrap(); // ~2 billion collections

    let result = StorageEngine::open(&db_path);
    // Either fails or handles gracefully
    assert!(result.is_ok() || result.is_err());
}

// =============================================================================
// STRESS CORRUPTION TESTS
// =============================================================================

/// Test: Random bit flips throughout file
#[test]
fn test_random_bit_flips() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // Create database with data
    {
        let mut storage = StorageEngine::open(&db_path).unwrap();
        storage.create_collection("stress").unwrap();

        for i in 1..=10 {
            let doc = json!({"_id": i, "data": format!("document_{}", i)});
            let doc_bytes = serde_json::to_vec(&doc).unwrap();
            storage
                .write_document(
                    "stress",
                    &ironbase_core::document::DocumentId::Int(i),
                    &doc_bytes,
                )
                .unwrap();
        }
        storage.flush().unwrap();
    }

    let file_len = file_len(&db_path).unwrap();

    // Flip bits at various locations
    let flip_offsets = [
        50,             // Header area
        300,            // Near start of data
        file_len / 2,   // Middle
        file_len - 100, // Near end
    ];

    for &offset in &flip_offsets {
        if offset < file_len {
            // Make a backup first
            let original = read_bytes_at(&db_path, offset, 1).unwrap();

            // Flip a bit
            corrupt_bit(&db_path, offset, 3).unwrap();

            // Try to open - should not panic
            let result = StorageEngine::open(&db_path);
            assert!(
                result.is_ok() || result.is_err(),
                "Offset {} caused panic",
                offset
            );

            // Restore original
            corrupt_bytes_at(&db_path, offset, &original).unwrap();
        }
    }
}

/// Test: Verify no panics with any single-byte corruption in header
#[test]
fn test_header_corruption_sweep() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // Create database
    {
        let mut storage = StorageEngine::open(&db_path).unwrap();
        storage.create_collection("test").unwrap();
        storage.flush().unwrap();
    }

    // Test corruption at each header position
    for offset in (0..format::HEADER_SIZE).step_by(16) {
        // Make backup
        let original = read_bytes_at(&db_path, offset, 1).unwrap_or(vec![0]);

        // Corrupt
        corrupt_bytes_at(&db_path, offset, &[0xAB]).unwrap();

        // Try to open - verify no panic
        let result = StorageEngine::open(&db_path);
        let _ = result; // We don't care if it succeeds or fails, just no panic

        // Restore
        corrupt_bytes_at(&db_path, offset, &original).unwrap();
    }
}

// =============================================================================
// ERROR MESSAGE QUALITY TESTS
// =============================================================================

/// Test: Verify corruption errors include helpful context
#[test]
fn test_error_messages_have_context() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    {
        let mut storage = StorageEngine::open(&db_path).unwrap();
        storage.create_collection("test").unwrap();
        storage.flush().unwrap();
    }

    // Corrupt magic
    corrupt_bytes_at(&db_path, 0, b"CORRUPT!").unwrap();

    let result = StorageEngine::open(&db_path);

    if let Err(e) = result {
        let error_msg = format!("{:?}", e);
        // Error should have SOME useful information
        assert!(
            error_msg.len() > 10,
            "Error message too short: {}",
            error_msg
        );
    }
}

// =============================================================================
// GRACEFUL DEGRADATION TESTS
// =============================================================================

/// Test: Compaction handles corrupted documents
#[test]
fn test_compaction_with_corruption() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // Create database with documents
    {
        let mut storage = StorageEngine::open(&db_path).unwrap();
        storage.create_collection("compact_test").unwrap();

        for i in 1..=5 {
            let doc = json!({"_id": i, "value": i});
            let doc_bytes = serde_json::to_vec(&doc).unwrap();
            storage
                .write_document(
                    "compact_test",
                    &ironbase_core::document::DocumentId::Int(i),
                    &doc_bytes,
                )
                .unwrap();
        }
        storage.flush().unwrap();
    }

    // Add garbage at end (simulating partial write)
    append_garbage(&db_path, &[0xFF; 50]).unwrap();

    // Reopen should still work (garbage is orphaned, not in catalog)
    let storage = StorageEngine::open(&db_path).unwrap();
    let meta = storage.get_collection_meta("compact_test").unwrap();

    // All 5 documents should still be tracked
    assert_eq!(meta.document_count, 5);
}

/// Test: Database remains usable after encountering corrupt document
#[test]
fn test_database_usable_after_corruption_encounter() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // Create database
    {
        let mut storage = StorageEngine::open(&db_path).unwrap();
        storage.create_collection("test").unwrap();

        let doc = json!({"_id": 1, "status": "good"});
        let doc_bytes = serde_json::to_vec(&doc).unwrap();
        storage
            .write_document(
                "test",
                &ironbase_core::document::DocumentId::Int(1),
                &doc_bytes,
            )
            .unwrap();

        storage.flush().unwrap();
    }

    // Try to read at invalid offset (should fail gracefully)
    {
        let mut storage = StorageEngine::open(&db_path).unwrap();

        // This should fail
        let _ = storage.read_data(999999);

        // But storage should still be usable for valid operations
        let collections = storage.list_collections();
        assert!(collections.contains(&"test".to_string()));
    }
}
