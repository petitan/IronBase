// index_numeric_edge_tests.rs
// Indexed and unindexed queries agree on integers beyond 2^53 and on u64
// values above i64::MAX (audit 2026-10-07 Q15).

use ironbase_core::storage::MemoryStorage;
use ironbase_core::DatabaseCore;
use serde_json::{json, Value};
use std::collections::HashMap;

/// (matching `_id`s, count) with or without an index on `v`.
fn ids(indexed: bool, docs: &[Value], query: &Value) -> (Vec<i64>, u64) {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    for d in docs {
        let fields: HashMap<String, Value> = d.as_object().unwrap().clone().into_iter().collect();
        db.insert_one("c", fields).unwrap();
    }
    let coll = db.collection("c").unwrap();
    if indexed {
        coll.create_index("v".to_string(), false, false).unwrap();
    }
    let mut out: Vec<i64> = coll
        .find(query)
        .unwrap()
        .iter()
        .map(|d| d["_id"].as_i64().unwrap())
        .collect();
    out.sort();
    (out, coll.count_documents(query).unwrap())
}

fn assert_same(docs: &[Value], query: Value) {
    let scan = ids(false, docs, &query);
    let index = ids(true, docs, &query);
    assert_eq!(index, scan, "{query}: index vs scan");
}

#[test]
fn u64_beyond_i64_max_index_matches_scan() {
    let docs = [
        json!({"_id": 1, "v": u64::MAX}),
        json!({"_id": 2, "v": u64::MAX - 1}),
        json!({"_id": 3, "v": i64::MAX}),
        json!({"_id": 4, "v": 1}),
    ];
    assert_same(&docs, json!({"v": u64::MAX}));
    assert_same(&docs, json!({"v": {"$gt": i64::MAX}}));
    assert_same(&docs, json!({"v": {"$gte": u64::MAX - 1}}));
    assert_same(&docs, json!({"v": {"$lt": u64::MAX}}));
    assert_same(&docs, json!({"v": {"$in": [u64::MAX - 1, 1]}}));
}

#[test]
fn int_beyond_2_53_index_matches_scan() {
    let big = 9_007_199_254_740_993_i64; // 2^53 + 1
    let docs = [
        json!({"_id": 1, "v": big}),
        json!({"_id": 2, "v": big - 1}),
        json!({"_id": 3, "v": 9_007_199_254_740_992.0_f64}),
        json!({"_id": 4, "v": 9.1e15}),
    ];
    assert_same(&docs, json!({"v": big}));
    assert_same(&docs, json!({"v": {"$gt": big - 1}}));
    assert_same(&docs, json!({"v": {"$gte": 9_007_199_254_740_992.0_f64}}));
    assert_same(&docs, json!({"v": {"$lt": 9.1e15}}));
    assert_same(&docs, json!({"v": {"$lte": big}}));
}
