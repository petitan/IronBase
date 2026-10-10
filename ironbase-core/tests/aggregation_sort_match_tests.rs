// aggregation_sort_match_tests.rs
// $sort on mixed types and $match after $group (audit 2026-10-07 A1, A2).

use ironbase_core::storage::MemoryStorage;
use ironbase_core::DatabaseCore;
use serde_json::{json, Value};
use std::collections::HashMap;

fn db_with(docs: &[Value]) -> DatabaseCore<MemoryStorage> {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    for d in docs {
        let f: HashMap<String, Value> = d.as_object().unwrap().clone().into_iter().collect();
        db.insert_one("c", f).unwrap();
    }
    db
}

fn values(docs: &[Value], field: &str) -> Vec<Value> {
    docs.iter().map(|d| d[field].clone()).collect()
}

/// A2: mixed types sort in one total order (missing < null < numbers <
/// strings < bools < objects < arrays, the order `find` uses) instead of
/// panicking, with and without the top-k path; objects compare field by field.
#[test]
fn sort_mixed_types_is_a_total_order() {
    let mut docs = Vec::new();
    for i in 0..40 {
        docs.push(json!({"n": i, "v": i}));
        docs.push(json!({"n": i, "v": format!("s{i:02}")}));
        docs.push(json!({"n": i, "v": i as f64 + 0.5}));
    }
    docs.push(json!({"n": 100, "v": null}));
    docs.push(json!({"n": 101}));
    docs.push(json!({"n": 102, "v": true}));
    docs.push(json!({"n": 103, "v": {"a": 2}}));
    docs.push(json!({"n": 104, "v": {"a": 1, "b": 9}}));
    docs.push(json!({"n": 105, "v": [1]}));
    let db = db_with(&docs);
    let c = db.collection("c").unwrap();

    let all = c
        .aggregate(&json!([{"$sort": {"v": 1}}, {"$project": {"_id": 0, "n": 1, "v": 1}}]))
        .unwrap();
    assert_eq!(all.len(), docs.len());
    let v = values(&all, "v");
    assert_eq!(all[0]["n"], json!(101)); // missing
    assert_eq!(v[1], Value::Null);
    assert_eq!(v[2], json!(0));
    assert_eq!(v[3], json!(0.5));
    assert_eq!(v[81], json!(39.5));
    assert_eq!(v[82], json!("s00"));
    let tail: Vec<Value> = v[v.len() - 4..].to_vec();
    assert_eq!(
        tail,
        vec![
            json!(true),
            json!({"a": 1, "b": 9}),
            json!({"a": 2}),
            json!([1])
        ]
    );

    // Top-k agrees with the full sort, ascending and descending
    for dir in [1, -1] {
        let full = c
            .aggregate(&json!([{"$sort": {"v": dir, "n": 1}}]))
            .unwrap();
        let top = c
            .aggregate(&json!([{"$sort": {"v": dir, "n": 1}}, {"$limit": 7}]))
            .unwrap();
        assert_eq!(top, full[..7].to_vec(), "dir {dir}");
    }
}

/// A1: $match after $group matches on any `_id` type (object, null, float,
/// bool) and a document without `_id` does not match a fake `_id: 0`.
#[test]
fn match_after_group_on_any_id_type() {
    let db = db_with(&[
        json!({"y": 2024, "t": "a", "q": 1}),
        json!({"y": 2024, "t": "b", "q": 2}),
        json!({"t": "c", "q": 3}),
        json!({"y": 1.5, "t": "d", "q": 4}),
    ]);
    let c = db.collection("c").unwrap();

    let by_obj = c
        .aggregate(&json!([
            {"$group": {"_id": {"y": "$y", "t": "$t"}, "s": {"$sum": "$q"}}},
            {"$match": {"_id.t": "b"}}
        ]))
        .unwrap();
    assert_eq!(values(&by_obj, "s"), vec![json!(2)]);

    let null_group = c
        .aggregate(&json!([
            {"$group": {"_id": "$y", "s": {"$sum": "$q"}}},
            {"$match": {"_id": null}}
        ]))
        .unwrap();
    assert_eq!(values(&null_group, "s"), vec![json!(3)]);

    let float_group = c
        .aggregate(&json!([
            {"$group": {"_id": "$y", "s": {"$sum": "$q"}}},
            {"$match": {"_id": {"$lt": 2000}}}
        ]))
        .unwrap();
    assert_eq!(values(&float_group, "s"), vec![json!(4)]);

    // $project removed _id: there is no _id to match
    let no_id = c
        .aggregate(&json!([
            {"$project": {"_id": 0, "q": 1}},
            {"$match": {"_id": 0}}
        ]))
        .unwrap();
    assert!(no_id.is_empty(), "{no_id:?}");
}
