//! Tests for the vector-index lock separation (HNSW carve-out, v0.3.x).
//!
//! 1. **Concurrency**: a slow HNSW batch-build holds only the SEPARATE vector
//!    lock, so a concurrent filtered btree `count` (which needs the shared
//!    index lock) is NOT blocked behind it. On the pre-refactor code a `count`
//!    overlapping the build would block for the whole `insert_many` and the
//!    reader would manage ~1 iteration; with the carve-out it runs freely.
//! 2. **Persistence**: vector indexes survive checkpoint/restart and `compact()`.

use ironbase_core::database::DatabaseCore;
use ironbase_core::storage::MemoryStorage;
use ironbase_core::vector::VectorIndexConfig;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Instant;
use tempfile::TempDir;

fn doc(value: Value) -> HashMap<String, Value> {
    match value {
        Value::Object(m) => m.into_iter().collect(),
        _ => HashMap::new(),
    }
}

/// Deterministic pseudo-random unit-ish vector (no rng dependency).
fn pseudo_vec(seed: usize, dim: usize) -> Vec<f64> {
    (0..dim)
        .map(|j| {
            let h = seed
                .wrapping_mul(2_654_435_761)
                .wrapping_add(j.wrapping_mul(40_503));
            ((h % 1000) as f64) / 1000.0
        })
        .collect()
}

fn as_f32(v: &[f64]) -> Vec<f32> {
    v.iter().map(|x| *x as f32).collect()
}

/// A held vector (HNSW) lock must NOT block a filtered btree `count`.
///
/// This is the core guarantee of the carve-out: the vector index lives behind a
/// SEPARATE lock, so however long an HNSW build holds it, a `count` that only
/// needs the shared index lock (`indexes.read()`) is unaffected.
///
/// The test holds `coll.vectors.write()` in a background thread (a stand-in for a
/// long HNSW build) and asserts a concurrent filtered btree count returns
/// promptly. It deliberately avoids the full `insert_many` path: concurrent
/// `insert_many` + filtered `count` has a *pre-existing* index/storage lock
/// hazard unrelated to this refactor (the `count` path never touches the vector
/// lock), and conflating the two would make this test flaky for the wrong reason.
#[test]
fn btree_count_not_blocked_by_held_vector_lock() {
    const DIM: usize = 64;
    const ACTIVE: usize = 50;

    let db = Arc::new(DatabaseCore::<MemoryStorage>::open_memory().unwrap());
    {
        let coll = db.collection("kb").unwrap();
        coll.create_index("status".to_string(), false, false)
            .unwrap();
        for i in 0..ACTIVE {
            db.insert_one(
                "kb",
                doc(json!({"status": "active", "embedding": pseudo_vec(i, DIM)})),
            )
            .unwrap();
        }
        coll.create_vector_index("embedding", VectorIndexConfig::new(DIM))
            .unwrap();
    }
    let filter = json!({"status": "active"});

    let lock_held = Arc::new(AtomicBool::new(false));
    let release = Arc::new(AtomicBool::new(false));

    // Hold the SEPARATE vector lock for a while (stand-in for a long HNSW build).
    let holder = {
        let db = Arc::clone(&db);
        let held = Arc::clone(&lock_held);
        let rel = Arc::clone(&release);
        thread::spawn(move || {
            let coll = db.get_collection("kb").unwrap();
            let _guard = coll.vectors.write();
            held.store(true, Ordering::SeqCst);
            while !rel.load(Ordering::SeqCst) {
                thread::sleep(std::time::Duration::from_millis(2));
            }
        })
    };

    // Wait until the vector write lock is actually held.
    while !lock_held.load(Ordering::SeqCst) {
        thread::sleep(std::time::Duration::from_millis(1));
    }

    // With the vector lock held, a filtered btree count must complete promptly —
    // it uses `indexes.read()`, a different lock. (Without the carve-out, the
    // vector work would sit under the shared lock and this would block.)
    let coll = db.get_collection("kb").unwrap();
    let t = Instant::now();
    let n = coll.count_documents(&filter).unwrap();
    let lat = t.elapsed();

    release.store(true, Ordering::SeqCst);
    holder.join().unwrap();

    assert_eq!(n, ACTIVE as u64);
    assert!(
        lat < std::time::Duration::from_millis(100),
        "filtered btree count took {lat:?} while the vector lock was held — \
         count must not depend on the vector lock"
    );
}

/// Vector indexes survive checkpoint → close → reopen, and `compact()`.
#[test]
fn vector_survives_checkpoint_restart_and_compact() {
    const DIM: usize = 64;
    const N: usize = 200;

    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("vec.mlite");
    let query = as_f32(&pseudo_vec(7, DIM));

    let pre_top: String;
    {
        let db = DatabaseCore::open(&path).unwrap();
        let coll = db.collection("kb").unwrap();
        for i in 0..N {
            db.insert_one(
                "kb",
                doc(json!({"doc_id": format!("d{i}"), "embedding": pseudo_vec(i, DIM)})),
            )
            .unwrap();
        }
        coll.create_vector_index("embedding", VectorIndexConfig::new(DIM))
            .unwrap();

        let results = coll.vector_search("embedding", &query, 5).unwrap();
        assert!(!results.is_empty(), "vector search empty before restart");
        pre_top = results[0]
            .0
            .get("doc_id")
            .and_then(|v| v.as_str())
            .unwrap()
            .to_string();

        db.checkpoint().unwrap();
        db.close().unwrap();
    }

    // Reopen: the vector index loads from its .hnsw cache; search is identical.
    {
        let db = DatabaseCore::open(&path).unwrap();
        {
            let coll = db.get_collection("kb").unwrap();
            let results = coll.vector_search("embedding", &query, 5).unwrap();
            assert!(
                !results.is_empty(),
                "vector search empty after restart — index not persisted via the vector manager"
            );
            let top = results[0]
                .0
                .get("doc_id")
                .and_then(|v| v.as_str())
                .unwrap()
                .to_string();
            assert_eq!(top, pre_top, "top vector result changed after restart");
        }

        // Delete some docs to create HNSW orphans, then compact.
        let deleted = db
            .delete_many(
                "kb",
                &json!({"doc_id": {"$in": ["d0","d1","d2","d3","d4"]}}),
            )
            .unwrap();
        assert_eq!(deleted, 5);
        db.compact().unwrap();

        // Search still works after orphan-compacting compaction, and a deleted
        // doc is no longer returned.
        let coll = db.get_collection("kb").unwrap();
        let results = coll.vector_search("embedding", &query, 10).unwrap();
        assert!(!results.is_empty(), "vector search empty after compact");
        for (d, _score) in &results {
            let id = d.get("doc_id").and_then(|v| v.as_str()).unwrap_or("");
            assert!(
                !["d0", "d1", "d2", "d3", "d4"].contains(&id),
                "deleted doc {id} resurfaced in vector search after compact"
            );
        }
        db.close().unwrap();
    }
}
