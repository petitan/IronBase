// scan_lock_release_tests.rs
// An unindexed count / find scan must not hold the storage lock for the
// whole scan: a concurrent insert used to wait until it finished (audit
// 2026-10-07 T5).

use ironbase_core::find_options::FindOptions;
use ironbase_core::storage::MemoryStorage;
use ironbase_core::DatabaseCore;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const DOCS: usize = 40_000;

fn db() -> DatabaseCore<MemoryStorage> {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    let pad = "x".repeat(400);
    for n in 0..DOCS {
        let fields: HashMap<String, Value> =
            HashMap::from([("n".to_string(), json!(n)), ("pad".to_string(), json!(pad))]);
        db.insert_one("c", fields).unwrap();
    }
    db
}

/// Runs `scan` on one thread while another inserts every 5 ms until it is done;
/// returns (scan duration, longest single insert).
fn race(db: &DatabaseCore<MemoryStorage>, scan: impl Fn() + Send + Sync) -> (Duration, Duration) {
    let done = AtomicBool::new(false);
    std::thread::scope(|s| {
        let scanner = s.spawn(|| {
            let t = Instant::now();
            scan();
            done.store(true, Ordering::SeqCst);
            t.elapsed()
        });
        let mut longest = Duration::ZERO;
        let mut n = -1i64;
        while !done.load(Ordering::SeqCst) {
            let fields: HashMap<String, Value> = HashMap::from([("n".to_string(), json!(n))]);
            let t = Instant::now();
            db.insert_one("c", fields).unwrap();
            longest = longest.max(t.elapsed());
            n -= 1;
            // pace the writer so it does not slow the scan itself down
            std::thread::sleep(Duration::from_millis(5));
        }
        (scanner.join().unwrap(), longest)
    })
}

#[test]
fn unindexed_count_does_not_block_writers() {
    let db = db();
    let coll = db.collection("c").unwrap();
    let query = json!({"pad": {"$regex": "y"}, "n": {"$gte": 0}});
    let (scan, insert) = race(&db, || {
        assert_eq!(coll.count_documents(&query).unwrap(), 0);
    });
    assert!(
        insert < scan / 8,
        "an insert waited {insert:?} during a {scan:?} count scan"
    );
}

#[test]
fn find_document_load_does_not_block_writers() {
    let db = db();
    let coll = db.collection("c").unwrap();
    // the index makes collecting the ids cheap, so the document load (and
    // the in-memory sort on another field) is most of the find
    coll.create_index("n".to_string(), false, false).unwrap();
    let (scan, insert) = race(&db, || {
        // sort without limit: the slow path loads every match
        let opts = FindOptions::new().with_sort(vec![("pad".to_string(), 1)]);
        let docs = coll
            .find_with_options(&json!({"n": {"$gte": 0}}), opts)
            .unwrap();
        assert!(docs.len() >= DOCS);
    });
    assert!(
        insert < scan / 8,
        "an insert waited {insert:?} during a {scan:?} find"
    );
}
