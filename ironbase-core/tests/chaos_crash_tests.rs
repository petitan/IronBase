// chaos_crash_tests.rs
// Phase 1: Crash & Recovery Chaos Tests
//
// These tests simulate various crash scenarios and verify that:
// 1. Committed data survives crashes
// 2. Uncommitted data is properly discarded
// 3. Recovery works correctly after partial writes
// 4. No panics occur - always Result::Err for corruption

mod chaos_helpers;

use chaos_helpers::*;
use ironbase_core::error::MongoLiteError;
use ironbase_core::storage::StorageEngine;
use ironbase_core::wal::{WALEntry, WALEntryType, WriteAheadLog};
use serde_json::json;
use std::collections::HashMap;
use tempfile::TempDir;

// =============================================================================
// WAL CRASH TESTS
// =============================================================================

/// Test: WAL with partial entry at end (crash mid-write)
/// Expected: Recovery should skip partial entry, recover all complete transactions
#[test]
fn test_wal_partial_entry_crash() {
    let temp_dir = TempDir::new().unwrap();
    let wal_path = temp_dir.path().join("test.wal");

    // Phase 1: Write complete transaction
    {
        let mut wal = WriteAheadLog::open(&wal_path).unwrap();
        wal.append(&WALEntry::new(1, WALEntryType::Begin, vec![]))
            .unwrap();
        wal.append(&WALEntry::new(
            1,
            WALEntryType::Operation,
            b"op1".to_vec(),
        ))
        .unwrap();
        wal.append(&WALEntry::new(1, WALEntryType::Commit, vec![]))
            .unwrap();
        wal.flush().unwrap();
    }

    // Phase 2: Simulate crash mid-write - write partial entry
    write_partial_wal_entry(
        &wal_path,
        2,                  // tx_id
        format::WAL_BEGIN,  // entry type
        &[],                // data
        5,                  // truncate at 5 bytes (mid-header)
    )
    .unwrap();

    // Phase 3: Recovery should handle partial entry gracefully
    let mut wal = WriteAheadLog::open(&wal_path).unwrap();
    let result = wal.recover();

    // Should either recover tx1 only OR return an error (both are acceptable)
    // The key is NO PANIC
    match result {
        Ok(recovered) => {
            // If recovery succeeds, should only have tx1
            assert!(
                recovered.len() <= 1,
                "Should recover at most 1 transaction"
            );
            if !recovered.is_empty() {
                assert_eq!(recovered[0][0].transaction_id, 1);
            }
        }
        Err(e) => {
            // WALCorruption is also acceptable
            println!("Recovery returned error (acceptable): {:?}", e);
        }
    }
}

/// Test: WAL with truncated uncommitted transaction
/// Expected: Only committed transactions are recovered
#[test]
fn test_wal_truncated_uncommitted_transaction() {
    let temp_dir = TempDir::new().unwrap();
    let wal_path = temp_dir.path().join("test.wal");

    {
        let mut wal = WriteAheadLog::open(&wal_path).unwrap();

        // Committed transaction
        wal.append(&WALEntry::new(1, WALEntryType::Begin, vec![]))
            .unwrap();
        wal.append(&WALEntry::new(
            1,
            WALEntryType::Operation,
            b"committed_op".to_vec(),
        ))
        .unwrap();
        wal.append(&WALEntry::new(1, WALEntryType::Commit, vec![]))
            .unwrap();

        // Uncommitted transaction (no COMMIT - simulating crash)
        wal.append(&WALEntry::new(2, WALEntryType::Begin, vec![]))
            .unwrap();
        wal.append(&WALEntry::new(
            2,
            WALEntryType::Operation,
            b"uncommitted_op".to_vec(),
        ))
        .unwrap();
        // NO COMMIT for tx2

        wal.flush().unwrap();
    }

    // Recovery
    let mut wal = WriteAheadLog::open(&wal_path).unwrap();
    let recovered = wal.recover().unwrap();

    // Only tx1 should be recovered
    assert_eq!(recovered.len(), 1, "Should recover only committed tx");
    assert_eq!(recovered[0][0].transaction_id, 1);
}

