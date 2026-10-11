// aggregation_memory_limit_tests.rs
// AggregationLimits::max_memory_mb is enforced on the documents an
// aggregation materializes (audit 2026-10-07 M3: only document counts were).

use ironbase_core::aggregation::AggregationLimits;
use ironbase_core::storage::MemoryStorage;
use ironbase_core::DatabaseCore;
use serde_json::{json, Value};
use std::collections::HashMap;

fn db_with_big_docs(n: usize) -> DatabaseCore<MemoryStorage> {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    let body = "x".repeat(20_000);
    for i in 0..n {
        let d: HashMap<String, Value> = json!({"i": i, "body": body, "tags": [1, 2, 3, 4]})
            .as_object()
            .unwrap()
            .clone()
            .into_iter()
            .collect();
        db.insert_one("c", d).unwrap();
    }
    db
}

fn limits_mb(mb: usize) -> AggregationLimits {
    AggregationLimits {
        max_memory_mb: mb,
        ..AggregationLimits::unlimited()
    }
}

/// ~4 MB of documents against a 1 MB budget: every materializing pipeline
/// shape fails with a memory-limit error instead of holding them all.
#[test]
fn max_memory_mb_is_enforced() {
    let db = db_with_big_docs(200);
    let c = db.collection("c").unwrap();
    for pipeline in [
        json!([{"$project": {"body": 1}}]),
        json!([{"$sort": {"i": -1}}]),
        json!([{"$unwind": "$tags"}]),
    ] {
        let err = match c.aggregate_with_limits(&pipeline, limits_mb(1)) {
            Ok(docs) => panic!("{pipeline} held {} docs past a 1 MB limit", docs.len()),
            Err(e) => e,
        };
        assert!(
            err.to_string().contains("memory limit"),
            "{pipeline}: unexpected error {err}"
        );
    }
}

/// Within the budget, or when the documents are not materialized ($group
/// streams them), the same data aggregates normally.
#[test]
fn within_budget_and_streaming_group_still_work() {
    let db = db_with_big_docs(200);
    let c = db.collection("c").unwrap();
    assert_eq!(
        c.aggregate_with_limits(&json!([{"$sort": {"i": -1}}]), limits_mb(64))
            .unwrap()
            .len(),
        200
    );
    let grouped = c
        .aggregate_with_limits(
            &json!([{"$group": {"_id": null, "n": {"$sum": 1}}}]),
            limits_mb(1),
        )
        .unwrap();
    assert_eq!(grouped[0]["n"], json!(200));
    let limited = c
        .aggregate_with_limits(&json!([{"$limit": 5}]), limits_mb(1))
        .unwrap();
    assert_eq!(limited.len(), 5);
}

/// The `Pipeline` API on in-memory documents enforces the same budget.
#[test]
fn pipeline_api_enforces_max_memory_mb() {
    use ironbase_core::aggregation::Pipeline;
    let body = "x".repeat(20_000);
    let docs: Vec<Value> = (0..200).map(|i| json!({"i": i, "body": body})).collect();
    let pipeline = Pipeline::from_json(&json!([{"$project": {"body": 1}}])).unwrap();
    let err = pipeline
        .execute_streaming_with_limits(docs.clone().into_iter().map(Ok), limits_mb(1))
        .unwrap_err();
    assert!(err.to_string().contains("memory limit"), "{err}");
    let ok = pipeline
        .execute_streaming_with_limits(docs.into_iter().map(Ok), limits_mb(64))
        .unwrap();
    assert_eq!(ok.len(), 200);
}
