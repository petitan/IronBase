//! Property-based tests for the index / query planner consistency invariant.
//!
//! Core invariant under test: a query result must be identical regardless
//! of which execution strategy the planner chooses (collection scan vs.
//! single-field index vs. compound index vs. forced hint). The planner is
//! free to pick any strategy for performance reasons, but it MUST NOT
//! change the result set.
//!
//! Each property generates a random document collection and a random
//! filter, then runs the same query along two paths (e.g. before-index
//! vs. after-index, or default vs. hinted) and asserts equality of the
//! result-doc-id set. proptest minimizes failing inputs automatically.
//!
//! Coverage motivation: audits #27 / #28 (query operators + query planner)
//! turned up several IndexScan{Null} short-circuit bugs and equality-vs-
//! object filter mismatches. Per-input regression tests caught them, but
//! fuzzed property checks catch the next class — when the planner picks
//! a different strategy under a slight input change, the two strategies
//! must still agree.

use ironbase_core::{DatabaseCore, StorageEngine};
use proptest::prelude::*;
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap};
use tempfile::TempDir;

/// Insert a list of `(id, value)` pairs into a fresh collection on a
/// fresh DB. Returns the open DB so the caller can run queries.
fn fresh_db_with_v_docs(values: &[i64], db_label: &str) -> (TempDir, DatabaseCore<StorageEngine>) {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join(format!("{}.mlite", db_label));
    let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
    let _ = db.collection("c").unwrap();
    for (i, v) in values.iter().enumerate() {
        let mut d = HashMap::new();
        d.insert("_id".to_string(), json!(i as i64));
        d.insert("v".to_string(), json!(v));
        db.insert_one("c", d).unwrap();
    }
    (tmp, db)
}

/// Extract `_id` integers from a `Vec<Value>` query result into a sorted
/// set so two result sets can be compared regardless of order.
fn ids_of(docs: &[Value]) -> BTreeSet<i64> {
    docs.iter()
        .filter_map(|d| d.get("_id").and_then(|x| x.as_i64()))
        .collect()
}

// ============================================================================
// Property 1 — equality count must agree before and after creating an index
// ============================================================================
//
// Insert N random integers, count `{v == target}` first via collection scan
// (no index exists yet), then create a btree index on `v` and count again.
// Both must equal the ground-truth count computed in Rust.
proptest! {
    #![proptest_config(ProptestConfig::with_cases(40))]
    #[test]
    fn prop_count_index_vs_scan_equality(
        values in prop::collection::vec(0i64..50, 5..40),
        target in 0i64..50,
    ) {
        let (_tmp, db) = fresh_db_with_v_docs(&values, "p1");
        let coll = db.collection("c").unwrap();
        let filter = json!({"v": target});

        let scan_count = coll.count_documents(&filter).unwrap();

        coll.create_index("v".to_string(), false, false).unwrap();
        let index_count = coll.count_documents(&filter).unwrap();

        prop_assert_eq!(scan_count, index_count,
            "scan vs index disagree on equality count");

        let truth = values.iter().filter(|x| **x == target).count() as u64;
        prop_assert_eq!(scan_count, truth, "ground truth disagrees with scan count");
    }
}

// ============================================================================
// Property 2 — find result-doc-id set must match before and after indexing
// ============================================================================
//
// `count_documents` could in principle short-circuit to a wrong number while
// `find` returns the right docs, or vice versa. This property pins the
// stronger invariant: the SET of returned _ids matches.
proptest! {
    #![proptest_config(ProptestConfig::with_cases(40))]
    #[test]
    fn prop_find_doc_id_set_index_vs_scan(
        values in prop::collection::vec(0i64..30, 5..40),
        target in 0i64..30,
    ) {
        let (_tmp, db) = fresh_db_with_v_docs(&values, "p2");
        let coll = db.collection("c").unwrap();
        let filter = json!({"v": target});

        let scan_ids = ids_of(&coll.find(&filter).unwrap());

        coll.create_index("v".to_string(), false, false).unwrap();
        let index_ids = ids_of(&coll.find(&filter).unwrap());

        prop_assert_eq!(&scan_ids, &index_ids,
            "scan vs index disagree on find result _id set");

        // Cross-check against ground truth: every doc whose v == target,
        // identified by its insert order index.
        let truth: BTreeSet<i64> = values.iter().enumerate()
            .filter(|(_, v)| **v == target)
            .map(|(i, _)| i as i64)
            .collect();
        prop_assert_eq!(scan_ids, truth, "ground truth set disagrees with scan");
    }
}

