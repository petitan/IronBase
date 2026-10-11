// aggregation_limit_skip_tests.rs
// $skip/$limit and the document limit (audit 2026-10-07 A5, A6) and the
// document count of a $group behind another stage (A7), on both execution
// paths: the context path used by `aggregate*` and the public
// `Pipeline::execute_streaming_with_limits`.

use ironbase_core::{AggregationLimitContext, AggregationLimits, Pipeline};
use serde_json::{json, Value};

const DOC_LIMIT: usize = 20;

fn limits() -> AggregationLimits {
    AggregationLimits {
        max_docs_without_match: DOC_LIMIT,
        max_docs_with_match: DOC_LIMIT,
        ..Default::default()
    }
}

fn docs(n: usize) -> Vec<Value> {
    (0..n)
        .map(|i| json!({"i": i, "even": i % 2 == 0, "tags": [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]}))
        .collect()
}

/// Runs `pipeline` over `n` documents on both execution paths.
fn run_both(n: usize, pipeline: Value) -> [ironbase_core::Result<Vec<Value>>; 2] {
    let p = Pipeline::from_json(&pipeline).unwrap();
    let ctx = AggregationLimitContext::new(limits());
    let with_context = p.execute_with_context(docs(n).into_iter().map(Ok), &ctx);
    let streaming = p.execute_streaming_with_limits(docs(n).into_iter().map(Ok), limits());
    [with_context, streaming]
}

fn ids(result: &ironbase_core::Result<Vec<Value>>) -> Vec<u64> {
    match result {
        Ok(docs) => docs.iter().map(|d| d["i"].as_u64().unwrap()).collect(),
        Err(e) => panic!("unexpected error: {e}"),
    }
}

fn assert_limit_error(result: &ironbase_core::Result<Vec<Value>>, what: &str) {
    match result {
        Ok(docs) => panic!(
            "{what}: expected a document-limit error, got {} documents",
            docs.len()
        ),
        Err(e) => assert!(
            e.to_string().contains("document limit"),
            "{what}: unexpected error: {e}"
        ),
    }
}

/// A5: within the limit the result is exact.
#[test]
fn skip_limit_within_doc_limit_is_exact() {
    for r in &run_both(50, json!([{"$skip": 5}, {"$limit": 3}])) {
        assert_eq!(ids(r), vec![5, 6, 7]);
    }
}

/// A5: needing more documents than the limit is an error, not a silently
/// shorter (or empty) result.
#[test]
fn skip_limit_beyond_doc_limit_errors_instead_of_truncating() {
    for (k, r) in run_both(50, json!([{"$skip": 5}, {"$limit": 30}]))
        .iter()
        .enumerate()
    {
        assert_limit_error(r, &format!("path {k}: skip 5 + limit 30"));
    }
    for (k, r) in run_both(50, json!([{"$skip": 25}, {"$limit": 3}]))
        .iter()
        .enumerate()
    {
        assert_limit_error(r, &format!("path {k}: skip 25 + limit 3"));
    }
}

/// A6: a $match/$project between $skip and $limit must not turn skip + limit
/// into a document limit.
#[test]
fn skip_match_limit_reads_past_skip_plus_limit() {
    for pipeline in [
        json!([{"$skip": 2}, {"$match": {"even": true}}, {"$limit": 5}]),
        json!([{"$skip": 2}, {"$project": {"i": 1, "even": 1}}, {"$match": {"even": true}}, {"$limit": 5}]),
    ] {
        for r in &run_both(50, pipeline.clone()) {
            assert_eq!(ids(r), vec![2, 4, 6, 8, 10], "{pipeline}");
        }
    }
}

/// A7: a $group behind $sort or $unwind must count each source document
/// once, not again as its own input.
#[test]
fn group_behind_sort_or_unwind_counts_source_documents_once() {
    // 15 source documents, limit 20: counted twice they exceeded it
    for r in &run_both(
        15,
        json!([{"$sort": {"i": -1}}, {"$group": {"_id": null, "n": {"$sum": 1}}}]),
    ) {
        let docs = r.as_ref().expect("$sort then $group");
        assert_eq!(docs[0]["n"], json!(15));
    }
    // 5 source documents unwound into 50: the unwound documents are not
    // source documents
    for r in &run_both(
        5,
        json!([{"$unwind": "$tags"}, {"$group": {"_id": null, "n": {"$sum": 1}}}]),
    ) {
        let docs = r.as_ref().expect("$unwind then $group");
        assert_eq!(docs[0]["n"], json!(50));
    }
}
