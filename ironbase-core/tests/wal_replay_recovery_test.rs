//! Integration tests for task #26 phase 4/5:
//! WAL-replay based recovery for fulltext / fuzzy / hnsw indexes.
//!
//! Scenario for each test:
//!   1. Create DB, create non-btree index, insert batch A
//!   2. Explicit `checkpoint()` — non-btree index flushed with watermark=W
//!   3. Insert batch B (in WAL, not yet in index file)
//!   4. Simulate crash: `std::mem::forget(db)` so Drop does NOT run
//!      `mark_clean_shutdown` and the file header stays `was_clean=false`.
//!   5. Reopen — `initialize_index_manager` should take the WAL-replay path
//!      (load index file at W, apply ops with tx_id > W from WAL).
//!   6. Verify every doc from batch A ∪ B is findable via the index's search,
//!      proving post-crash recovery preserved all committed data.
//!
//! The tests assert END STATE only (data presence), which is valid regardless
//! of whether replay or the rebuild fallback executed. The replay path is
//! separately covered by the unit tests in `recovery::index_replay_ops::tests`
//! and by the `try_wal_replay_for_collection` logic exercised here.

use ironbase_core::storage::StorageEngine;
use ironbase_core::DatabaseCore;
use serde_json::json;
use std::collections::HashMap;
use tempfile::TempDir;

fn doc(tag: &str, content: &str) -> HashMap<String, serde_json::Value> {
    let mut m = HashMap::new();
    m.insert("tag".to_string(), json!(tag));
    m.insert("content".to_string(), json!(content));
    m
}

/// Open → insert A → checkpoint → insert B → simulated crash → reopen.
/// Verify all A ∪ B docs are findable via fulltext search.
#[test]
fn fulltext_wal_replay_recovery_preserves_all_docs() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("fts_replay.mlite");

    // Phase 1: create DB + fulltext index + batch A
    {
        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        let coll = db.collection("docs").unwrap();
        coll.create_fulltext_index("content".to_string(), "english", None, None)
            .unwrap();

        // Batch A: 10 docs, all mention "alpha"
        for i in 0..10 {
            db.insert_one(
                "docs",
                doc(&format!("a{}", i), &format!("alpha document number {}", i)),
            )
            .unwrap();
        }

        // Checkpoint → .ftidx written with watermark at this tx_id
        db.checkpoint().unwrap();

        // Batch B: 5 docs, all mention "beta" — these go into WAL only,
        // the .ftidx file does NOT yet reflect them.
        for i in 0..5 {
            db.insert_one(
                "docs",
                doc(&format!("b{}", i), &format!("beta document number {}", i)),
            )
            .unwrap();
        }

        // Simulate kill -9: release the file lock so this process can reopen,
        // but skip the clean shutdown path — the on-disk `clean_shutdown` byte
        // stays false so the next open takes the recovery branch.
        db.__simulate_crash_for_test();
    }

    // Phase 2: reopen after crash
    let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
    let coll = db.collection("docs").unwrap();

    // Alpha search: must find all 10 batch-A docs.
    let alpha = coll
        .fulltext_search("content", "alpha", Some(100), None, None, None)
        .unwrap();
    assert_eq!(
        alpha.len(),
        10,
        "batch-A docs lost after crash: expected 10, got {}",
        alpha.len()
    );

    // Beta search: must find all 5 batch-B docs — these were only in the WAL
    // at crash time, so replay (or rebuild fallback) must restore them.
    let beta = coll
        .fulltext_search("content", "beta", Some(100), None, None, None)
        .unwrap();
    assert_eq!(
        beta.len(),
        5,
        "batch-B WAL-only docs lost after crash: expected 5, got {}",
        beta.len()
    );

    // Total catalog sanity check
    assert_eq!(db.count_documents("docs", &json!({})).unwrap(), 15);
}

/// Same scenario but with fuzzy index. Verifies the fuzzy replay path.
#[test]
fn fuzzy_wal_replay_recovery_preserves_all_docs() {
    use ironbase_core::index::fuzzy::FuzzyAlgorithm;

    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("fz_replay.mlite");

    {
        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        let coll = db.collection("names").unwrap();
        coll.create_fuzzy_index("name".to_string(), FuzzyAlgorithm::JaroWinkler, 0.7)
            .unwrap();

        // Batch A
        for n in &["Alice", "Bob", "Charles", "Diana", "Eve"] {
            let mut d = HashMap::new();
            d.insert("name".to_string(), json!(n));
            db.insert_one("names", d).unwrap();
        }

        db.checkpoint().unwrap();

        // Batch B — after watermark
        for n in &["Frank", "Grace", "Henry"] {
            let mut d = HashMap::new();
            d.insert("name".to_string(), json!(n));
            db.insert_one("names", d).unwrap();
        }

        db.__simulate_crash_for_test();
    }

    // Reopen and verify every name is still fuzzy-searchable
    let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
    let coll = db.collection("names").unwrap();

    for exact in &[
        "Alice", "Bob", "Charles", "Diana", "Eve", "Frank", "Grace", "Henry",
    ] {
        let found = coll.find(&json!({ "name": { "$fuzzy": exact } })).unwrap();
        assert!(
            !found.is_empty(),
            "Fuzzy search for '{}' returned 0 results post-crash",
            exact
        );
    }

    assert_eq!(db.count_documents("names", &json!({})).unwrap(), 8);
}