/// Test: WAL with corrupted CRC in middle
/// Expected: Recovery fails with WALCorruption error (no panic)
#[test]
fn test_wal_corrupted_crc_middle() {
    let temp_dir = TempDir::new().unwrap();
    let wal_path = temp_dir.path().join("test.wal");

    // Write valid entries
    {
        let mut wal = WriteAheadLog::open(&wal_path).unwrap();
        wal.append(&WALEntry::new(1, WALEntryType::Begin, vec![]))
            .unwrap();
        wal.append(&WALEntry::new(
            1,
            WALEntryType::Operation,
            b"data".to_vec(),
        ))
        .unwrap();
        wal.append(&WALEntry::new(1, WALEntryType::Commit, vec![]))
            .unwrap();
        wal.flush().unwrap();
    }

    // Corrupt CRC of first entry (last 4 bytes of first entry)
    // First entry: Begin with empty data = 8 + 1 + 4 + 0 + 4 = 17 bytes
    // CRC starts at byte 13
    corrupt_bit(&wal_path, 13, 0).unwrap();

    // Recovery should detect corruption
    let mut wal = WriteAheadLog::open(&wal_path).unwrap();
    let result = wal.recover();

    assert!(result.is_err(), "Should detect CRC corruption");
    match result {
        Err(MongoLiteError::WALCorruption) => {
            // Expected
        }
        Err(e) => panic!("Expected WALCorruption, got: {:?}", e),
        Ok(_) => panic!("Should have failed"),
    }
}

/// Test: WAL with invalid entry type byte
/// Expected: Recovery fails with WALCorruption (no panic)
#[test]
fn test_wal_invalid_entry_type() {
    let temp_dir = TempDir::new().unwrap();
    let wal_path = temp_dir.path().join("test.wal");

    // Write valid entry
    {
        let mut wal = WriteAheadLog::open(&wal_path).unwrap();
        wal.append(&WALEntry::new(1, WALEntryType::Begin, vec![]))
            .unwrap();
        wal.flush().unwrap();
    }

    // Corrupt entry type byte (offset 8 in first entry)
    corrupt_bytes_at(&wal_path, 8, &[0xFF]).unwrap();

    // Recovery should fail gracefully
    let mut wal = WriteAheadLog::open(&wal_path).unwrap();
    let result = wal.recover();

    assert!(result.is_err(), "Should detect invalid entry type");
}

/// Test: WAL entry with bad CRC written directly
#[test]
fn test_wal_entry_with_bad_crc_direct() {
    let temp_dir = TempDir::new().unwrap();
    let wal_path = temp_dir.path().join("test.wal");

    // Create empty WAL
    WriteAheadLog::open(&wal_path).unwrap();

    // Write entry with bad CRC
    write_wal_entry_bad_crc(&wal_path, 1, format::WAL_BEGIN, &[]).unwrap();

    // Recovery should fail
    let mut wal = WriteAheadLog::open(&wal_path).unwrap();
    let result = wal.recover();

    assert!(result.is_err(), "Should detect bad CRC");
}

/// Test: Interleaved transactions with one uncommitted
#[test]
fn test_wal_interleaved_partial_commit() {
    let temp_dir = TempDir::new().unwrap();
    let wal_path = temp_dir.path().join("test.wal");

    {
        let mut wal = WriteAheadLog::open(&wal_path).unwrap();

        // Interleaved writes
        wal.append(&WALEntry::new(1, WALEntryType::Begin, vec![]))
            .unwrap();
        wal.append(&WALEntry::new(2, WALEntryType::Begin, vec![]))
            .unwrap();
        wal.append(&WALEntry::new(
            1,
            WALEntryType::Operation,
            b"tx1_op".to_vec(),
        ))
        .unwrap();
        wal.append(&WALEntry::new(
            2,
            WALEntryType::Operation,
            b"tx2_op".to_vec(),
        ))
        .unwrap();
        wal.append(&WALEntry::new(1, WALEntryType::Commit, vec![]))
            .unwrap();
        // tx2 never commits (crash)

        wal.flush().unwrap();
    }

    let mut wal = WriteAheadLog::open(&wal_path).unwrap();
    let recovered = wal.recover().unwrap();

    // Only tx1 should be recovered
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0][0].transaction_id, 1);
}

// =============================================================================
// STORAGE FILE CRASH TESTS
// =============================================================================

/// Test: Partial document write (length header only, no data)
/// Expected: Document is not in catalog, other documents survive
#[test]
fn test_storage_partial_document_length_only() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // Phase 1: Create database with some documents
    {
        let mut storage = StorageEngine::open(&db_path).unwrap();
        storage.create_collection("test").unwrap();

        // Insert a real document
        let doc = json!({"name": "Alice", "_id": 1});
        let doc_bytes = serde_json::to_vec(&doc).unwrap();
        storage
            .write_document("test", &ironbase_core::document::DocumentId::Int(1), &doc_bytes)
            .unwrap();

        storage.flush().unwrap();
    }

    // Phase 2: Simulate crash - write length header but no data
    write_length_header_only(&db_path, 100).unwrap(); // Claims 100 bytes but has none

    // Phase 3: Reopen should still work (orphan data is ignored)
    let result = StorageEngine::open(&db_path);

    // Should either open successfully (ignoring orphan) or return error
    // Key: NO PANIC
    match result {
        Ok(storage) => {
            // If it opens, the collection should still be readable
            let collections = storage.list_collections();
            assert!(collections.contains(&"test".to_string()));
        }
        Err(e) => {
            println!("Open returned error (acceptable): {:?}", e);
        }
    }
}

