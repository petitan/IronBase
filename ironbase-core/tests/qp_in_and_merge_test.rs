//! Query-planner correctness regression tests — PR1 (audit 2026-06-06).
//!
//! Covers:
//! - A: `$in` with an object/array element collapsing to IndexKey::Null
//!   (collect_in_candidates was missing the audit #27-A is_indexable_value guard).
//! - C: `$and` of two same-field membership operators ($in/$ne/$nin) where the
//!   clause merge silently overwrote the first (resolve_range_conditions).
//! - G: `$in` with duplicate values double-counted by the fully-covered count path.

mod common;
use common::assert_index_matches_scan;
use serde_json::json;

// ----- A: $in with non-indexable (object/array) elements -----

#[test]
fn in_with_object_element_matches_scan() {
    let docs = vec![
        json!({"meta": {"a": 1}}),
        json!({"meta": {"a": 2}}),
        json!({"meta": "plain"}),
    ];
    // Correct answer: 1 (only {"a":1}).
    assert_index_matches_scan(&docs, "meta", &json!({"meta": {"$in": [{"a": 1}]}}));
}

#[test]
fn in_with_mixed_scalar_and_object_element_matches_scan() {
    let docs = vec![
        json!({"meta": {"a": 1}}),
        json!({"meta": "x"}),
        json!({"meta": "y"}),
    ];
    // Object element mixed with a scalar element: must still fall back to scan.
    assert_index_matches_scan(&docs, "meta", &json!({"meta": {"$in": ["x", {"a": 1}]}}));
}

// ----- G: $in with duplicate values -----

#[test]
fn in_with_duplicate_values_not_double_counted() {
    let docs = vec![
        json!({"status": "active"}),
        json!({"status": "active"}),
        json!({"status": "active"}),
        json!({"status": "inactive"}),
    ];
    // Correct answer: 3 (each active doc counted once), not 6.
    assert_index_matches_scan(
        &docs,
        "status",
        &json!({"status": {"$in": ["active", "active"]}}),
    );
}

#[test]
fn in_with_duplicate_and_distinct_values() {
    let docs = vec![
        json!({"status": "a"}),
        json!({"status": "b"}),
        json!({"status": "b"}),
        json!({"status": "c"}),
    ];
    assert_index_matches_scan(
        &docs,
        "status",
        &json!({"status": {"$in": ["a", "b", "b", "a"]}}),
    );
}

// ----- C: $and clause-merge collisions -----

#[test]
fn and_two_in_same_field_intersects() {
    let docs = vec![
        json!({"status": "a"}),
        json!({"status": "b"}),
        json!({"status": "c"}),
    ];
    // Intersection of {a,b} and {b,c} is {b} → 1.
    assert_index_matches_scan(
        &docs,
        "status",
        &json!({"$and": [
            {"status": {"$in": ["a", "b"]}},
            {"status": {"$in": ["b", "c"]}}
        ]}),
    );
}

#[test]
fn and_two_ne_same_field() {
    let docs = vec![
        json!({"status": "a"}),
        json!({"status": "b"}),
        json!({"status": "c"}),
    ];
    // status != a AND status != b → only c → 1.
    assert_index_matches_scan(
        &docs,
        "status",
        &json!({"$and": [
            {"status": {"$ne": "a"}},
            {"status": {"$ne": "b"}}
        ]}),
    );
}
