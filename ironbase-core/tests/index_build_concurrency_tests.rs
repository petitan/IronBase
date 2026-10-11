// index_build_concurrency_tests.rs
// create_index while other threads update, delete and insert (audit
// 2026-10-07 T2): the build must not fail with a spurious unique error, and the
// finished index must hold exactly the live documents' current keys.

use ironbase_core::storage::MemoryStorage;
use ironbase_core::DatabaseCore;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

const DOCS: i64 = 2000;

fn fields(v: Value) -> HashMap<String, Value> {
    v.as_object().unwrap().clone().into_iter().collect()
}

fn check_index(db: &DatabaseCore<MemoryStorage>, field: &str) {
    let coll = db.collection("c").unwrap();
    let live = coll.find(&json!({})).unwrap();
    let size = coll
        .indexes
        .read()
        .get_btree_index(&format!("c_{field}"))
        .unwrap()
        .size();
    assert_eq!(
        size as usize,
        live.len(),
        "{field}: index entries != live documents"
    );
    for d in &live {
        let hits = coll.find(&json!({ field: d[field].clone() })).unwrap();
        assert!(
            hits.iter().any(|h| h["_id"] == d["_id"]),
            "{field}: {:?} not found through the index",
            d["_id"]
        );
    }
}

#[test]
fn create_index_concurrent_with_writers_is_consistent() {
    for round in 0..2 {
        let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
        for i in 0..DOCS {
            db.insert_one("c", fields(json!({"_id": i, "u": i, "g": i % 7})))
                .unwrap();
        }
        let done = AtomicBool::new(false);
        std::thread::scope(|s| {
            s.spawn(|| {
                let mut k: i64 = 0;
                let mut next = DOCS;
                while !done.load(Ordering::SeqCst) {
                    let id = (k * 7919 + round) % DOCS;
                    match k % 3 {
                        0 => {
                            let _ = db.update_one(
                                "c",
                                &json!({"_id": id}),
                                &json!({"$set": {"u": DOCS * 10 + k, "g": k % 5 + 100}}),
                            );
                        }
                        1 => {
                            let _ = db.delete_one("c", &json!({"_id": id}));
                        }
                        _ => {
                            db.insert_one(
                                "c",
                                fields(json!({"_id": next, "u": DOCS * 1000 + next, "g": 9})),
                            )
                            .unwrap();
                            next += 1;
                        }
                    }
                    k += 1;
                }
            });
            let coll = db.collection("c").unwrap();
            let unique = coll.create_index("u".to_string(), true, false);
            let plain = coll.create_index("g".to_string(), false, false);
            done.store(true, Ordering::SeqCst);
            unique.expect("unique build failed under concurrent writes");
            plain.expect("non-unique build failed under concurrent writes");
        });
        check_index(&db, "u");
        check_index(&db, "g");
    }
}

/// Non-unique index: no phantom (deleted) or stale (pre-update) entries.
#[test]
fn plain_index_build_concurrent_with_writers_has_no_stale_entries() {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    for i in 0..DOCS {
        db.insert_one("c", fields(json!({"_id": i, "g": i % 7})))
            .unwrap();
    }
    let done = AtomicBool::new(false);
    std::thread::scope(|s| {
        s.spawn(|| {
            let mut k: i64 = 0;
            while !done.load(Ordering::SeqCst) {
                let id = (k * 7919) % DOCS;
                if k % 2 == 0 {
                    let _ = db.update_one(
                        "c",
                        &json!({"_id": id}),
                        &json!({"$set": {"g": k % 5 + 100}}),
                    );
                } else {
                    let _ = db.delete_one("c", &json!({"_id": id}));
                }
                k += 1;
            }
        });
        let built = db
            .collection("c")
            .unwrap()
            .create_index("g".to_string(), false, false);
        done.store(true, Ordering::SeqCst);
        built.unwrap();
    });
    check_index(&db, "g");
}