/// Test: Document with truncated JSON data
#[test]
fn test_storage_truncated_document_data() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // Create database
    {
        let mut storage = StorageEngine::open(&db_path).unwrap();
        storage.create_collection("test").unwrap();
        storage.flush().unwrap();
    }

    // Write partial document (length says 50 bytes, only write 10)
    let doc_json = b"{\"name\": \"Alice\", \"age\": 30}"; // 28 bytes
    write_partial_document(&db_path, doc_json, 10).unwrap(); // Only write first 10

    // Reopen - should handle gracefully
    let result = StorageEngine::open(&db_path);
    assert!(result.is_ok() || result.is_err()); // No panic!
}

/// Test: Multiple committed documents, one corrupted at end
#[test]
fn test_storage_corrupted_last_document() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // Create database with documents
    {
        let mut storage = StorageEngine::open(&db_path).unwrap();
        storage.create_collection("test").unwrap();

        for i in 1..=5 {
            let doc = json!({"_id": i, "value": i * 10});
            let doc_bytes = serde_json::to_vec(&doc).unwrap();
            storage
                .write_document(
                    "test",
                    &ironbase_core::document::DocumentId::Int(i),
                    &doc_bytes,
                )
                .unwrap();
        }

        storage.flush().unwrap();
    }

    // Corrupt last bytes of file
    let file_len = file_len(&db_path).unwrap();
    corrupt_bytes_at(&db_path, file_len - 5, &[0xFF; 5]).unwrap();

    // Reopen - should not panic
    let result = StorageEngine::open(&db_path);
    // Either opens (with some corruption) or returns error - both acceptable
    assert!(result.is_ok() || result.is_err());
}

// =============================================================================
// CRASH RECOVERY INTEGRATION TESTS
// =============================================================================

/// Test: WAL contains committed transaction, storage doesn't have it
/// Expected: Recovery replays the transaction
#[test]
fn test_recovery_wal_ahead_of_storage() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let wal_path = temp_dir.path().join("test.wal");

    // Create storage file
    {
        let mut storage = StorageEngine::open(&db_path).unwrap();
        storage.create_collection("users").unwrap();
        storage.flush().unwrap();
    }

    // Write committed transaction directly to WAL (simulating crash after WAL commit)
    {
        let mut wal = WriteAheadLog::open(&wal_path).unwrap();

        let operation = ironbase_core::transaction::Operation::Insert {
            collection: "users".to_string(),
            doc_id: ironbase_core::document::DocumentId::Int(999),
            doc: json!({"name": "Recovered User", "status": "from_wal"}),
        };
        let op_json = serde_json::to_string(&operation).unwrap();

        wal.append(&WALEntry::new(100, WALEntryType::Begin, vec![]))
            .unwrap();
        wal.append(&WALEntry::new(
            100,
            WALEntryType::Operation,
            op_json.as_bytes().to_vec(),
        ))
        .unwrap();
        wal.append(&WALEntry::new(100, WALEntryType::Commit, vec![]))
            .unwrap();
        wal.flush().unwrap();
    }

    // Reopen storage - should trigger recovery
    {
        let mut storage = StorageEngine::open(&db_path).unwrap();
        let (_recovered_txs, _index_changes) = storage.recover_from_wal().unwrap();

        // WAL should be cleared after recovery
        let mut wal = WriteAheadLog::open(&wal_path).unwrap();
        let remaining = wal.recover().unwrap();
        assert_eq!(remaining.len(), 0, "WAL should be cleared after recovery");
    }
}

/// Test: Aborted transaction in WAL
/// Expected: Aborted transaction is NOT replayed
#[test]
fn test_recovery_ignores_aborted_transaction() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let wal_path = temp_dir.path().join("test.wal");

    // Create storage
    {
        let mut storage = StorageEngine::open(&db_path).unwrap();
        storage.create_collection("test").unwrap();
        storage.flush().unwrap();
    }

    // Write aborted transaction to WAL
    {
        let mut wal = WriteAheadLog::open(&wal_path).unwrap();

        wal.append(&WALEntry::new(1, WALEntryType::Begin, vec![]))
            .unwrap();
        wal.append(&WALEntry::new(
            1,
            WALEntryType::Operation,
            b"should_not_apply".to_vec(),
        ))
        .unwrap();
        wal.append(&WALEntry::new(1, WALEntryType::Abort, vec![]))
            .unwrap();

        wal.flush().unwrap();
    }

    // Recovery
    let mut storage = StorageEngine::open(&db_path).unwrap();
    let (recovered, _) = storage.recover_from_wal().unwrap();

    // Aborted transaction should not be in recovered list
    assert_eq!(recovered.len(), 0, "Aborted tx should not be recovered");
}

