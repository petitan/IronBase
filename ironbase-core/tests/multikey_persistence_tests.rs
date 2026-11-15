// multikey_persistence_tests.rs
// An index made multikey by a write after its creation stays multikey across
// a reopen, a crash and an update. Loaded with the flag off, the index-based
// count counted such a document once per key: {"o": {"x": [0, -1]}} matched
// `{"o.x": {"$gt": -1.5}}` twice (found by `pbt_crash_recovery`).

use ironbase_core::{DatabaseCore, StorageEngine};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;

fn fields(doc: Value) -> HashMap<String, Value> {
    doc.as_object().unwrap().clone().into_iter().collect()
}

/// `count_documents` and `find` of `query` on collection `s`
fn count_and_find(db: &DatabaseCore<StorageEngine>, query: &Value) -> (u64, usize) {
    let coll = db.get_collection("s").unwrap();
    (
        coll.count_documents(query).unwrap(),
        coll.find(query).unwrap().len(),
    )
}

/// An index on `o.x` created on an empty collection, then a document with
/// an array there
fn db_with_array_after_index(path: &Path) -> DatabaseCore<StorageEngine> {
    let db = DatabaseCore::<StorageEngine>::open(path).unwrap();
    db.collection("s")
        .unwrap()
        .create_index("o.x".to_string(), false, false)
        .unwrap();
    db.insert_one("s", fields(json!({"o": {"x": [0, -1]}})))
        .unwrap();
    db
}

/// Copy the files of a live database, as a killed process leaves them
fn crash_copy(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_file()
            && entry.path().extension().is_none_or(|e| e != "lock")
        {
            std::fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
        }
    }
}

#[test]
fn test_multikey_index_counts_a_document_once_after_reopen() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("s.mlite");
    let query = json!({"o.x": {"$gt": -1.5}});

    let db = db_with_array_after_index(&path);
    assert_eq!(count_and_find(&db, &query), (1, 1));
    db.close().unwrap();
    drop(db);

    let db = DatabaseCore::<StorageEngine>::open(&path).unwrap();
    assert_eq!(count_and_find(&db, &query), (1, 1));
    assert_eq!(
        count_and_find(&db, &json!({"$and": [{"o.x": {"$gt": -1.5}}]})),
        (1, 1)
    );
}

#[test]
fn test_multikey_index_counts_a_document_once_after_a_crash() {
    let dir = tempfile::TempDir::new().unwrap();
    let live = dir.path().join("live");
    std::fs::create_dir(&live).unwrap();
    let query = json!({"o.x": {"$gt": -1.5}});

    let db = db_with_array_after_index(&live.join("s.mlite"));
    // A checkpoint writes the index file: the crash after it loads that file
    db.checkpoint().unwrap();
    db.insert_one("s", fields(json!({"o": {"x": [5, 6]}})))
        .unwrap();
    let crashed = dir.path().join("crashed");
    crash_copy(&live, &crashed);
    drop(db);

    let db = DatabaseCore::<StorageEngine>::open(crashed.join("s.mlite")).unwrap();
    assert_eq!(count_and_find(&db, &query), (2, 2));
}

/// The first array arrives after the last checkpoint: the index file loaded
/// after the crash lacks it, and the WAL replay marks the index multikey
#[test]
fn test_wal_replay_marks_the_index_multikey() {
    let dir = tempfile::TempDir::new().unwrap();
    let live = dir.path().join("live");
    std::fs::create_dir(&live).unwrap();
    let query = json!({"o.x": {"$gt": -1.5}});

    let db = DatabaseCore::<StorageEngine>::open(live.join("s.mlite")).unwrap();
    db.collection("s")
        .unwrap()
        .create_index("o.x".to_string(), false, false)
        .unwrap();
    db.insert_one("s", fields(json!({"o": {"x": 1}}))).unwrap();
    db.checkpoint().unwrap();
    db.insert_one("s", fields(json!({"o": {"x": [0, -1]}})))
        .unwrap();
    let crashed = dir.path().join("crashed");
    crash_copy(&live, &crashed);
    drop(db);

    let db = DatabaseCore::<StorageEngine>::open(crashed.join("s.mlite")).unwrap();
    assert_eq!(count_and_find(&db, &query), (2, 2));
    db.close().unwrap();
    drop(db);
    // ... and the replayed flag is stored with the index file
    let db = DatabaseCore::<StorageEngine>::open(crashed.join("s.mlite")).unwrap();
    assert_eq!(count_and_find(&db, &query), (2, 2));
}

/// An update bringing an array makes the index multikey, before and after
/// a reopen (the batch update path did not mark it)
#[test]
fn test_update_bringing_an_array_makes_the_index_multikey() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("s.mlite");
    let query = json!({"o.x": {"$gt": -1.5}});

    let db = DatabaseCore::<StorageEngine>::open(&path).unwrap();
    db.collection("s")
        .unwrap()
        .create_index("o.x".to_string(), false, false)
        .unwrap();
    db.insert_one("s", fields(json!({"n": 1, "o": {"x": 7}})))
        .unwrap();
    db.update_one("s", &json!({"n": 1}), &json!({"$set": {"o.x": [0, -1]}}))
        .unwrap();
    assert_eq!(count_and_find(&db, &query), (1, 1));
    db.close().unwrap();
    drop(db);

    let db = DatabaseCore::<StorageEngine>::open(&path).unwrap();
    assert_eq!(count_and_find(&db, &query), (1, 1));
}
