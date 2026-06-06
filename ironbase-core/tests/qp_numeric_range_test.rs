//! Query-planner correctness regression tests — PR5 (audit 2026-06-06, finding D).
//!
//! `IndexKey` orders ALL Int below ALL Float, so a single `[Int(a), Int(b)]`
//! range scan never reaches Float keys — but range operators compare
//! numerically. A field mixing integers and fractional/x.0 floats therefore
//! silently dropped matching docs from indexed find/count. The fix scans BOTH
//! the Int and Float key buckets.

mod common;
use common::assert_index_matches_scan;
use serde_json::{json, Value};

fn mixed_ages() -> Vec<Value> {
    vec![
        json!({"age": 20}),   // Int
        json!({"age": 30.5}), // Float (fractional)
        json!({"age": 40}),   // Int
        json!({"age": 50.0}), // Float (whole-number)
    ]
}

#[test]
fn bounded_range_spans_int_and_float() {
    // $gte:18 ∧ $lt:65 → all 4.
    assert_index_matches_scan(
        &mixed_ages(),
        "age",
        &json!({"age": {"$gte": 18, "$lt": 65}}),
    );
}

#[test]
fn inclusive_both_bounds() {
    assert_index_matches_scan(
        &mixed_ages(),
        "age",
        &json!({"age": {"$gte": 20, "$lte": 50.0}}),
    );
}

#[test]
fn exclusive_lower_bound() {
    // $gt:20 → 30.5, 40, 50.0 → 3.
    assert_index_matches_scan(&mixed_ages(), "age", &json!({"age": {"$gt": 20}}));
}

#[test]
fn exclusive_upper_float_bound() {
    // $lt:50.0 → 20, 30.5, 40 → 3.
    assert_index_matches_scan(&mixed_ages(), "age", &json!({"age": {"$lt": 50.0}}));
}

#[test]
fn half_bounded_float_lower() {
    // $gte:30.5 → 30.5, 40, 50.0 → 3.
    assert_index_matches_scan(&mixed_ages(), "age", &json!({"age": {"$gte": 30.5}}));
}

#[test]
fn int_upper_bound_does_not_drop_floats() {
    // $lt:40 → 20, 30.5 → 2 (the float 30.5 must not be dropped).
    assert_index_matches_scan(&mixed_ages(), "age", &json!({"age": {"$lt": 40}}));
}

#[test]
fn float_only_bounds() {
    // $gte:30.5 ∧ $lte:40.5 → 30.5, 40 → 2.
    assert_index_matches_scan(
        &mixed_ages(),
        "age",
        &json!({"age": {"$gte": 30.5, "$lte": 40.5}}),
    );
}

#[test]
fn whole_number_float_inclusive_with_int_bound() {
    // $lte:50 (Int bound) must still include the Float 50.0 → all 4.
    assert_index_matches_scan(&mixed_ages(), "age", &json!({"age": {"$lte": 50}}));
}

#[test]
fn negative_mixed_range() {
    let docs = vec![
        json!({"age": -5}),
        json!({"age": -2.5}),
        json!({"age": 0}),
        json!({"age": 3.5}),
    ];
    // $gte:-3 ∧ $lt:1 → -2.5, 0 → 2.
    assert_index_matches_scan(&docs, "age", &json!({"age": {"$gte": -3, "$lt": 1}}));
}

#[test]
fn pure_int_range_regression() {
    let docs = (1..=10).map(|n| json!({ "age": n })).collect::<Vec<_>>();
    assert_index_matches_scan(&docs, "age", &json!({"age": {"$gte": 3, "$lt": 8}}));
    assert_index_matches_scan(&docs, "age", &json!({"age": {"$gt": 3, "$lte": 8}}));
}

#[test]
fn pure_float_range_regression() {
    let docs = vec![
        json!({"age": 1.1}),
        json!({"age": 2.2}),
        json!({"age": 3.3}),
        json!({"age": 4.4}),
    ];
    assert_index_matches_scan(&docs, "age", &json!({"age": {"$gte": 2.0, "$lt": 4.0}}));
}

/// Many ranges over a richer mixed dataset — broad oracle sweep.
#[test]
fn sweep_mixed_dataset() {
    let docs = vec![
        json!({"age": 0}),
        json!({"age": 1.5}),
        json!({"age": 2}),
        json!({"age": 2.5}),
        json!({"age": 3}),
        json!({"age": 10.0}),
        json!({"age": 10}),
        json!({"age": 100.25}),
        json!({"age": -1}),
        json!({"age": -1.5}),
    ];
    let queries = [
        json!({"age": {"$gte": 0, "$lte": 10}}),
        json!({"age": {"$gt": 0, "$lt": 10}}),
        json!({"age": {"$gte": 2, "$lte": 2.5}}),
        json!({"age": {"$lt": 0}}),
        json!({"age": {"$gte": 10}}),
        json!({"age": {"$gt": 2.5, "$lt": 100.25}}),
        json!({"age": {"$gte": -1.5, "$lte": -1}}),
    ];
    for q in &queries {
        assert_index_matches_scan(&docs, "age", q);
    }
}

// --- Int bound precision beyond ±2^53 (review follow-up: f64 round-trip bug) ---

#[test]
fn integer_bound_beyond_2pow53() {
    let base: i64 = 1 << 53; // 2^53 = 9007199254740992
    let docs = (0..4)
        .map(|i| json!({ "age": base + i }))
        .collect::<Vec<_>>();
    // $gte: 2^53+1 → {2^53+1, 2^53+2, 2^53+3} = 3; must NOT include 2^53.
    assert_index_matches_scan(&docs, "age", &json!({"age": {"$gte": base + 1}}));
    assert_index_matches_scan(&docs, "age", &json!({"age": {"$lte": base + 1}}));
    assert_index_matches_scan(
        &docs,
        "age",
        &json!({"age": {"$gt": base + 1, "$lt": base + 3}}),
    );
    assert_index_matches_scan(&docs, "age", &json!({"age": {"$lt": base + 2}}));
}

// --- Float bound outside i64 range must collapse the Int bucket (saturation bug) ---

#[test]
fn float_bound_outside_i64_range() {
    let docs = vec![
        json!({"age": i64::MAX}),
        json!({"age": 5}),
        json!({"age": 2e30}),
    ];
    // $gte:1e30 matches only the Float 2e30 (1) — i64::MAX (≈9.2e18) is NOT ≥ 1e30.
    assert_index_matches_scan(&docs, "age", &json!({"age": {"$gte": 1e30}}));

    let docs2 = vec![
        json!({"age": i64::MIN}),
        json!({"age": -5}),
        json!({"age": -2e30}),
    ];
    // $lte:-1e30 matches only -2e30 (1) — i64::MIN is NOT ≤ -1e30.
    assert_index_matches_scan(&docs2, "age", &json!({"age": {"$lte": -1e30}}));
}

// --- Multikey [int, float] array must not double-count on the narrowed path ---

#[test]
fn multikey_cross_bucket_no_double_count_narrowed() {
    // Two-field query → NOT fully covered → index-narrowed path (scan_numeric_buckets).
    let docs = vec![
        json!({"age": [20, 30.5], "k": "x"}), // both elements in range → count ONCE
        json!({"age": [40], "k": "x"}),
        json!({"age": [5], "k": "x"}),
    ];
    assert_index_matches_scan(
        &docs,
        "age",
        &json!({"age": {"$gte": 18, "$lte": 65}, "k": "x"}),
    );
}