// ============================================================================
// Property 3 — range query consistency (the historically buggy path)
// ============================================================================
//
// `IndexScan` for range queries went through several iterations (sparse
// array handling, $eq with object filter — audits #27 / #28). The
// invariant: `{$gte, $lte}` count via index equals collection scan count
// equals the inclusive Rust-side count.
proptest! {
    #![proptest_config(ProptestConfig::with_cases(40))]
    #[test]
    fn prop_range_query_index_vs_scan(
        values in prop::collection::vec(0i64..1000, 10..50),
        a in 0i64..1000,
        b in 0i64..1000,
    ) {
        let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
        let (_tmp, db) = fresh_db_with_v_docs(&values, "p3");
        let coll = db.collection("c").unwrap();
        let filter = json!({"v": {"$gte": lo, "$lte": hi}});

        let scan_count = coll.count_documents(&filter).unwrap();

        coll.create_index("v".to_string(), false, false).unwrap();
        let index_count = coll.count_documents(&filter).unwrap();

        prop_assert_eq!(scan_count, index_count,
            "range query: scan and index disagree");

        let truth = values.iter().filter(|x| **x >= lo && **x <= hi).count() as u64;
        prop_assert_eq!(scan_count, truth, "range ground truth disagrees");
    }
}

// ============================================================================
// Property 4 — explicit hint must not change the result set
// ============================================================================
//
// `find_with_hint(filter, "c_v")` forces the planner to use the named
// index. The result must be identical to the default `find(filter)` —
// hints can only steer execution, not change semantics.
proptest! {
    #![proptest_config(ProptestConfig::with_cases(30))]
    #[test]
    fn prop_find_with_hint_matches_default(
        values in prop::collection::vec(0i64..40, 5..30),
        target in 0i64..40,
    ) {
        let (_tmp, db) = fresh_db_with_v_docs(&values, "p4");
        let coll = db.collection("c").unwrap();
        coll.create_index("v".to_string(), false, false).unwrap();

        let filter = json!({"v": target});
        let default_ids = ids_of(&coll.find(&filter).unwrap());
        let hint_ids = ids_of(&coll.find_with_hint(&filter, "c_v").unwrap());

        prop_assert_eq!(default_ids, hint_ids,
            "default planner and hinted execution returned different _id sets");
    }
}

// ============================================================================
// Property 5 — compound-index prefix query equals scan
// ============================================================================
//
// MongoDB-compatible prefix rule: a compound index on `(a, b)` answers
// queries on `a` alone via a prefix scan. The result must match the
// collection-scan ground truth, even though the planner now picks a
// different IndexScan plan (compound vs. none).
proptest! {
    #![proptest_config(ProptestConfig::with_cases(30))]
    #[test]
    fn prop_compound_index_prefix_query(
        a_vals in prop::collection::vec(0i64..10, 8..30),
        b_vals in prop::collection::vec(0i64..10, 8..30),
        target_a in 0i64..10,
    ) {
        let n = a_vals.len().min(b_vals.len());
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("p5.mlite");
        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        let coll = db.collection("c").unwrap();
        for i in 0..n {
            let mut d = HashMap::new();
            d.insert("_id".to_string(), json!(i as i64));
            d.insert("a".to_string(), json!(a_vals[i]));
            d.insert("b".to_string(), json!(b_vals[i]));
            db.insert_one("c", d).unwrap();
        }

        let filter = json!({"a": target_a});
        let scan_count = coll.count_documents(&filter).unwrap();

        coll.create_compound_index(
            vec!["a".to_string(), "b".to_string()],
            false,
            false,
        ).unwrap();
        let index_count = coll.count_documents(&filter).unwrap();

        prop_assert_eq!(scan_count, index_count,
            "compound prefix scan disagrees with collection scan");

        let truth = a_vals.iter().take(n).filter(|x| **x == target_a).count() as u64;
        prop_assert_eq!(scan_count, truth, "compound prefix ground truth mismatch");
    }
}

// ============================================================================
// Property 6 — aggregate $match $count matches count_documents
// ============================================================================
//
// `count_documents` and `aggregate([$match, $count])` go through different
// stage code paths but answer the same logical question. Equality checks
// that the aggregation pipeline's `$match` filter semantics agree with
// the find/count operator semantics.
proptest! {
    #![proptest_config(ProptestConfig::with_cases(30))]
    #[test]
    fn prop_aggregate_match_count_matches_count_documents(
        values in prop::collection::vec(0i64..50, 5..30),
        target in 0i64..50,
    ) {
        let (_tmp, db) = fresh_db_with_v_docs(&values, "p6");
        let coll = db.collection("c").unwrap();
        coll.create_index("v".to_string(), false, false).unwrap();

        let filter = json!({"v": target});
        let cd = coll.count_documents(&filter).unwrap();

        let pipeline = json!([
            {"$match": {"v": target}},
            {"$count": "n"},
        ]);
        let result = coll.aggregate(&pipeline).unwrap();

        let agg = if result.is_empty() {
            0
        } else {
            result[0].get("n").and_then(|x| x.as_u64()).unwrap_or(0)
        };

        prop_assert_eq!(cd, agg,
            "count_documents and aggregate $match+$count disagree");
    }
}
