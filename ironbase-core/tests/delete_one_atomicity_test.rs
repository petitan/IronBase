//! Regression tests for delete_one AND delete_many atomicity (audit P2-3).
//!
//! Every delete write path previously de-indexed BEFORE acquiring the storage write
//! lock (reversed lock order vs #75) and read the target document lock-free, so two
//! concurrent deletes of the same document could both pass the live read and each
//! write a tombstone + `adjust_live_count(-1)` → the live count drifted below the
//! true value. All paths now funnel through `tombstone_doc_atomic`, performing the
//! whole read-modify-write under one storage write lock (re-read + tombstone-check),
//! mirroring `update_one_prepare`.
//!
//! NOTE: counts after a mutation are read through a FRESH collection handle on
//! purpose — the per-handle QueryCache (audit P2-5) means a reused handle could
//! otherwise serve a stale count and mask the real storage state.

use ironbase_core::storage::MemoryStorage;
use ironbase_core::{DatabaseCore, DocumentId};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;

fn doc(field: &str, val: Value) -> HashMap<String, Value> {
    HashMap::from([(field.to_string(), val)])
}

/// Count live docs via a FRESH handle (avoids any per-handle query-cache staleness).
fn fresh_count(db: &DatabaseCore<MemoryStorage>) -> u64 {
    db.collection("c")
        .unwrap()
        .count_documents(&json!({}))
        .unwrap()
}

#[test]
fn test_delete_one_fast_path_id_still_works() {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    db.collection("c").unwrap();
    let id = db.insert_one("c", doc("name", json!("Alice"))).unwrap();

    let deleted = db.delete_one("c", &json!({ "_id": id })).unwrap();
    assert_eq!(deleted, 1);
    assert_eq!(fresh_count(&db), 0);
}

#[test]
fn test_delete_one_slow_path_filter_still_works() {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    db.collection("c").unwrap();
    db.insert_one("c", doc("name", json!("Alice"))).unwrap();
    db.insert_one("c", doc("name", json!("Bob"))).unwrap();

    let deleted = db.delete_one("c", &json!({ "name": "Bob" })).unwrap();
    assert_eq!(deleted, 1);
    assert_eq!(fresh_count(&db), 1);
    assert_eq!(
        db.collection("c")
            .unwrap()
            .count_documents(&json!({ "name": "Alice" }))
            .unwrap(),
        1
    );
}

#[test]
fn test_delete_one_normalized_string_id() {
    // {"_id": "123"} must resolve to DocumentId::Int(123) under the lock.
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    db.collection("c").unwrap();
    let id = db.insert_one("c", doc("name", json!("Alice"))).unwrap();
    let n = match id {
        DocumentId::Int(n) => n,
        other => panic!("expected Int auto-id, got {:?}", other),
    };

    let deleted = db
        .delete_one("c", &json!({ "_id": n.to_string() }))
        .unwrap();
    assert_eq!(deleted, 1, "string _id must normalize to the stored Int id");
    assert_eq!(fresh_count(&db), 0);
}

#[test]
fn test_delete_one_not_found_returns_zero() {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    db.collection("c").unwrap();
    db.insert_one("c", doc("name", json!("setup"))).unwrap();

    assert_eq!(db.delete_one("c", &json!({ "_id": 999_999 })).unwrap(), 0);
    assert_eq!(db.delete_one("c", &json!({ "name": "nope" })).unwrap(), 0);
    assert_eq!(fresh_count(&db), 1);
}

#[test]
fn test_delete_one_sequential_second_is_idempotent() {
    // The second delete of an already-deleted id must report 0 and must not
    // decrement the live count again.
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    db.collection("c").unwrap();
    db.insert_one("c", doc("name", json!("keep"))).unwrap();
    let id = db.insert_one("c", doc("name", json!("victim"))).unwrap();

    assert_eq!(db.delete_one("c", &json!({ "_id": id })).unwrap(), 1);
    assert_eq!(db.delete_one("c", &json!({ "_id": id })).unwrap(), 0);
    assert_eq!(db.delete_one("c", &json!({ "_id": id })).unwrap(), 0);
    // exactly one live doc remains; count not driven negative/over-decremented
    assert_eq!(fresh_count(&db), 1);
}

