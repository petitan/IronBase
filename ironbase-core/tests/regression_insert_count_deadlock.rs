//! Regression test for issue #75: ABBA deadlock between a concurrent
//! `insert_many` and an index-using filtered `count`.
//!
//! Root cause: the write paths (`insert_many_raw` / `insert_one_raw` /
//! `update_*_raw`) hold `storage.write()` across their index updates for
//! read-modify-write atomicity, i.e. they lock in **storage → indexes** order.
//! `count_with_plan` (and `drop_index`) used the opposite **indexes → storage**
//! order — holding `indexes.read()` while acquiring `storage.read()`. Under
//! concurrency this formed a classic ABBA cycle:
//!   * writer holds `storage.write`, waits for `indexes.write` (batch_add)
//!   * reader holds `indexes.read`, waits for `storage.read`
//!
//! Fix: unify everyone on the **storage → indexes** order. This test pins the
//! invariant: a writer and a reader hammering the same collection must both make
//! progress and finish promptly. Before the fix the writer deadlocked on its
//! very first `insert_many` (0 inserts completed).

use ironbase_core::database::DatabaseCore;
use ironbase_core::storage::MemoryStorage;
use serde_json::json;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

#[test]
fn insert_many_and_filtered_count_do_not_deadlock() {
    let db = Arc::new(DatabaseCore::<MemoryStorage>::open_memory().unwrap());
    let coll = db.collection("probe").unwrap();
    // Index on "status" makes the filtered count a fully-covered IndexScan,
    // which is exactly the count_with_plan branch that co-held indexes+storage.
    coll.create_index("status".to_string(), false, false)
        .unwrap();

    let stop = Arc::new(AtomicBool::new(false));
    let inserted = Arc::new(AtomicU64::new(0));
    let counts = Arc::new(AtomicU64::new(0));

    // Writer: continuous insert_many batches.
    let writer = {
        let db = Arc::clone(&db);
        let stop = Arc::clone(&stop);
        let inserted = Arc::clone(&inserted);
        thread::spawn(move || {
            let mut seq = 0u64;
            while !stop.load(Ordering::Relaxed) {
                let mut batch = Vec::with_capacity(200);
                for _ in 0..200 {
                    seq += 1;
                    batch.push(HashMap::from([
                        ("status".to_string(), json!("active")),
                        ("seq".to_string(), json!(seq)),
                    ]));
                }
                db.insert_many("probe", batch).expect("insert_many");
                inserted.fetch_add(200, Ordering::Relaxed);
            }
        })
    };

    // Reader: continuous filtered count (fully-covered index path).
    let reader = {
        let db = Arc::clone(&db);
        let stop = Arc::clone(&stop);
        let counts = Arc::clone(&counts);
        thread::spawn(move || {
            let coll = db.collection("probe").unwrap();
            while !stop.load(Ordering::Relaxed) {
                coll.count_documents(&json!({"status": "active"}))
                    .expect("count");
                counts.fetch_add(1, Ordering::Relaxed);
            }
        })
    };

    // Run briefly, then require BOTH threads to wind down quickly.
    thread::sleep(Duration::from_secs(3));
    stop.store(true, Ordering::Relaxed);

    let deadline = Instant::now() + Duration::from_secs(10);
    while !(writer.is_finished() && reader.is_finished()) {
        assert!(
            Instant::now() < deadline,
            "DEADLOCK: threads stuck (inserted={}, counts={})",
            inserted.load(Ordering::Relaxed),
            counts.load(Ordering::Relaxed)
        );
        thread::sleep(Duration::from_millis(25));
    }
    writer.join().unwrap();
    reader.join().unwrap();

    // Both sides must have made real progress.
    assert!(
        inserted.load(Ordering::Relaxed) > 0,
        "writer made no progress"
    );
    assert!(
        counts.load(Ordering::Relaxed) > 0,
        "reader made no progress"
    );
}