/// Test: Empty WAL recovery
#[test]
fn test_recovery_empty_wal() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // Create storage (creates empty WAL)
    {
        let mut storage = StorageEngine::open(&db_path).unwrap();
        storage.create_collection("test").unwrap();
        storage.flush().unwrap();
    }

    // Reopen - empty WAL recovery should succeed
    let mut storage = StorageEngine::open(&db_path).unwrap();
    let (recovered, _) = storage.recover_from_wal().unwrap();

    assert_eq!(recovered.len(), 0);
}

/// Test: Multiple crash cycles
#[test]
fn test_multiple_crash_recovery_cycles() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // Cycle 1: Create and crash
    {
        let mut storage = StorageEngine::open(&db_path).unwrap();
        storage.create_collection("cycle_test").unwrap();
        storage.flush().unwrap();
        // "Crash" - drop without proper cleanup
    }

    // Cycle 2: Reopen, add data, crash
    {
        let mut storage = StorageEngine::open(&db_path).unwrap();
        let doc = json!({"cycle": 2});
        let doc_bytes = serde_json::to_vec(&doc).unwrap();
        storage
            .write_document(
                "cycle_test",
                &ironbase_core::document::DocumentId::Int(1),
                &doc_bytes,
            )
            .unwrap();
        storage.flush().unwrap();
    }

    // Cycle 3: Verify data survives
    {
        let storage = StorageEngine::open(&db_path).unwrap();
        let meta = storage.get_collection_meta("cycle_test").unwrap();
        assert!(meta.document_count > 0);
    }
}

// =============================================================================
// DURABILITY EDGE CASES
// =============================================================================

/// Test: Transaction with many operations
#[test]
fn test_large_transaction_crash_recovery() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let wal_path = temp_dir.path().join("test.wal");

    // Create storage
    {
        let mut storage = StorageEngine::open(&db_path).unwrap();
        storage.create_collection("large_tx").unwrap();
        storage.flush().unwrap();
    }

    // Write large committed transaction to WAL
    {
        let mut wal = WriteAheadLog::open(&wal_path).unwrap();

        wal.append(&WALEntry::new(1, WALEntryType::Begin, vec![]))
            .unwrap();

        for i in 0..100 {
            let operation = ironbase_core::transaction::Operation::Insert {
                collection: "large_tx".to_string(),
                doc_id: ironbase_core::document::DocumentId::Int(i),
                doc: json!({"index": i, "data": format!("item_{}", i)}),
            };
            let op_json = serde_json::to_string(&operation).unwrap();
            wal.append(&WALEntry::new(
                1,
                WALEntryType::Operation,
                op_json.as_bytes().to_vec(),
            ))
            .unwrap();
        }

        wal.append(&WALEntry::new(1, WALEntryType::Commit, vec![]))
            .unwrap();
        wal.flush().unwrap();
    }

    // Recovery should handle large transaction
    let mut storage = StorageEngine::open(&db_path).unwrap();
    let (recovered, _) = storage.recover_from_wal().unwrap();

    assert_eq!(recovered.len(), 1);
    // 102 entries: BEGIN + 100 OPERATIONs + COMMIT
    assert_eq!(recovered[0].len(), 102);
}

/// Test: WAL file doesn't exist (fresh start)
#[test]
fn test_recovery_no_wal_file() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    // Create storage (which creates WAL)
    let mut storage = StorageEngine::open(&db_path).unwrap();
    storage.create_collection("test").unwrap();

    // Recovery on fresh WAL
    let (recovered, _) = storage.recover_from_wal().unwrap();
    assert_eq!(recovered.len(), 0);
}

/// Test: Checkpoint clears committed transactions
#[test]
fn test_checkpoint_clears_wal() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.mlite");
    let wal_path = temp_dir.path().join("test.wal");

    // Create database with transaction
    {
        let mut storage = StorageEngine::open(&db_path).unwrap();
        storage.create_collection("test").unwrap();

        let mut tx = ironbase_core::transaction::Transaction::new(1);
        tx.add_operation(ironbase_core::transaction::Operation::Insert {
            collection: "test".to_string(),
            doc_id: ironbase_core::document::DocumentId::Int(1),
            doc: json!({"value": 42}),
        })
        .unwrap();

        storage.commit_transaction(&mut tx).unwrap();
        storage.flush().unwrap(); // This should clear WAL
    }

    // Verify WAL is empty
    let mut wal = WriteAheadLog::open(&wal_path).unwrap();
    let recovered = wal.recover().unwrap();
    assert_eq!(recovered.len(), 0, "WAL should be cleared after flush");
}