/// THE P2-3 GUARD (fast path): N threads delete the SAME _id concurrently. Exactly
/// one may succeed; the live count must drop by exactly one. With the old code both
/// threads could pass the lock-free live read and each tombstone + decrement → total
/// deleted == 2 and the count under-counted. The assertion is a hard invariant (true
/// under ANY scheduling for correct code), so this test never fails spuriously.
#[test]
fn test_concurrent_delete_same_id_fast_path_decrements_once() {
    const THREADS: usize = 8;
    const ROUNDS: usize = 25;

    for _round in 0..ROUNDS {
        let db = Arc::new(DatabaseCore::<MemoryStorage>::open_memory().unwrap());
        db.collection("c").unwrap();
        db.insert_one("c", doc("filler", json!(1))).unwrap();
        let id = db.insert_one("c", doc("filler", json!(2))).unwrap();

        let barrier = Arc::new(Barrier::new(THREADS));
        let total = Arc::new(AtomicU64::new(0));

        let handles: Vec<_> = (0..THREADS)
            .map(|_| {
                let db = Arc::clone(&db);
                let barrier = Arc::clone(&barrier);
                let total = Arc::clone(&total);
                let id = id.clone();
                thread::spawn(move || {
                    barrier.wait();
                    let d = db.delete_one("c", &json!({ "_id": id })).unwrap();
                    total.fetch_add(d, Ordering::SeqCst);
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(
            total.load(Ordering::SeqCst),
            1,
            "exactly one concurrent delete of the same _id may succeed"
        );
        assert_eq!(
            fresh_count(&db),
            1,
            "live count must drop by exactly one (no double-decrement)"
        );
    }
}

/// Same invariant via the SLOW path (non-_id filter): N threads delete by the same
/// field filter that matches exactly one document.
#[test]
fn test_concurrent_delete_same_filter_slow_path_decrements_once() {
    const THREADS: usize = 8;
    const ROUNDS: usize = 25;

    for _round in 0..ROUNDS {
        let db = Arc::new(DatabaseCore::<MemoryStorage>::open_memory().unwrap());
        db.collection("c").unwrap();
        db.insert_one("c", doc("tag", json!("keep"))).unwrap();
        db.insert_one("c", doc("tag", json!("victim"))).unwrap();

        let barrier = Arc::new(Barrier::new(THREADS));
        let total = Arc::new(AtomicU64::new(0));

        let handles: Vec<_> = (0..THREADS)
            .map(|_| {
                let db = Arc::clone(&db);
                let barrier = Arc::clone(&barrier);
                let total = Arc::clone(&total);
                thread::spawn(move || {
                    barrier.wait();
                    let d = db.delete_one("c", &json!({ "tag": "victim" })).unwrap();
                    total.fetch_add(d, Ordering::SeqCst);
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(
            total.load(Ordering::SeqCst),
            1,
            "slow path: exactly one concurrent delete may succeed"
        );
        assert_eq!(fresh_count(&db), 1, "slow path: no double-decrement");
    }
}

/// Genuine concurrency over DISTINCT ids must delete each exactly once (the atomic
/// path must not lose or over-count deletes when threads touch different docs).
#[test]
fn test_concurrent_delete_distinct_ids_exact_count() {
    const N: usize = 64;

    let db = Arc::new(DatabaseCore::<MemoryStorage>::open_memory().unwrap());
    db.collection("c").unwrap();
    let ids: Vec<DocumentId> = (0..N)
        .map(|i| db.insert_one("c", doc("seq", json!(i))).unwrap())
        .collect();

    let barrier = Arc::new(Barrier::new(N));
    let total = Arc::new(AtomicU64::new(0));
    let handles: Vec<_> = ids
        .into_iter()
        .map(|id| {
            let db = Arc::clone(&db);
            let barrier = Arc::clone(&barrier);
            let total = Arc::clone(&total);
            thread::spawn(move || {
                barrier.wait();
                let d = db.delete_one("c", &json!({ "_id": id })).unwrap();
                total.fetch_add(d, Ordering::SeqCst);
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }

    assert_eq!(
        total.load(Ordering::SeqCst),
        N as u64,
        "each distinct id must be deleted exactly once"
    );
    assert_eq!(fresh_count(&db), 0);
}

// ===========================================================================
// delete_many — same chokepoint (tombstone_doc_atomic). Covers both the
// in-memory raw path (delete_many_raw_with_docs) and the Safe-mode bulk path
// (delete_many_prepare/persist), which previously also de-indexed before the
// storage lock with precomputed tombstone/index lists (audit P2-3).
// ===========================================================================

#[test]
fn test_delete_many_basic_memory() {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    db.collection("c").unwrap();
    for i in 0..10 {
        db.insert_one("c", HashMap::from([("g".to_string(), json!(i % 2))]))
            .unwrap();
    }
    let deleted = db.delete_many("c", &json!({ "g": 0 })).unwrap();
    assert_eq!(deleted, 5);
    assert_eq!(fresh_count(&db), 5);
}

#[test]
fn test_delete_many_safe_mode_prepare_persist() {
    // File-backed DB → Safe durability → delete_many_prepare + delete_many_persist.
    let dir = tempfile::TempDir::new().unwrap();
    let db = DatabaseCore::open(dir.path().join("dm.mlite")).unwrap();
    db.collection("c").unwrap();
    for i in 0..10 {
        db.insert_one("c", HashMap::from([("g".to_string(), json!(i % 2))]))
            .unwrap();
    }
    let deleted = db.delete_many("c", &json!({ "g": 1 })).unwrap();
    assert_eq!(deleted, 5);
    assert_eq!(
        db.collection("c")
            .unwrap()
            .count_documents(&json!({}))
            .unwrap(),
        5
    );
}

#[test]
fn test_delete_many_id_in_fast_path_safe() {
    // Exercises extract_id_in_query fast path in delete_many_prepare.
    let dir = tempfile::TempDir::new().unwrap();
    let db = DatabaseCore::open(dir.path().join("dm_in.mlite")).unwrap();
    db.collection("c").unwrap();
    let ids: Vec<DocumentId> = (0..6)
        .map(|i| {
            db.insert_one("c", HashMap::from([("n".to_string(), json!(i))]))
                .unwrap()
        })
        .collect();
    let target: Vec<Value> = ids
        .iter()
        .take(3)
        .map(|id| serde_json::to_value(id).unwrap())
        .collect();
    let deleted = db
        .delete_many("c", &json!({ "_id": { "$in": target } }))
        .unwrap();
    assert_eq!(deleted, 3);
    assert_eq!(
        db.collection("c")
            .unwrap()
            .count_documents(&json!({}))
            .unwrap(),
        3
    );
}

/// THE P2-3 GUARD for delete_many (in-memory raw path): N threads delete the same
/// filter matching M docs. MemoryStorage delete_many returns the ACTUAL tombstoned
/// count, so the sum across threads must be exactly M (each victim deleted once),
/// and the live count must drop by exactly M. The pre-fix code could tombstone the
/// same doc from two threads → sum > M and an over-decremented count.
#[test]
fn test_concurrent_delete_many_same_filter_memory_no_double_decrement() {
    const THREADS: usize = 8;
    const MATCH: u64 = 12;
    const ROUNDS: usize = 15;

    for _round in 0..ROUNDS {
        let db = Arc::new(DatabaseCore::<MemoryStorage>::open_memory().unwrap());
        db.collection("c").unwrap();
        for _ in 0..5 {
            db.insert_one("c", doc("tag", json!("keep"))).unwrap();
        }
        for _ in 0..MATCH {
            db.insert_one("c", doc("tag", json!("victim"))).unwrap();
        }

        let barrier = Arc::new(Barrier::new(THREADS));
        let total = Arc::new(AtomicU64::new(0));
        let handles: Vec<_> = (0..THREADS)
            .map(|_| {
                let db = Arc::clone(&db);
                let barrier = Arc::clone(&barrier);
                let total = Arc::clone(&total);
                thread::spawn(move || {
                    barrier.wait();
                    let d = db.delete_many("c", &json!({ "tag": "victim" })).unwrap();
                    total.fetch_add(d, Ordering::SeqCst);
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(
            total.load(Ordering::SeqCst),
            MATCH,
            "each victim deleted exactly once across all threads"
        );
        assert_eq!(
            fresh_count(&db),
            5,
            "only 'keep' docs survive; no double-decrement"
        );
    }
}

/// THE P2-3 GUARD for delete_many (Safe bulk path): same as above on a file-backed
/// DB. Safe-mode delete_many returns the prepare-time match count per call (so the
/// summed return is not a clean invariant under contention), but the LIVE count must
/// drop by exactly M — the persist re-read guard prevents double-decrement.
#[test]
fn test_concurrent_delete_many_safe_mode_no_double_decrement() {
    const THREADS: usize = 6;
    const MATCH: usize = 10;

    let dir = tempfile::TempDir::new().unwrap();
    let db = Arc::new(DatabaseCore::open(dir.path().join("dm_conc.mlite")).unwrap());
    db.collection("c").unwrap();
    for _ in 0..4 {
        db.insert_one("c", doc("tag", json!("keep"))).unwrap();
    }
    for _ in 0..MATCH {
        db.insert_one("c", doc("tag", json!("victim"))).unwrap();
    }

    let barrier = Arc::new(Barrier::new(THREADS));
    let handles: Vec<_> = (0..THREADS)
        .map(|_| {
            let db = Arc::clone(&db);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                db.delete_many("c", &json!({ "tag": "victim" })).unwrap();
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }

    let remaining = db
        .collection("c")
        .unwrap()
        .count_documents(&json!({}))
        .unwrap();
    assert_eq!(
        remaining, 4,
        "only 'keep' docs survive; live count not over-decremented"
    );
}