/// Isolation: checkpoint then crash (no subsequent inserts) — does the file
/// produced by checkpoint alone survive reload?
#[test]
fn fulltext_survives_checkpoint_then_crash_no_post_inserts() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("fts_cp_crash.mlite");
    {
        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        let coll = db.collection("docs").unwrap();
        coll.create_fulltext_index("content".to_string(), "english", None, None)
            .unwrap();
        for i in 0..5 {
            db.insert_one("docs", doc(&format!("n{}", i), "alpha here"))
                .unwrap();
        }
        db.checkpoint().unwrap();
        // NO post-checkpoint inserts — crash immediately.
        db.__simulate_crash_for_test();
    }
    let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
    let coll = db.collection("docs").unwrap();
    let hits = coll
        .fulltext_search("content", "alpha", Some(100), None, None, None)
        .unwrap();
    println!("cp+crash (no post inserts): alpha = {}", hits.len());
    assert_eq!(hits.len(), 5);
}

/// Sanity: checkpoint + clean close + reopen — fulltext should survive.
/// Isolates whether checkpoint's .ftidx write is parseable on its own.
#[test]
fn fulltext_survives_checkpoint_then_clean_close() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("fts_cp.mlite");
    {
        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        let coll = db.collection("docs").unwrap();
        coll.create_fulltext_index("content".to_string(), "english", None, None)
            .unwrap();
        for i in 0..5 {
            db.insert_one("docs", doc(&format!("n{}", i), "alpha here"))
                .unwrap();
        }
        db.checkpoint().unwrap();
        // Drop naturally
    }
    let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
    let coll = db.collection("docs").unwrap();
    let hits = coll
        .fulltext_search("content", "alpha", Some(100), None, None, None)
        .unwrap();
    println!("cp+close reopen: alpha = {}", hits.len());
    assert_eq!(hits.len(), 5);
}

/// Sanity: after a clean close WITHOUT explicit checkpoint, fulltext survives
/// reopen. This confirms the normal persistence flow works.
#[test]
fn fulltext_survives_normal_reopen_no_checkpoint() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("fts_plain.mlite");
    {
        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        let coll = db.collection("docs").unwrap();
        coll.create_fulltext_index("content".to_string(), "english", None, None)
            .unwrap();
        for i in 0..5 {
            db.insert_one("docs", doc(&format!("n{}", i), "word alpha here"))
                .unwrap();
        }
        let pre = coll
            .fulltext_search("content", "alpha", Some(100), None, None, None)
            .unwrap();
        println!("plain reopen: pre-close alpha = {}", pre.len());
    }
    let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
    let coll = db.collection("docs").unwrap();
    let post = coll
        .fulltext_search("content", "alpha", Some(100), None, None, None)
        .unwrap();
    println!("plain reopen: post-reopen alpha = {}", post.len());
    assert_eq!(
        post.len(),
        5,
        "fulltext survived normal reopen without checkpoint"
    );
}

/// Clean shutdown path — no replay needed. Sanity check that the new
/// `can_try_replay = !was_clean` gate does NOT trigger on clean shutdown
/// and the fast path still works end-to-end.
#[test]
fn clean_shutdown_does_not_trigger_replay_path() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("fts_clean.mlite");

    {
        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        let coll = db.collection("docs").unwrap();
        coll.create_fulltext_index("content".to_string(), "english", None, None)
            .unwrap();

        for i in 0..5 {
            db.insert_one(
                "docs",
                doc(&format!("c{}", i), &format!("clean shutdown doc {}", i)),
            )
            .unwrap();
        }

        // Drop naturally → Drop impl runs → flush + mark_clean_shutdown
    }

    let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
    let coll = db.collection("docs").unwrap();
    let hits = coll
        .fulltext_search("content", "clean", Some(100), None, None, None)
        .unwrap();
    assert_eq!(hits.len(), 5);
}
