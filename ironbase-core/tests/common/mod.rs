//! Shared oracle helpers for query-planner correctness regression tests
//! (audit 2026-06-06).
//!
//! Oracle pattern: insert the SAME documents into an indexed collection and a
//! non-indexed one. `count_documents` and `find` must agree with the
//! ground-truth collection scan (the non-indexed collection). Any divergence
//! is a planner/index correctness bug.
#![allow(dead_code)]

use ironbase_core::storage::MemoryStorage;
use ironbase_core::DatabaseCore;
use serde_json::Value;
use std::collections::HashMap;

pub fn doc(v: &Value) -> HashMap<String, Value> {
    v.as_object()
        .expect("doc must be a JSON object")
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

fn build(docs: &[Value]) -> DatabaseCore<MemoryStorage> {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    db.collection("idx").unwrap();
    db.collection("noidx").unwrap();
    for d in docs {
        db.insert_one("idx", doc(d)).unwrap();
        db.insert_one("noidx", doc(d)).unwrap();
    }
    db
}

fn compare(db: &DatabaseCore<MemoryStorage>, query: &Value) {
    let cidx = db.get_collection("idx").unwrap();
    let cno = db.get_collection("noidx").unwrap();

    let count_idx = cidx.count_documents(query).unwrap();
    let count_scan = cno.count_documents(query).unwrap();
    let find_idx = cidx.find(query).unwrap().len() as u64;
    let find_scan = cno.find(query).unwrap().len() as u64;

    // Ground truth: scan count == scan find (sanity).
    assert_eq!(
        count_scan, find_scan,
        "ground-truth count/find disagree (no index) for {query}"
    );
    assert_eq!(
        count_idx, count_scan,
        "indexed count_documents diverges from collection scan for {query}: {count_idx} != {count_scan}"
    );
    assert_eq!(
        find_idx, find_scan,
        "indexed find diverges from collection scan for {query}: {find_idx} != {find_scan}"
    );
}

/// Single-field B+ tree index (non-unique, non-sparse) on `index_field`.
pub fn assert_index_matches_scan(docs: &[Value], index_field: &str, query: &Value) {
    let db = build(docs);
    db.get_collection("idx")
        .unwrap()
        .create_index(index_field.to_string(), false, false)
        .unwrap();
    compare(&db, query);
}

/// Case-insensitive B+ tree index (non-unique) on `index_field`.
pub fn assert_ci_index_matches_scan(docs: &[Value], index_field: &str, query: &Value) {
    let db = build(docs);
    db.get_collection("idx")
        .unwrap()
        .create_ci_index(index_field.to_string(), false)
        .unwrap();
    compare(&db, query);
}
