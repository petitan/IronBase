// Storage compaction tests using public DatabaseCore API
use ironbase_core::{DatabaseCore, StorageEngine};
use serde_json::json;
use std::collections::HashMap;
use tempfile::TempDir;

#[test]
fn test_compaction_removes_tombstones() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("compact_test.mlite");

    let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();

    // Insert 10 documents
    for i in 0..10 {
        let mut doc = HashMap::new();
        doc.insert("id".to_string(), json!(i));
        doc.insert("name".to_string(), json!(format!("User{}", i)));
        db.insert_one("users", doc).unwrap();
    }

    // Delete half (creates tombstones)
    for i in 0..5i64 {
        db.delete_one("users", &json!({"id": i})).unwrap();
    }

    // Compact
    let stats = db.compact().unwrap();

    // Verify stats
    assert_eq!(stats.tombstones_removed, 5);
    assert!(stats.space_saved() > 0);
    assert!(stats.size_after < stats.size_before);
}

#[test]
fn test_compaction_preserves_live_documents() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("compact_preserve.mlite");

    let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();

    // Insert documents
    for i in 0..20 {
        let mut doc = HashMap::new();
        doc.insert("value".to_string(), json!(i * 100));
        db.insert_one("items", doc).unwrap();
    }

    // Compact
    let stats = db.compact().unwrap();

    // All documents should be kept (no tombstones)
    assert_eq!(stats.documents_kept, 20);
    assert_eq!(stats.tombstones_removed, 0);

    // Verify all documents still exist via query
    let coll = db.collection("items").unwrap();
    let docs = coll.find(&json!({})).unwrap();
    assert_eq!(docs.len(), 20);
}

#[test]
fn test_compaction_multi_collection() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("compact_multi.mlite");

    let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();

    // Insert to users collection
    for i in 0..10 {
        let mut doc = HashMap::new();
        doc.insert("name".to_string(), json!(format!("User{}", i)));
        db.insert_one("users", doc).unwrap();
    }

    // Delete some from users (tombstones)
    for i in 0..3i64 {
        db.delete_one("users", &json!({"name": format!("User{}", i)}))
            .unwrap();
    }

    // Insert to posts collection
    for i in 0..10 {
        let mut doc = HashMap::new();
        doc.insert("title".to_string(), json!(format!("Post{}", i)));
        db.insert_one("posts", doc).unwrap();
    }

    // Compact
    let stats = db.compact().unwrap();

    // Should have removed tombstones
    assert_eq!(stats.tombstones_removed, 3);
    // Should keep: 7 users + 10 posts = 17
    assert_eq!(stats.documents_kept, 17);
}

#[test]
fn test_compaction_handles_updates() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("compact_updates.mlite");

    let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();

    // Insert document
    let mut doc = HashMap::new();
    doc.insert("_id".to_string(), json!(1));
    doc.insert("value".to_string(), json!(100));
    db.insert_one("data", doc).unwrap();

    // Update it 5 times (creates old versions)
    for i in 2..=6 {
        db.update_one(
            "data",
            &json!({"_id": 1}),
            &json!({"$set": {"value": i * 100}}),
        )
        .unwrap();
    }

    // Compact - should keep only latest version
    let stats = db.compact().unwrap();

    assert_eq!(stats.documents_kept, 1); // Only latest version
    assert!(stats.size_after < stats.size_before); // Size reduced

    // Verify latest value is preserved
    let coll = db.collection("data").unwrap();
    let doc = coll.find_one(&json!({"_id": 1})).unwrap().unwrap();
    assert_eq!(doc["value"], json!(600)); // Latest value
}

#[test]
fn test_compaction_stats() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("compact_stats.mlite");

    let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();

    // Insert 100 documents
    for i in 0..100 {
        let mut doc = HashMap::new();
        doc.insert("id".to_string(), json!(i));
        doc.insert("data".to_string(), json!(vec![0u8; 100]));
        db.insert_one("test", doc).unwrap();
    }

    // Mark 50 as tombstones (delete)
    for i in 0..50i64 {
        db.delete_one("test", &json!({"id": i})).unwrap();
    }

    // Compact
    let stats = db.compact().unwrap();

    // Verify stats
    assert!(stats.size_before > 0);
    assert!(stats.size_after > 0);
    assert!(stats.size_after < stats.size_before);
    assert_eq!(stats.tombstones_removed, 50);
    assert_eq!(stats.documents_kept, 50);
    assert!(stats.space_saved() > 0);
    assert!(stats.compression_ratio() > 0.0);
    assert!(stats.compression_ratio() < 100.0);
}

