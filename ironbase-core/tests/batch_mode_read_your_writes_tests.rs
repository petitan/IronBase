// batch_mode_read_your_writes_tests.rs
// DurabilityMode::Batch (audit 2026-10-08):
// - every read sees the inserts acknowledged before it, through the database
//   API and through a kept CollectionCore handle;
// - a duplicate `_id` / unique key of a buffered insert is rejected at once,
//   never acknowledged and later lost with the rest of the batch;
// - acknowledged inserts survive close + reopen.

use ironbase_core::durability::DurabilityMode;
use ironbase_core::storage::StorageEngine;
use ironbase_core::DatabaseCore;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;

fn f(d: Value) -> HashMap<String, Value> {
    d.as_object().unwrap().clone().into_iter().collect()
}

fn open(path: &Path, batch_size: usize) -> DatabaseCore<StorageEngine> {
    DatabaseCore::<StorageEngine>::open_with_durability(path, DurabilityMode::Batch { batch_size })
        .unwrap()
}

fn ids(docs: &[Value]) -> Vec<i64> {
    let mut v: Vec<i64> = docs.iter().map(|d| d["_id"].as_i64().unwrap()).collect();
    v.sort();
    v
}

#[test]
fn reads_see_buffered_inserts() {
    let dir = tempfile::tempdir().unwrap();
    let db = open(&dir.path().join("r.mlite"), 100);
    db.insert_one("c", f(json!({"_id": 1, "a": 1}))).unwrap();
    let kept = db.collection("c").unwrap();

    db.insert_one("c", f(json!({"_id": 2, "a": 1}))).unwrap();
    assert_eq!(ids(&db.find("c", &json!({})).unwrap()), vec![1, 2]);
    assert_eq!(db.count_documents("c", &json!({})).unwrap(), 2);
    assert!(db.find_one("c", &json!({"_id": 2})).unwrap().is_some());

    db.insert_many(
        "c",
        vec![f(json!({"_id": 3, "a": 2})), f(json!({"_id": 4, "a": 2}))],
    )
    .unwrap();
    // the kept handle (as the Python binding keeps one)
    assert_eq!(ids(&kept.find(&json!({})).unwrap()), vec![1, 2, 3, 4]);
    assert_eq!(kept.count_documents(&json!({"a": 2})).unwrap(), 2);
    assert_eq!(kept.distinct("a", &json!({})).unwrap().len(), 2);
    let agg = kept
        .aggregate(&json!([{"$group": {"_id": null, "n": {"$sum": 1}}}]))
        .unwrap();
    assert_eq!(agg[0]["n"], json!(4));

    // a read of one collection leaves the other collection's inserts intact
    db.insert_one("d", f(json!({"_id": 1}))).unwrap();
    db.insert_one("c", f(json!({"_id": 5, "a": 3}))).unwrap();
    assert_eq!(db.count_documents("c", &json!({})).unwrap(), 5);
    assert_eq!(db.count_documents("d", &json!({})).unwrap(), 1);
}

#[test]
fn duplicate_id_of_a_buffered_insert_is_rejected_and_nothing_is_lost() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("d.mlite");
    {
        let db = open(&path, 100);
        db.insert_one("c", f(json!({"_id": 1, "a": 1}))).unwrap();
        db.insert_one("c", f(json!({"_id": 2, "a": 2}))).unwrap();
        assert!(db.insert_one("c", f(json!({"_id": 1, "a": 9}))).is_err());
        // insert_many: rejected as a whole, nothing of it tracked
        assert!(db
            .insert_many("c", vec![f(json!({"_id": 3})), f(json!({"_id": 2}))])
            .is_err());
        assert!(db
            .insert_many("c", vec![f(json!({"_id": 4})), f(json!({"_id": 4}))])
            .is_err());
        db.insert_one("c", f(json!({"_id": 3, "a": 3}))).unwrap();
        // an unrelated write flushes the batch without an error
        db.update_one("c", &json!({"_id": 99}), &json!({"$set": {"x": 1}}))
            .unwrap();
        db.close().unwrap();
    }
    let db = open(&path, 100);
    let docs = db.find("c", &json!({})).unwrap();
    assert_eq!(ids(&docs), vec![1, 2, 3]);
    assert_eq!(
        docs.iter().find(|d| d["_id"] == json!(1)).unwrap()["a"],
        json!(1)
    );
}

#[test]
fn duplicate_unique_key_of_a_buffered_insert_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("u.mlite");
    {
        let db = open(&path, 100);
        db.collection("u")
            .unwrap()
            .create_index("k".to_string(), true, false)
            .unwrap();
        db.insert_one("u", f(json!({"_id": 10, "k": "x"}))).unwrap();
        assert!(db.insert_one("u", f(json!({"_id": 11, "k": "x"}))).is_err());
        db.insert_one("u", f(json!({"_id": 12, "k": "y"}))).unwrap();
        db.close().unwrap();
    }
    let db = open(&path, 100);
    assert_eq!(ids(&db.find("u", &json!({})).unwrap()), vec![10, 12]);
}

/// A unique index created while inserts are buffered covers them.
#[test]
fn unique_index_created_over_buffered_inserts() {
    let dir = tempfile::tempdir().unwrap();
    let db = open(&dir.path().join("i.mlite"), 100);
    db.insert_one("u", f(json!({"_id": 1, "k": "x"}))).unwrap();
    db.collection("u")
        .unwrap()
        .create_index("k".to_string(), true, false)
        .unwrap();
    assert!(db.insert_one("u", f(json!({"_id": 2, "k": "x"}))).is_err());
    assert_eq!(db.count_documents("u", &json!({"k": "x"})).unwrap(), 1);
}

/// insert_many larger than the batch size: every document is WAL-committed
/// before it is persisted and all survive a reopen.
#[test]
fn insert_many_over_batch_size_survives_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("m.mlite");
    {
        let db = open(&path, 2);
        let docs: Vec<_> = (0..5).map(|i| f(json!({"_id": i}))).collect();
        db.insert_many("c", docs).unwrap();
        db.insert_one("c", f(json!({"_id": 5}))).unwrap();
        db.close().unwrap();
    }
    let db = open(&path, 2);
    assert_eq!(
        ids(&db.find("c", &json!({})).unwrap()),
        vec![0, 1, 2, 3, 4, 5]
    );
}

/// A transaction flushes the batch when it takes the write lock, so its
/// reads see the acknowledged inserts without waiting on its own lock.
#[test]
fn transaction_sees_buffered_inserts() {
    let dir = tempfile::tempdir().unwrap();
    let db = open(&dir.path().join("t.mlite"), 100);
    db.insert_one("c", f(json!({"_id": 1}))).unwrap();
    let tx = db.begin_transaction();
    db.insert_one_tx("c", f(json!({"_id": 2})), tx).unwrap();
    assert_eq!(ids(&db.find("c", &json!({})).unwrap()), vec![1]);
    db.commit_transaction(tx).unwrap();
    assert_eq!(ids(&db.find("c", &json!({})).unwrap()), vec![1, 2]);
}

/// Acknowledged inserts survive a plain drop (no close) and a checkpoint.
#[test]
fn buffered_inserts_survive_drop_and_checkpoint() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.mlite");
    {
        let db = open(&path, 100);
        db.insert_one("c", f(json!({"_id": 1}))).unwrap();
        db.checkpoint().unwrap();
        db.insert_one("c", f(json!({"_id": 2}))).unwrap();
    }
    let db = open(&path, 100);
    assert_eq!(ids(&db.find("c", &json!({})).unwrap()), vec![1, 2]);
}
