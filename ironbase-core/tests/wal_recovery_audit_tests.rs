// wal_recovery_audit_tests.rs
// Crash-recovery regressions from the WAL/recovery audit (2026-10-14).
// A crash is simulated by copying the database files (except the lock)
// while the database is still open.

use ironbase_core::storage::StorageEngine;
use ironbase_core::DatabaseCore;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn d(v: Value) -> HashMap<String, Value> {
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

fn open(p: &PathBuf) -> ironbase_core::error::Result<DatabaseCore<StorageEngine>> {
    DatabaseCore::<StorageEngine>::open(p)
}

/// `a` has an index; after a checkpoint `{k: 2}` is only in memory + WAL.
/// Then `touch` persists some metadata of another collection. After a crash
/// the indexed query on `a` must still find `{k: probe}` among `total` docs.
fn metadata_persist_keeps_wal(
    total: u64,
    probe: i64,
    touch: impl Fn(&DatabaseCore<StorageEngine>),
) {
    let t = TempDir::new().unwrap();
    let c = TempDir::new().unwrap();
    let p = t.path().join("db.mlite");
    let db = open(&p).unwrap();
    db.collection("a")
        .unwrap()
        .create_index("k".into(), false, false)
        .unwrap();
    db.insert_one("a", d(json!({"k": 1}))).unwrap();
    db.checkpoint().unwrap();
    db.insert_one("a", d(json!({"k": 2}))).unwrap();
    touch(&db);
    crash_copy(t.path(), c.path());
    let db2 = open(&c.path().join("db.mlite")).unwrap();
    let a2 = db2.collection("a").unwrap();
    assert_eq!(a2.count_documents(&json!({})).unwrap(), total);
    assert_eq!(
        a2.find(&json!({"k": probe})).unwrap().len(),
        1,
        "indexed query misses a committed document after the crash"
    );
}

/// W1: the implicit create of a new collection cleared the whole WAL.
#[test]
fn implicit_collection_create_keeps_wal() {
    metadata_persist_keeps_wal(2, 2, |db| {
        db.insert_one("b", d(json!({"z": 1}))).unwrap();
    });
}

/// W1: index creation on another collection persisted its metadata with a
/// WAL-clearing flush too.
#[test]
fn index_create_on_other_collection_keeps_wal() {
    metadata_persist_keeps_wal(3, 3, |db| {
        db.insert_one("b", d(json!({"z": 1}))).unwrap();
        db.checkpoint().unwrap();
        db.insert_one("a", d(json!({"k": 3}))).unwrap();
        db.collection("b")
            .unwrap()
            .create_index("z".into(), false, false)
            .unwrap();
    });
}

/// W2: explicit transactions also log `IndexChange` entries; replaying them
/// at open on top of the already recovered index re-inserted history: `x`
/// moved from doc 1 to doc 2 made the unique index reject the reopen.
#[test]
fn unique_index_history_does_not_block_reopen() {
    let t = TempDir::new().unwrap();
    let c = TempDir::new().unwrap();
    let p = t.path().join("db.mlite");
    let db = open(&p).unwrap();
    db.collection("u")
        .unwrap()
        .create_index("email".into(), true, false)
        .unwrap();
    db.checkpoint().unwrap();
    let tx = db.begin_transaction();
    db.insert_one_tx("u", d(json!({"_id": 1, "email": "x"})), tx)
        .unwrap();
    db.commit_transaction(tx).unwrap();
    let tx = db.begin_transaction();
    db.update_one_tx("u", &json!({"_id": 1}), json!({"$set": {"email": "y"}}), tx)
        .unwrap();
    db.commit_transaction(tx).unwrap();
    let tx = db.begin_transaction();
    db.insert_one_tx("u", d(json!({"_id": 2, "email": "x"})), tx)
        .unwrap();
    db.commit_transaction(tx).unwrap();
    crash_copy(t.path(), c.path());
    let db2 = open(&c.path().join("db.mlite")).expect("reopen after crash");
    let u = db2.collection("u").unwrap();
    let x = u.find(&json!({"email": "x"})).unwrap();
    assert_eq!(x.len(), 1);
    assert_eq!(x[0]["_id"], json!(2));
    assert_eq!(u.find(&json!({"email": "y"})).unwrap().len(), 1);
}

/// W2: a multikey index keeps no stale key of an updated array after the
/// replay.
#[test]
fn multikey_index_history_is_not_replayed() {
    let t = TempDir::new().unwrap();
    let c = TempDir::new().unwrap();
    let p = t.path().join("db.mlite");
    let db = open(&p).unwrap();
    db.collection("u")
        .unwrap()
        .create_index("tags".into(), false, false)
        .unwrap();
    db.checkpoint().unwrap();
    let tx = db.begin_transaction();
    db.insert_one_tx("u", d(json!({"_id": 1, "tags": ["a", "b"]})), tx)
        .unwrap();
    db.commit_transaction(tx).unwrap();
    let tx = db.begin_transaction();
    db.update_one_tx(
        "u",
        &json!({"_id": 1}),
        json!({"$set": {"tags": ["c"]}}),
        tx,
    )
    .unwrap();
    db.commit_transaction(tx).unwrap();
    crash_copy(t.path(), c.path());
    let db2 = open(&c.path().join("db.mlite")).unwrap();
    let u = db2.collection("u").unwrap();
    assert_eq!(u.count_documents(&json!({"tags": "a"})).unwrap(), 0);
    assert_eq!(u.find(&json!({"tags": "c"})).unwrap().len(), 1);
}

/// W3: recovering corrupted metadata from the WAL snapshot cleared the WAL,
/// dropping the committed inserts logged after the snapshot.
#[test]
fn metadata_recovery_keeps_committed_wal() {
    use std::io::{Read, Seek, SeekFrom, Write};
    let t = TempDir::new().unwrap();
    let c = TempDir::new().unwrap();
    let p = t.path().join("db.mlite");
    let db = open(&p).unwrap();
    db.collection("a").unwrap();
    for i in 0..5 {
        db.insert_one("a", d(json!({"k": i}))).unwrap();
    }
    crash_copy(t.path(), c.path());
    // Corrupt the metadata length field (a "recoverable" Corruption error)
    let cp = c.path().join("db.mlite");
    let mut f = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&cp)
        .unwrap();
    let mut hdr = [0u8; 64];
    f.read_exact(&mut hdr).unwrap();
    let meta_off = u64::from_le_bytes(hdr[36..44].try_into().unwrap());
    f.seek(SeekFrom::Start(meta_off + 4)).unwrap();
    f.write_all(&0xFFFF_FFF0u32.to_le_bytes()).unwrap();
    drop(f);
    let db2 = open(&cp).unwrap();
    let a = db2.collection("a").unwrap();
    assert_eq!(
        a.find(&json!({})).unwrap().len(),
        5,
        "committed (fsynced WAL) inserts lost"
    );
}
