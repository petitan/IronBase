// shared_query_cache_tests.rs
// The query result cache is shared by every CollectionCore handle of a
// database (audit 2026-10-07 Q6): a handle kept across writes made through
// DatabaseCore or another handle never serves a stale cached result.

use ironbase_core::durability::DurabilityMode;
use ironbase_core::storage::{MemoryStorage, StorageEngine};
use ironbase_core::DatabaseCore;
use serde_json::{json, Value};
use std::collections::HashMap;

fn fields(d: Value) -> HashMap<String, Value> {
    d.as_object().unwrap().clone().into_iter().collect()
}

fn ids(docs: &[Value]) -> Vec<i64> {
    let mut v: Vec<i64> = docs.iter().map(|d| d["_id"].as_i64().unwrap()).collect();
    v.sort();
    v
}

macro_rules! check_handle_sees_every_write {
    ($db:expr) => {{
        let db = $db;
        db.insert_one("c", fields(json!({"_id": 1, "a": 1}))).unwrap();
        // The long-lived handle (as kept by the C# cursor / bindings)
        let coll = db.collection("c").unwrap();
        let q = json!({"a": 1});
        let seen = |label: &str, expected: Vec<i64>| {
            // twice: the second read is served from the cache
            for _ in 0..2 {
                assert_eq!(ids(&coll.find(&q).unwrap()), expected, "{label}");
            }
        };
        seen("initial", vec![1]);

        db.insert_one("c", fields(json!({"_id": 2, "a": 1}))).unwrap();
        seen("insert_one", vec![1, 2]);

        db.insert_many(
            "c",
            vec![fields(json!({"_id": 3, "a": 1})), fields(json!({"_id": 4, "a": 2}))],
        )
        .unwrap();
        seen("insert_many", vec![1, 2, 3]);

        db.update_one("c", &json!({"_id": 1}), &json!({"$set": {"a": 9}}))
            .unwrap();
        seen("update_one", vec![2, 3]);

        db.update_many("c", &json!({"a": 2}), &json!({"$set": {"a": 1}}))
            .unwrap();
        seen("update_many", vec![2, 3, 4]);

        db.delete_one("c", &json!({"_id": 2})).unwrap();
        seen("delete_one", vec![3, 4]);

        db.delete_many("c", &json!({"_id": {"$in": [3]}})).unwrap();
        seen("delete_many", vec![4]);

        db.update_one("c", &json!({"_id": 4}), &json!({"$set": {"a": 0}}))
            .unwrap();
        seen("update to no match", vec![]);
        db.insert_one("c", fields(json!({"_id": 5, "a": 1}))).unwrap();
        seen("insert after empty", vec![5]);

        // Drop + recreate under the same name
        db.drop_collection("c").unwrap();
        db.insert_one("c", fields(json!({"_id": 6, "a": 1}))).unwrap();
        let fresh = db.collection("c").unwrap();
        assert_eq!(ids(&fresh.find(&q).unwrap()), vec![6], "after drop");
    }};
}

#[test]
fn memory_handle_sees_every_write() {
    check_handle_sees_every_write!(DatabaseCore::<MemoryStorage>::open_memory().unwrap());
}

#[test]
fn file_handle_sees_every_write_safe_and_unsafe() {
    // Batch mode is left out: its buffered inserts are not visible to find
    // until the batch is flushed, cache or no cache (separate issue)
    for mode in [
        DurabilityMode::Safe,
        DurabilityMode::Unsafe {
            auto_checkpoint_ops: None,
        },
    ] {
        let dir = tempfile::tempdir().unwrap();
        let db =
            DatabaseCore::<StorageEngine>::open_with_durability(dir.path().join("q.mlite"), mode)
                .unwrap();
        check_handle_sees_every_write!(db);
    }
}

#[test]
fn rename_does_not_serve_the_old_collections_results() {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    db.insert_one("old", fields(json!({"_id": 1, "a": 1})))
        .unwrap();
    db.insert_one("new_src", fields(json!({"_id": 2, "a": 1})))
        .unwrap();
    let q = json!({"a": 1});
    assert_eq!(
        ids(&db.collection("old").unwrap().find(&q).unwrap()),
        vec![1]
    );
    db.rename_collection("new_src", "renamed").unwrap();
    db.rename_collection("old", "new_src").unwrap();
    assert_eq!(
        ids(&db.collection("new_src").unwrap().find(&q).unwrap()),
        vec![1]
    );
    assert_eq!(
        ids(&db.collection("renamed").unwrap().find(&q).unwrap()),
        vec![2]
    );
}

/// A committed transaction invalidates the cache of a long-lived handle.
#[test]
fn transaction_commit_invalidates() {
    let dir = tempfile::tempdir().unwrap();
    let db = DatabaseCore::<StorageEngine>::open(dir.path().join("t.mlite")).unwrap();
    db.insert_one("c", fields(json!({"_id": 1, "a": 1})))
        .unwrap();
    let coll = db.collection("c").unwrap();
    let q = json!({"a": 1});
    assert_eq!(ids(&coll.find(&q).unwrap()), vec![1]);
    assert_eq!(ids(&coll.find(&q).unwrap()), vec![1]);
    let tx = db.begin_transaction();
    db.insert_one_tx("c", fields(json!({"_id": 2, "a": 1})), tx)
        .unwrap();
    db.commit_transaction(tx).unwrap();
    assert_eq!(ids(&coll.find(&q).unwrap()), vec![1, 2]);
}

/// A large result is not cached (bounded memory), and still correct.
#[test]
fn large_results_are_not_cached() {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    let docs: Vec<_> = (0..10_050)
        .map(|i| fields(json!({"_id": i, "a": 1})))
        .collect();
    db.insert_many("c", docs).unwrap();
    let coll = db.collection("c").unwrap();
    assert_eq!(coll.find(&json!({"a": 1})).unwrap().len(), 10_050);
    assert_eq!(coll.query_cache.stats().size, 0);
    assert_eq!(coll.find(&json!({"_id": {"$lt": 5}})).unwrap().len(), 5);
    assert_eq!(coll.query_cache.stats().size, 1);
}
