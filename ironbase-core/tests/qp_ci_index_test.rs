//! Query-planner correctness regression tests — PR3 (audit 2026-06-06, finding E).
//!
//! A case-insensitive index stores keys lowercased. A case-sensitive query
//! (equality / range / non-(?i) regex / $in) must NOT be planned against it,
//! or the index scan misses documents. After the fix such queries fall back to
//! the collection scan; only `(?i)` regex uses the CI index.

mod common;
use common::assert_ci_index_matches_scan;
use serde_json::json;

fn name_docs() -> Vec<serde_json::Value> {
    vec![
        json!({"name": "Alice"}),
        json!({"name": "alice"}),
        json!({"name": "Bob"}),
        json!({"name": "albert"}),
    ]
}

#[test]
fn cs_equality_on_ci_index_matches_scan() {
    // {name:"Alice"} is exact (case-sensitive) → only "Alice" → 1.
    assert_ci_index_matches_scan(&name_docs(), "name", &json!({"name": "Alice"}));
}

#[test]
fn cs_eq_operator_on_ci_index_matches_scan() {
    assert_ci_index_matches_scan(&name_docs(), "name", &json!({"name": {"$eq": "alice"}}));
}

#[test]
fn cs_regex_prefix_on_ci_index_matches_scan() {
    // ^Al (case-sensitive) → only "Alice" → 1.
    assert_ci_index_matches_scan(&name_docs(), "name", &json!({"name": {"$regex": "^Al"}}));
}

#[test]
fn cs_range_on_ci_index_matches_scan() {
    assert_ci_index_matches_scan(&name_docs(), "name", &json!({"name": {"$gte": "M"}}));
}

#[test]
fn cs_in_on_ci_index_matches_scan() {
    assert_ci_index_matches_scan(
        &name_docs(),
        "name",
        &json!({"name": {"$in": ["Alice", "Bob"]}}),
    );
}

#[test]
fn ci_regex_on_ci_index_still_correct() {
    // (?i)^al → "Alice","alice","albert" → 3. The CI index IS used here and must
    // still agree with the scan.
    assert_ci_index_matches_scan(
        &name_docs(),
        "name",
        &json!({"name": {"$regex": "(?i)^al"}}),
    );
}