#[test]
fn test_compaction_persistence() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("compact_persist.mlite");

    {
        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();

        // Insert documents
        for i in 0..10 {
            let mut doc = HashMap::new();
            doc.insert("id".to_string(), json!(i));
            db.insert_one("items", doc).unwrap();
        }

        // Mark half as deleted
        for i in 0..5i64 {
            db.delete_one("items", &json!({"id": i})).unwrap();
        }

        db.compact().unwrap();
        db.flush().unwrap();
    }

    // Reopen and verify compacted state persisted
    {
        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        let coll = db.collection("items").unwrap();

        // Should only have 5 documents (tombstones removed)
        let docs = coll.find(&json!({})).unwrap();
        assert_eq!(docs.len(), 5);
    }
}

/// Test that compaction correctly updates data_end_offset to prevent sparse hole recreation
/// This was a critical bug: after compaction, Drop::flush() would write at the old offset,
/// recreating the sparse hole that compaction just eliminated.
#[test]
fn test_compaction_no_sparse_hole_on_drop() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("compact_sparse.mlite");

    let size_after_compact: u64;

    // Phase 1: Create database with lots of data, delete most, compact
    {
        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();

        // Insert many large documents to create significant file size
        for i in 0..100 {
            let mut doc = HashMap::new();
            doc.insert("id".to_string(), json!(i));
            doc.insert("data".to_string(), json!("x".repeat(1000))); // 1KB padding
            db.insert_one("items", doc).unwrap();
        }

        // Delete 90% of documents
        for i in 0..90i64 {
            db.delete_one("items", &json!({"id": i})).unwrap();
        }

        // Compact to remove tombstones
        let stats = db.compact().unwrap();
        size_after_compact = stats.size_after;

        // Database is dropped here - Drop::flush() should NOT recreate sparse hole
    }

    // Phase 2: Verify file size didn't grow significantly
    let file_size = std::fs::metadata(&db_path).unwrap().len();

    // File should be approximately the size after compact (with tolerance for metadata re-write)
    // The bug would cause file_size to be >> size_after_compact (back to original size)
    // Normal behavior: Drop::flush() appends metadata (~few KB to ~8MB depending on doc count)
    let tolerance = 64 * 1024; // Allow 64KB for metadata re-write (append-only design)
    assert!(
        file_size <= size_after_compact + tolerance,
        "File grew after Drop! size_after_compact={}, file_size={}, diff={}. \
         This indicates data_end_offset was not properly updated during compaction.",
        size_after_compact,
        file_size,
        file_size.saturating_sub(size_after_compact)
    );

    // Phase 3: Verify database still works correctly
    {
        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        let coll = db.collection("items").unwrap();
        let docs = coll.find(&json!({})).unwrap();
        assert_eq!(docs.len(), 10, "Should have 10 documents after compact");
    }
}

/// P1-7: `last_compact_size` must persist across a graceful close+reopen so the
/// reopened DB keeps an accurate `bloat_ratio`. Before the fix it reset to 0 →
/// `bloat_ratio = +inf` → the auto-compact rewrote the whole file on every
/// restart. This regression guard fails on the unfixed code (estimated_live=0,
/// bloat_ratio=inf) and passes after the Header field is persisted.
#[test]
fn last_compact_size_persists_across_reopen() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("persist_compact.mlite");

    {
        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        for i in 0..50i64 {
            let mut doc = HashMap::new();
            doc.insert("v".to_string(), json!(i));
            db.insert_one("c", doc).unwrap();
        }
        // Create reclaimable bloat, then compact (sets the in-memory baseline).
        for i in 0..20i64 {
            db.delete_one("c", &json!({ "v": i })).unwrap();
        }
        let stats = db.compact().unwrap();
        assert!(stats.size_after > 0);
        // Graceful close persists last_compact_size into the Header.
        db.close().unwrap();
    }

    // Reopen: the baseline must come back from the Header, NOT reset to 0.
    let db2 = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
    let w = db2.storage_wastage();
    assert!(
        w.estimated_live_bytes > 0,
        "last_compact_size must persist across reopen (got 0 → bloat_ratio would be +inf)"
    );
    assert!(
        w.bloat_ratio.is_finite(),
        "bloat_ratio must be finite after reopen, got {}",
        w.bloat_ratio
    );
}

