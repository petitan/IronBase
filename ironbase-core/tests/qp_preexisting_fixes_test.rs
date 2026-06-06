//! Pre-existing query-planner bugs surfaced by the 2026-06-06 code-review and
//! fixed in the follow-up:
//! - #10: mixed-type range bounds ({$gte: num, $lte: str}) over-counted the
//!   fully-covered count fast path (analyze_range_query_v2 lacked the
//!   value_type_rank guard the merge path has).
//! - #5: a multikey (array) index counted index ENTRIES, not distinct docs, on
//!   the fully-covered count fast path (range / $in / regex-prefix), so a doc
//!   with several matching array elements was over-counted vs find().

mod common;
use common::assert_index_matches_scan;
use serde_json::json;

// ---- #10: mixed-type range bounds ----

#[test]
fn mixed_type_range_numeric_lower_string_upper() {
    // {$gte:5, $lte:"z"} — Int(5) < String("z") is a valid-direction key range
    // spanning Int+Float+String, but operator semantics match nothing (cross-type
    // compare → no match). Correct answer: 0.
    let docs = vec![
        json!({"f": 5}),
        json!({"f": 10}),
        json!({"f": "a"}),
        json!({"f": "z"}),
    ];
    assert_index_matches_scan(&docs, "f", &json!({"f": {"$gte": 5, "$lte": "z"}}));
}

#[test]
fn same_type_numeric_range_still_indexed() {
    // Regression: a genuine same-type range must still work after the guard.
    let docs = (1..=10).map(|n| json!({ "f": n })).collect::<Vec<_>>();
    assert_index_matches_scan(&docs, "f", &json!({"f": {"$gte": 3, "$lte": 8}}));
}

#[test]
fn same_type_string_range_still_indexed() {
    let docs = vec![
        json!({"f": "apple"}),
        json!({"f": "banana"}),
        json!({"f": "cherry"}),
    ];
    assert_index_matches_scan(&docs, "f", &json!({"f": {"$gte": "a", "$lte": "c"}}));
}

// ---- #5: multikey fully-covered count must count distinct docs, not entries ----

#[test]
fn multikey_range_fully_covered() {
    // Doc with two array elements BOTH in range must count once (not twice).
    let docs = vec![
        json!({"tags": [20, 30]}),
        json!({"tags": [40]}),
        json!({"tags": [5]}),
    ];
    assert_index_matches_scan(&docs, "tags", &json!({"tags": {"$gte": 18, "$lte": 65}}));
}

#[test]
fn multikey_in_fully_covered() {
    // $in matching two array elements of one doc must count once.
    let docs = vec![json!({"tags": ["a", "b"]}), json!({"tags": ["c"]})];
    assert_index_matches_scan(&docs, "tags", &json!({"tags": {"$in": ["a", "b"]}}));
}

#[test]
fn multikey_cross_bucket_fully_covered() {
    // Mixed [int, float] array, both in range → Int bucket + Float bucket both
    // hit the same doc → must count once.
    let docs = vec![json!({"tags": [20, 30.5]}), json!({"tags": [40]})];
    assert_index_matches_scan(&docs, "tags", &json!({"tags": {"$gte": 18, "$lte": 65}}));
}

#[test]
fn multikey_regex_prefix_fully_covered() {
    // Two array string elements both match the prefix → must count once.
    let docs = vec![
        json!({"name": ["alpha", "apex"]}),
        json!({"name": ["beta"]}),
    ];
    assert_index_matches_scan(&docs, "name", &json!({"name": {"$regex": "^a"}}));
}
