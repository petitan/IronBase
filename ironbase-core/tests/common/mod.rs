//! Shared oracle helpers for query-planner correctness regression tests
//! (audit 2026-06-06).
//!
//! Oracle pattern: insert the SAME documents into an indexed collection and a
//! non-indexed one. `count_documents` and `find` must agree with the
//! ground-truth collection scan (the non-indexed collection). Any divergence
//! is a planner/index correctness bug.
#![allow(dead_code)]

use ironbase_core::storage::MemoryStorage;
use ironbase_core::{DatabaseCore, StorageEngine};
use serde_json::Value;
use std::collections::HashMap;
use tempfile::TempDir;

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

/// Compares indexed `count`/`find` against the ground-truth collection scan.
/// Returns the ground-truth row count so the caller can also assert the
/// file-backed streaming cursor (see `assert_cursor_matches_scan`).
fn compare(db: &DatabaseCore<MemoryStorage>, query: &Value) -> u64 {
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
    count_scan
}

/// Third leg of the parity oracle: drain `find_streaming` on a **file-backed**
/// collection and assert the cursor row count matches the ground-truth scan.
///
/// MemoryStorage early-returns `find_streaming` to the materialized
/// `collect_doc_ids_from_plan` path, so the streaming cursor
/// (`FindCursor::*_from_plan`) is only ever exercised on a real `.mlite` file.
/// Without this leg a cursor-only divergence (e.g. the numeric Int/Float
/// two-bucket split that originally missed the cursor) is invisible to the
/// MemoryStorage `count == find == scan` checks above.
fn assert_cursor_matches_scan(
    docs: &[Value],
    index_field: &str,
    query: &Value,
    ci: bool,
    scan: u64,
) {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("parity.mlite");
    let db = DatabaseCore::<StorageEngine>::open(&path).unwrap();
    db.collection("idx").unwrap();
    for d in docs {
        db.insert_one("idx", doc(d)).unwrap();
    }
    let cidx = db.get_collection("idx").unwrap();
    if ci {
        cidx.create_ci_index(index_field.to_string(), false)
            .unwrap();
    } else {
        cidx.create_index(index_field.to_string(), false, false)
            .unwrap();
    }

    let mut cursor = cidx.find_streaming(query).unwrap();
    let mut cursor_n = 0u64;
    while cursor.next().unwrap().is_some() {
        cursor_n += 1;
    }
    assert_eq!(
        cursor_n, scan,
        "file-backed streaming cursor diverges from collection scan for {query}: {cursor_n} != {scan}"
    );
}

/// Single-field B+ tree index (non-unique, non-sparse) on `index_field`.
pub fn assert_index_matches_scan(docs: &[Value], index_field: &str, query: &Value) {
    let db = build(docs);
    db.get_collection("idx")
        .unwrap()
        .create_index(index_field.to_string(), false, false)
        .unwrap();
    let scan = compare(&db, query);
    assert_cursor_matches_scan(docs, index_field, query, false, scan);
}

/// Case-insensitive B+ tree index (non-unique) on `index_field`.
pub fn assert_ci_index_matches_scan(docs: &[Value], index_field: &str, query: &Value) {
    let db = build(docs);
    db.get_collection("idx")
        .unwrap()
        .create_ci_index(index_field.to_string(), false)
        .unwrap();
    let scan = compare(&db, query);
    assert_cursor_matches_scan(docs, index_field, query, true, scan);
}