/// P1-3: idle checkpoints (no writes since the last flush) must NOT re-append the
/// metadata catalog. Before the fix the clean path fell through to a full
/// `storage.checkpoint()` whose `flush_metadata()` appends the catalog regardless
/// of dirtiness, so an idle DB grew on every 60s checkpoint (multi-GB/day at
/// 100GB). This guard fails on the unfixed code (file grows) and passes after the
/// clean path is routed to WAL-clear-only.
#[test]
fn idle_checkpoint_does_not_grow_file() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("idle_ckpt.mlite");
    let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
    for i in 0..30i64 {
        let mut doc = HashMap::new();
        doc.insert("v".to_string(), json!(i));
        db.insert_one("c", doc).unwrap();
    }
    // First checkpoint flushes the (dirty) metadata to disk.
    db.checkpoint_wal_only().unwrap();
    let size1 = std::fs::metadata(&db_path).unwrap().len();

    // No writes since → metadata is clean → further checkpoints must be no-ops
    // for the data file (only WAL is touched), so the file must NOT grow.
    for _ in 0..5 {
        db.checkpoint_wal_only().unwrap();
    }
    let size2 = std::fs::metadata(&db_path).unwrap().len();
    assert_eq!(
        size2, size1,
        "idle checkpoints must not re-append metadata (file grew {} → {})",
        size1, size2
    );
}

/// PR #89 follow-up, finding A (companion to the storage-level crash test
/// `committed_write_survives_crash_via_checkpoint`): exercise the two-phase
/// `checkpoint_wal_only` (including the re-check the fix added to the Phase-B
/// `None` arm) under heavy concurrency. The fix calls `storage.checkpoint()`
/// while already holding `storage.write()`; this guards against a deadlock or
/// panic on that path and confirms that under a *graceful* shutdown every
/// committed insert is present. (It does NOT prove the crash-durability fix —
/// a graceful close always flushes the in-memory catalog, healing any WAL
/// clear; the crash case is covered deterministically in the storage test.)
#[test]
fn concurrent_inserts_and_wal_checkpoints_stay_consistent() {
    use std::sync::Arc;
    use std::thread;

    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("ckpt_race.mlite");
    // Enough concurrency to race Phase A/B against committing inserts and hit the
    // None/Some arms repeatedly; kept modest so CI cost stays low (this is a
    // deadlock/consistency smoke test, not the durability guard).
    const N: i64 = 600;

    {
        let db = Arc::new(DatabaseCore::<StorageEngine>::open(&db_path).unwrap());

        let writer = {
            let db = Arc::clone(&db);
            thread::spawn(move || {
                for i in 0..N {
                    let mut doc = HashMap::new();
                    doc.insert("i".to_string(), json!(i));
                    db.insert_one("c", doc).unwrap();
                }
            })
        };

        // Race the two-phase checkpoint against the writer, so Phase A
        // frequently catches a clean state (→ None path, which now re-checks
        // is_metadata_dirty and may call checkpoint() under the write lock).
        // yield_now() between iterations avoids a CPU-burning busy-spin that
        // would also starve the writer and inflate the test's wall-clock.
        while !writer.is_finished() {
            db.checkpoint_wal_only().unwrap();
            std::thread::yield_now();
        }
        writer.join().unwrap();

        db.checkpoint_wal_only().unwrap();
        db.close().unwrap();
    }

    // Graceful shutdown → every committed insert must be present (no deadlock,
    // no panic, no double-count).
    let db2 = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
    let count = db2.count_documents("c", &json!({})).unwrap();
    assert_eq!(
        count as i64, N,
        "concurrent inserts + checkpoints lost consistency (got {} of {})",
        count, N
    );
}

