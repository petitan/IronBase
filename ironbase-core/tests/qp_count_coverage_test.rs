//! Query-planner correctness regression tests — PR2 (audit 2026-06-06, finding B).
//!
//! `count_documents` fully-covered fast path must not be taken when the single
//! indexed field carries an operator the chosen plan does not encode (residual
//! predicate). Otherwise count over-counts vs find/collection scan.

mod common;
use common::assert_index_matches_scan;
use serde_json::json;

#[test]
fn range_plus_ne_same_field() {
    let docs = (5..=9).map(|n| json!({ "f": n })).collect::<Vec<_>>();
    // $gte:5 AND $ne:7 → {5,6,8,9} → 4 (not 5).
    assert_index_matches_scan(&docs, "f", &json!({"f": {"$gte": 5, "$ne": 7}}));
}

#[test]
fn in_plus_ne_same_field() {
    let docs = (1..=3).map(|n| json!({ "f": n })).collect::<Vec<_>>();
    // $in:[1,2] AND $ne:1 → {2} → 1.
    assert_index_matches_scan(&docs, "f", &json!({"f": {"$in": [1, 2], "$ne": 1}}));
}

#[test]
fn range_plus_nin_same_field() {
    let docs = (5..=9).map(|n| json!({ "f": n })).collect::<Vec<_>>();
    // $gte:5 AND $nin:[7,8] → {5,6,9} → 3.
    assert_index_matches_scan(&docs, "f", &json!({"f": {"$gte": 5, "$nin": [7, 8]}}));
}

#[test]
fn in_plus_range_same_field() {
    let docs = ["a", "b", "c", "d"]
        .iter()
        .map(|s| json!({ "status": s }))
        .collect::<Vec<_>>();
    // $in:[b,d] AND $gte:b → {b,d} → 2.
    assert_index_matches_scan(
        &docs,
        "status",
        &json!({"status": {"$in": ["b", "d"], "$gte": "b"}}),
    );
}

#[test]
fn range_plus_type_same_field() {
    let docs = vec![
        json!({"f": 5}),
        json!({"f": 6}),
        json!({"f": "hello"}),
        json!({"f": "world"}),
        json!({"f": 7}),
    ];
    // $gte:5 AND $type:"string" — residual $type must be applied.
    assert_index_matches_scan(&docs, "f", &json!({"f": {"$gte": 5, "$type": "string"}}));
}

#[test]
fn and_in_plus_nin_same_field() {
    let docs = ["a", "b", "c"]
        .iter()
        .map(|s| json!({ "status": s }))
        .collect::<Vec<_>>();
    // $in:[a,b,c] AND $nin:[b] → {a,c} → 2.
    assert_index_matches_scan(
        &docs,
        "status",
        &json!({"$and": [
            {"status": {"$in": ["a", "b", "c"]}},
            {"status": {"$nin": ["b"]}}
        ]}),
    );
}

// --- Pure (fully-covered) cases must remain correct ---

#[test]
fn pure_range_still_correct() {
    let docs = (1..=10).map(|n| json!({ "f": n })).collect::<Vec<_>>();
    assert_index_matches_scan(&docs, "f", &json!({"f": {"$gte": 5}}));
    assert_index_matches_scan(&docs, "f", &json!({"f": {"$gte": 3, "$lt": 8}}));
}

#[test]
fn pure_equality_and_in_still_correct() {
    let docs = (1..=10).map(|n| json!({ "f": n })).collect::<Vec<_>>();
    assert_index_matches_scan(&docs, "f", &json!({"f": 5}));
    assert_index_matches_scan(&docs, "f", &json!({"f": {"$eq": 5}}));
    assert_index_matches_scan(&docs, "f", &json!({"f": {"$in": [2, 4, 6]}}));
}
