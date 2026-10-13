// aggregation_numeric_unwind_tests.rs
// Audit 2026-10-07 A10 (1 vs 1.0 in $group/$addToSet/distinct), A11
// ($project integer arithmetic), A12 ($sum overflow), A13 ($unwind options).

use ironbase_core::storage::MemoryStorage;
use ironbase_core::DatabaseCore;
use serde_json::{json, Value};
use std::collections::HashMap;

fn db(docs: &[Value]) -> DatabaseCore<MemoryStorage> {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    for d in docs {
        let fields: HashMap<String, Value> = d.as_object().unwrap().clone().into_iter().collect();
        db.insert_one("c", fields).unwrap();
    }
    db
}

fn aggregate(docs: &[Value], pipeline: Value) -> Vec<Value> {
    db(docs)
        .collection("c")
        .unwrap()
        .aggregate(&pipeline)
        .unwrap()
}

/// A10: 1 and 1.0 are one group, one set element and one distinct value.
#[test]
fn equal_numbers_group_and_dedup_together() {
    let docs = [json!({"_id": 1, "v": 1}), json!({"_id": 2, "v": 1.0})];
    let groups = aggregate(&docs, json!([{"$group": {"_id": "$v", "n": {"$sum": 1}}}]));
    assert_eq!(groups.len(), 1, "{groups:?}");
    assert_eq!(groups[0]["n"], json!(2));

    let set = aggregate(
        &docs,
        json!([{"$group": {"_id": null, "s": {"$addToSet": "$v"}}}]),
    );
    assert_eq!(set[0]["s"].as_array().unwrap().len(), 1, "{set:?}");

    let distinct = db(&docs)
        .collection("c")
        .unwrap()
        .distinct("v", &json!({"_id": {"$gte": 0}}))
        .unwrap();
    assert_eq!(distinct.len(), 1, "{distinct:?}");
}

/// A11: integer arithmetic stays exact and integral.
#[test]
fn project_integer_arithmetic_is_exact() {
    let big = 9_007_199_254_740_993_i64; // 2^53 + 1
    let out = aggregate(
        &[json!({"_id": 1, "a": big, "b": 3, "c": 4})],
        json!([{"$project": {
            "sum": {"$add": ["$a", 1]},
            "small": {"$add": ["$b", "$c"]},
            "prod": {"$multiply": ["$b", "$c"]},
            "diff": {"$subtract": ["$a", "$b"]},
            "mixed": {"$add": ["$b", 0.5]}
        }}]),
    );
    assert_eq!(out[0]["sum"], json!(big + 1));
    assert_eq!(out[0]["small"], json!(7));
    assert!(out[0]["small"].is_i64());
    assert_eq!(out[0]["prod"], json!(12));
    assert_eq!(out[0]["diff"], json!(big - 3));
    assert_eq!(out[0]["mixed"], json!(3.5));
}

/// A12: an i64 overflow in $sum turns the sum into a double instead of
/// saturating.
#[test]
fn sum_overflow_promotes_to_double() {
    let out = aggregate(
        &[
            json!({"_id": 1, "v": i64::MAX}),
            json!({"_id": 2, "v": i64::MAX}),
        ],
        json!([{"$group": {"_id": null, "s": {"$sum": "$v"}}}]),
    );
    let s = out[0]["s"].as_f64().unwrap();
    assert!((s - 2.0 * i64::MAX as f64).abs() < 1e6, "{s}");
    assert!(!out[0]["s"].is_i64(), "{:?}", out[0]["s"]);
}

/// A13: preserveNullAndEmptyArrays and includeArrayIndex follow MongoDB.
#[test]
fn unwind_preserve_and_index_follow_mongodb() {
    let docs = [
        json!({"_id": 1, "a": []}),
        json!({"_id": 2}),
        json!({"_id": 3, "a": null}),
        json!({"_id": 4, "a": "x"}),
    ];
    let out = aggregate(
        &docs,
        json!([
            {"$unwind": {"path": "$a", "includeArrayIndex": "i", "preserveNullAndEmptyArrays": true}},
            {"$sort": {"_id": 1}}
        ]),
    );
    assert_eq!(
        out,
        vec![
            json!({"_id": 1, "i": null}),
            json!({"_id": 2, "i": null}),
            json!({"_id": 3, "a": null, "i": null}),
            json!({"_id": 4, "a": "x", "i": null}),
        ]
    );
}
