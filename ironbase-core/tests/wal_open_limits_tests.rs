// wal_open_limits_tests.rs
// A database whose WAL is long or ends in garbage must still open
// (WAL/recovery audit 2026-10-14 W5, W6): recovery refused a WAL with more
// than 1 000 000 entries, and an invalid record at the end of the WAL (a
// crash that left a zero-filled or half-written tail) failed the open.
// A crash is simulated by copying the database files (except the lock)
// while the database is still open.

use ironbase_core::{DatabaseCore, WALEntry, WALEntryType};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn doc(v: Value) -> HashMap<String, Value> {
    serde_json::from_value(v).unwrap()
}

fn crash_copy(src_dir: &Path, dst_dir: &Path) {
    for e in std::fs::read_dir(src_dir).unwrap() {
        let e = e.unwrap();
        let name = e.file_name().to_string_lossy().to_string();
        if !name.ends_with(".lock") {
            std::fs::copy(e.path(), dst_dir.join(&name)).unwrap();
        }
    }
}

/// A crashed copy of a database whose WAL holds 3 committed inserts into `c`.
fn crashed_db(live: &TempDir, crashed: &TempDir) -> PathBuf {
    let path = live.path().join("db.mlite");
    let db = DatabaseCore::open(&path).unwrap();
    db.insert_one("c", doc(json!({"k": 0}))).unwrap();
    db.checkpoint().unwrap();
    for k in 1..=3 {
        db.insert_one("c", doc(json!({"k": k}))).unwrap();
    }
    crash_copy(live.path(), crashed.path());
    crashed.path().join("db.mlite")
}

fn append_to_wal(db_path: &Path, bytes: &[u8]) {
    let mut wal = std::fs::OpenOptions::new()
        .append(true)
        .open(db_path.with_extension("wal"))
        .unwrap();
    wal.write_all(bytes).unwrap();
    wal.sync_all().unwrap();
}

fn count(db_path: &Path) -> u64 {
    let db = DatabaseCore::open(db_path).expect("database must open");
    db.count_documents("c", &json!({})).unwrap()
}

#[test]
fn zero_filled_wal_tail_is_ignored() {
    let (live, crashed) = (TempDir::new().unwrap(), TempDir::new().unwrap());
    let path = crashed_db(&live, &crashed);
    // The file grew but the data blocks never reached the disk
    append_to_wal(&path, &[0u8; 4096]);
    assert_eq!(count(&path), 4);
}

#[test]
fn torn_record_at_wal_tail_is_ignored() {
    let (live, crashed) = (TempDir::new().unwrap(), TempDir::new().unwrap());
    let path = crashed_db(&live, &crashed);
    // A whole record whose bytes only partly reached the disk: bad checksum
    let mut torn = WALEntry::new(1_000_000, WALEntryType::Begin, b"x".repeat(64)).serialize();
    let n = torn.len();
    torn[n - 10..].fill(0xAB);
    append_to_wal(&path, &torn);
    assert_eq!(count(&path), 4);
}

#[test]
fn corrupt_record_in_the_middle_still_fails() {
    let (live, crashed) = (TempDir::new().unwrap(), TempDir::new().unwrap());
    let path = crashed_db(&live, &crashed);
    let mut bad = WALEntry::new(1_000_000, WALEntryType::Begin, vec![]).serialize();
    let n = bad.len();
    bad[n - 1] ^= 0xFF;
    // A valid committed transaction after the damaged record: real
    // corruption, not a torn tail, so recovery must not silently stop there
    let mut bytes = bad;
    bytes.extend(WALEntry::new(1_000_001, WALEntryType::Begin, vec![]).serialize());
    bytes.extend(WALEntry::new(1_000_001, WALEntryType::Commit, vec![]).serialize());
    append_to_wal(&path, &bytes);
    assert!(DatabaseCore::open(&path).is_err());
}

#[test]
fn wal_with_over_a_million_entries_opens() {
    let (live, crashed) = (TempDir::new().unwrap(), TempDir::new().unwrap());
    let path = crashed_db(&live, &crashed);
    // 1 000 002 small entries: empty committed transactions
    let mut bytes = Vec::new();
    for tx in 0..500_001u64 {
        let id = 2_000_000 + tx;
        bytes.extend(WALEntry::new(id, WALEntryType::Begin, vec![]).serialize());
        bytes.extend(WALEntry::new(id, WALEntryType::Commit, vec![]).serialize());
    }
    append_to_wal(&path, &bytes);
    assert_eq!(count(&path), 4);
}

#[test]
fn writes_after_a_recovered_garbage_tail_survive_the_next_crash() {
    let (live, crashed) = (TempDir::new().unwrap(), TempDir::new().unwrap());
    let path = crashed_db(&live, &crashed);
    append_to_wal(&path, &[0u8; 4096]);

    // Recovery must drop the garbage: entries appended after it would
    // otherwise follow an invalid record and fail the next open
    let again = TempDir::new().unwrap();
    {
        let db = DatabaseCore::open(&path).unwrap();
        db.insert_one("c", doc(json!({"k": 4}))).unwrap();
        crash_copy(crashed.path(), again.path());
    }
    assert_eq!(count(&again.path().join("db.mlite")), 5);
}