/// PR #89 follow-up, finding C+D: `last_compact_size` reached the Header only via
/// `close()`. The Drop path persisted the tx_id watermark but NOT the compact-size
/// baseline, so a process that compacted and then dropped (or crashed) without an
/// explicit `close()` lost the baseline → `bloat_ratio = +inf` → the auto-compact
/// rewrote the whole file on the next start (P1-7). The fix persists the baseline
/// into the Header at compact time (under the storage write lock), which marks
/// metadata dirty so Drop's `flush()` writes it. This guard compacts and then lets
/// the DB *drop* (no `close()`), and requires the baseline to come back on reopen.
#[test]
fn last_compact_size_persists_across_drop_without_close() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("persist_compact_drop.mlite");

    let size_after_compact: u64;
    {
        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        for i in 0..50i64 {
            let mut doc = HashMap::new();
            doc.insert("v".to_string(), json!(i));
            db.insert_one("c", doc).unwrap();
        }
        for i in 0..20i64 {
            db.delete_one("c", &json!({ "v": i })).unwrap();
        }
        let stats = db.compact().unwrap();
        size_after_compact = stats.size_after;
        assert!(size_after_compact > 0);
        // NO close() — the DB drops here. Drop::flush() must persist the Header
        // because compact() marked metadata dirty via storage.set_last_compact_size.
    }

    let db2 = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
    let w = db2.storage_wastage();
    assert_eq!(
        w.estimated_live_bytes, size_after_compact,
        "last_compact_size must survive a Drop without close() (got {}, expected {})",
        w.estimated_live_bytes, size_after_compact
    );
    assert!(
        w.bloat_ratio.is_finite(),
        "bloat_ratio must be finite after reopen, got {}",
        w.bloat_ratio
    );
}

/// Fix C (2026-06-16): a DB that has live data but was NEVER compacted (legacy
/// pre-v6 file, or any never-compacted DB) persists `last_compact_size = 0`.
/// Without the seed, `storage_wastage()` returns `bloat_ratio = +inf`, so the
/// auto-compactor rewrites the whole file ~5 min after every startup — on a
/// large file that buffered whole-file I/O can freeze the host (verified: a
/// 7.1GB docs.mlite froze an 11GB WSL2 host 3×). The open path must seed the
/// baseline from the current file size when live data is present, so bloat
/// starts finite (~1.0). This guard fails on the unfixed code (bloat = +inf)
/// and passes after the seed.
#[test]
fn never_compacted_db_with_data_seeds_finite_bloat_on_open() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("never_compacted.mlite");

    {
        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        for i in 0..50i64 {
            let mut doc = HashMap::new();
            doc.insert("v".to_string(), json!(i));
            db.insert_one("c", doc).unwrap();
        }
        // NO compact() ever — graceful close persists last_compact_size = 0.
        db.close().unwrap();
    }

    let db2 = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
    let w = db2.storage_wastage();
    assert!(
        w.estimated_live_bytes > 0,
        "never-compacted DB with live data must seed a non-zero baseline (got 0 \
         → bloat_ratio would be +inf → spurious startup compaction)"
    );
    assert!(
        w.bloat_ratio.is_finite(),
        "bloat_ratio must be finite after the legacy seed, got {}",
        w.bloat_ratio
    );
}

/// Fix C corollary: an empty (or all-tombstone) DB has no live data, so the seed
/// must NOT fire — `last_compact_size` stays 0. This keeps genuine garbage
/// eligible for reclaim (the RAM-safety gate then makes that compaction safe) and
/// proves the seed does not blindly stamp every never-compacted file. Note: an
/// empty file also trips the min-file-size gate, so leaving bloat = +inf here is
/// harmless.
#[test]
fn empty_db_does_not_seed_baseline_on_open() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("empty_db.mlite");

    {
        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        db.close().unwrap();
    }

    let db2 = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
    let w = db2.storage_wastage();
    assert_eq!(
        w.estimated_live_bytes, 0,
        "empty DB must NOT seed a baseline (no live data); got {}",
        w.estimated_live_bytes
    );
}
