//! Reproduction + regression test for the upsert TOCTOU (audit P2-4).
//!
//! `update_one_with_options` performs upsert as `update_one()` (match — releases all
//! locks) THEN `insert_one()`, with NO lock spanning the two. Two concurrent upserts
//! on the same filter (no matching doc, no unique index) can therefore both observe
//! matched==0 and each insert → duplicate documents. MongoDB avoids this with a
//! unique index on the query field; without one it is a real race.
//!
//! These tests assert the CORRECT behaviour (one doc); they FAIL on the pre-fix code,
//! which is how the bug's reachability was empirically confirmed before fixing.

use ironbase_core::{storage::MemoryStorage, DatabaseCore, UpdateOptions};
use serde_json::json;
use std::sync::{Arc, Barrier};
use std::thread;

fn count_by_email(db: &DatabaseCore<MemoryStorage>, email: &str) -> u64 {
    // Fresh handle — avoids any per-handle query-cache staleness (audit P2-5).
    db.collection("users")
        .unwrap()
        .count_documents(&json!({ "email": email }))
        .unwrap()
}

/// THE P2-4 GUARD: N threads upsert the SAME filter concurrently (no pre-existing
/// doc, no unique index). At most ONE may insert; the rest must update the winner.
/// On the pre-fix code both can pass matched==0 and insert → >1 doc.
#[test]
fn test_concurrent_upsert_same_filter_creates_one_doc() {
    const THREADS: usize = 8;
    const ROUNDS: usize = 25;

    for _round in 0..ROUNDS {
        let db = Arc::new(DatabaseCore::<MemoryStorage>::open_memory().unwrap());
        db.collection("users").unwrap();

        let barrier = Arc::new(Barrier::new(THREADS));
        let handles: Vec<_> = (0..THREADS)
            .map(|i| {
                let db = Arc::clone(&db);
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    db.update_one_with_options(
                        "users",
                        &json!({ "email": "race@example.com" }),
                        &json!({ "$set": { "n": i } }),
                        UpdateOptions::new().with_upsert(true),
                    )
                    .unwrap();
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(
            count_by_email(&db, "race@example.com"),
            1,
            "concurrent upsert on the same filter must yield exactly one document"
        );
    }
}

/// Single-threaded upsert must still behave: first call inserts, second updates.
#[test]
fn test_upsert_sequential_insert_then_update() {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    db.collection("users").unwrap();

    let r1 = db
        .update_one_with_options(
            "users",
            &json!({ "email": "a@example.com" }),
            &json!({ "$set": { "name": "A" } }),
            UpdateOptions::new().with_upsert(true),
        )
        .unwrap();
    assert!(r1.upserted_id.is_some(), "first upsert inserts");

    let r2 = db
        .update_one_with_options(
            "users",
            &json!({ "email": "a@example.com" }),
            &json!({ "$set": { "name": "B" } }),
            UpdateOptions::new().with_upsert(true),
        )
        .unwrap();
    assert!(
        r2.upserted_id.is_none(),
        "second upsert updates the existing doc"
    );
    assert_eq!(r2.matched_count, 1);

    assert_eq!(count_by_email(&db, "a@example.com"), 1);
}

/// Same P2-4 guard on a FILE-backed DB → Safe durability → the generic
/// `update_one_with_options` variant (the MemoryStorage one is covered above).
#[test]
fn test_concurrent_upsert_same_filter_safe_mode() {
    const THREADS: usize = 8;
    const ROUNDS: usize = 3;

    for _round in 0..ROUNDS {
        let dir = tempfile::TempDir::new().unwrap();
        let db = Arc::new(DatabaseCore::open(dir.path().join("upsert.mlite")).unwrap());
        db.collection("users").unwrap();

        let barrier = Arc::new(Barrier::new(THREADS));
        let handles: Vec<_> = (0..THREADS)
            .map(|i| {
                let db = Arc::clone(&db);
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    db.update_one_with_options(
                        "users",
                        &json!({ "email": "race@example.com" }),
                        &json!({ "$set": { "n": i } }),
                        UpdateOptions::new().with_upsert(true),
                    )
                    .unwrap();
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }

        let count = db
            .collection("users")
            .unwrap()
            .count_documents(&json!({ "email": "race@example.com" }))
            .unwrap();
        assert_eq!(
            count, 1,
            "Safe-mode concurrent upsert must yield exactly one document"
        );
    }
}
