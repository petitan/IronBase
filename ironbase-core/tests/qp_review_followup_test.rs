//! Regression tests for the two pre-existing count/find divergences surfaced by
//! the #8 (QueryPlan executor unification) code review, 2026-06-08:
//!
//! 1. A single-sided non-numeric `IndexRangeScan` whose defaulted open end
//!    crosses other key-type buckets was counted EXACT (no post-filter), so
//!    `count_documents` summed index keys the operator can never match — e.g.
//!    `{f: {$lt: "z"}}` over mixed-type data returned 6 vs the correct 1.
//!    Fixed: `PlanRanges::from_plan` marks such a range non-exact, routing count
//!    through the index-narrowed post-filter (matching find).
//!
//! 2. A numeric `IndexRangeScan` (Int/Float two-bucket scan) returns its doc_ids
//!    DocumentId-sorted, not in index-key order, yet `collect_doc_ids_from_plan`
//!    reported `uses_index_sort = true` for it — so `find(...).sort({f: 1})`
//!    skipped the memory sort and returned DocumentId order. Fixed: index-sort is
//!    claimed only for a single contiguous (index-ordered) scan.

mod common;

use common::assert_index_matches_scan;
use ironbase_core::storage::MemoryStorage;
use ironbase_core::{DatabaseCore, FindOptions};
use serde_json::{json, Value};
use std::collections::HashMap;

fn doc(v: &Value) -> HashMap<String, Value> {
    v.as_object()
        .unwrap()
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

// ----------------------------------------------------------------------------
// Finding #1 — cross-type single-sided range count must equal find/scan.
// assert_index_matches_scan checks count == find == collection-scan AND drains a
// file-backed streaming cursor, across every bound shape.
// ----------------------------------------------------------------------------

fn mixed_type_docs() -> Vec<Value> {
    vec![
        json!({"f": 5}),
        json!({"f": 7}),
        json!({"f": 1.5}),
        json!({"f": true}),
        json!({"f": false}),
        json!({"f": "apple"}),
        json!({"f": "zebra"}),
        json!({"f": null}),
    ]
}

#[test]
fn cross_type_lt_string_upper_open_lower() {
    // [Null, "z") spans Int/Float/Bool/Null — only "apple" matches.
    assert_index_matches_scan(&mixed_type_docs(), "f", &json!({"f": {"$lt": "z"}}));
}

#[test]
fn cross_type_lte_string_upper_open_lower() {
    assert_index_matches_scan(&mixed_type_docs(), "f", &json!({"f": {"$lte": "m"}}));
}

#[test]
fn cross_type_gte_string_lower_open_upper() {
    // [String("a"), MaxKey] is type-homogeneous on a single-field index → stays
    // exact AND correct (only strings >= "a").
    assert_index_matches_scan(&mixed_type_docs(), "f", &json!({"f": {"$gte": "a"}}));
}

#[test]
fn cross_type_gt_string_lower_open_upper() {
    assert_index_matches_scan(&mixed_type_docs(), "f", &json!({"f": {"$gt": "apple"}}));
}

#[test]
fn cross_type_bounded_string_range() {
    // Fully-bounded same-bucket range — exact and correct.
    assert_index_matches_scan(
        &mixed_type_docs(),
        "f",
        &json!({"f": {"$gte": "a", "$lte": "z"}}),
    );
}

#[test]
fn cross_type_gte_bool_lower_open_upper() {
    // [Bool(true), MaxKey] crosses Int/Float/String → must NOT be exact.
    assert_index_matches_scan(&mixed_type_docs(), "f", &json!({"f": {"$gte": true}}));
}

#[test]
fn cross_type_numeric_range_unaffected() {
    // Control: a numeric range still counts both Int+Float buckets exactly.
    assert_index_matches_scan(
        &mixed_type_docs(),
        "f",
        &json!({"f": {"$gte": 0, "$lt": 100}}),
    );
}

// ----------------------------------------------------------------------------
// Finding #2 — find(...).sort({field}) order must match the collection scan,
// including for numeric (Int/Float two-bucket) ranges.
// ----------------------------------------------------------------------------

/// Build an indexed + a non-indexed collection with the same docs (insertion
/// order deliberately != sorted order), then assert that for every (query, sort)
/// the indexed `find_with_options` returns rows in the SAME order as the
/// ground-truth collection scan.
fn assert_sort_order_matches_scan(docs: &[Value], field: &str, query: &Value, sort_field: &str) {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    db.collection("idx").unwrap();
    db.collection("noidx").unwrap();
    for d in docs {
        db.insert_one("idx", doc(d)).unwrap();
        db.insert_one("noidx", doc(d)).unwrap();
    }
    db.get_collection("idx")
        .unwrap()
        .create_index(field.to_string(), false, false)
        .unwrap();

    for dir in [1, -1] {
        let opts = FindOptions::new().with_sort(vec![(sort_field.to_string(), dir)]);
        let got: Vec<Value> = db
            .get_collection("idx")
            .unwrap()
            .find_with_options(query, opts.clone())
            .unwrap()
            .iter()
            .map(|d| d[sort_field].clone())
            .collect();
        let want: Vec<Value> = db
            .get_collection("noidx")
            .unwrap()
            .find_with_options(query, opts)
            .unwrap()
            .iter()
            .map(|d| d[sort_field].clone())
            .collect();
        assert_eq!(
            got, want,
            "indexed sort order diverges from scan for {query} sort {sort_field}:{dir}"
        );
    }
}

fn unsorted_numeric_docs() -> Vec<Value> {
    // insertion order (= DocumentId order) is NOT the sorted order; mixed Int/Float
    vec![
        json!({"f": 40}),
        json!({"f": 30.5}),
        json!({"f": 20}),
        json!({"f": 50.0}),
        json!({"f": 10}),
    ]
}

#[test]
fn numeric_range_sort_order() {
    // The two-bucket numeric scan is DocumentId-sorted; sort must re-order it.
    assert_sort_order_matches_scan(
        &unsorted_numeric_docs(),
        "f",
        &json!({"f": {"$gte": 0, "$lt": 100}}),
        "f",
    );
}

#[test]
fn numeric_half_bounded_sort_order() {
    assert_sort_order_matches_scan(
        &unsorted_numeric_docs(),
        "f",
        &json!({"f": {"$gte": 0}}),
        "f",
    );
}

#[test]
fn string_range_sort_order_unaffected() {
    // Control: a single contiguous (index-ordered) string range still gets the
    // index-sort fast path and stays correctly ordered.
    let docs = vec![
        json!({"f": "delta"}),
        json!({"f": "alpha"}),
        json!({"f": "charlie"}),
        json!({"f": "bravo"}),
    ];
    assert_sort_order_matches_scan(&docs, "f", &json!({"f": {"$gte": "a", "$lte": "z"}}), "f");
}

#[test]
fn in_query_sort_order() {
    // $in is a multi-value (non-contiguous) scan → must also re-sort.
    assert_sort_order_matches_scan(
        &unsorted_numeric_docs(),
        "f",
        &json!({"f": {"$in": [10, 20, 30.5, 40, 50.0]}}),
        "f",
    );
}
