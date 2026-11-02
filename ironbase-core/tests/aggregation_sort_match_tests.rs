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

/// A3: a multi-key $sort applies its keys in the order written, not in
/// alphabetical order (serde_json `preserve_order`).
#[test]
fn multi_key_sort_keeps_key_order() {
    let db = db_with(&[
        json!({"n": 1, "a": 2, "b": 1}),
        json!({"n": 2, "a": 1, "b": 2}),
        json!({"n": 3, "a": 1, "b": 1}),
    ]);
    let c = db.collection("c").unwrap();
    let pipeline: Value = serde_json::from_str(r#"[{"$sort": {"b": 1, "a": 1}}]"#).unwrap();
    let full = c.aggregate(&pipeline).unwrap();
    assert_eq!(values(&full, "n"), vec![json!(3), json!(1), json!(2)]);
    let pipeline: Value =
        serde_json::from_str(r#"[{"$sort": {"b": 1, "a": 1}}, {"$limit": 2}]"#).unwrap();
    let top = c.aggregate(&pipeline).unwrap();
    assert_eq!(values(&top, "n"), vec![json!(3), json!(1)]);
}

/// With `preserve_order`, `$unset` of a nested field keeps the order of the
/// remaining fields (`Map::remove` would be a swap_remove).
#[test]
fn unset_keeps_nested_field_order() {
    let db =
        db_with(&[serde_json::from_str(r#"{"_id": 1, "o": {"a": 1, "b": 2, "c": 3}}"#).unwrap()]);
    db.update_one("c", &json!({"_id": 1}), &json!({"$unset": {"o.a": ""}}))
        .unwrap();
    let doc = db
        .collection("c")
        .unwrap()
        .find(&json!({"_id": 1}))
        .unwrap()
        .remove(0);
    let keys: Vec<&String> = doc["o"].as_object().unwrap().keys().collect();
    assert_eq!(keys, vec!["b", "c"]);
}

/// #127: nulls last with MongoDB's own idiom, a computed sort key from
/// `$addFields` + `$cond` / `$ifNull`
#[test]
fn add_fields_cond_if_null_sort_nulls_last() {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    for doc in [
        json!({"n": 1, "date": "2026-07-06"}),
        json!({"n": 2, "date": null}),
        json!({"n": 3}),
        json!({"n": 4, "date": "2026-01-01"}),
    ] {
        let map: HashMap<String, serde_json::Value> = serde_json::from_value(doc).unwrap();
        db.insert_one("r", map).unwrap();
    }
    let coll = db.collection("r").unwrap();

    for stage in ["$addFields", "$set"] {
        let out = coll
            .aggregate(&json!([
                { stage: {"_r": {"$cond": [{"$ifNull": ["$date", false]}, 0, 1]}} },
                {"$sort": {"_r": 1, "date": 1}},
                {"$project": {"_id": 0, "n": 1}}
            ]))
            .unwrap();
        let order: Vec<i64> = out.iter().map(|d| d["n"].as_i64().unwrap()).collect();
        // Dated documents ascending first, the null and the missing date
        // last (missing sorts before null)
        assert_eq!(order, vec![4, 1, 3, 2], "{stage}");
    }

    // $addFields keeps every field; literals, $literal, field copies,
    // dotted paths, the object form of $cond and truthiness
    let out = coll
        .aggregate(&json!([
            {"$match": {"n": 3}},
            {"$addFields": {
                "one": 1,
                "lit": {"$literal": "$n"},
                "copy": "$n",
                "gone": "$missing",
                "n": "$also_missing",
                "meta.d": {"$ifNull": ["$date", "$missing", "none"]},
                "big": {"$cond": {"if": "$n", "then": "yes", "else": "no"}},
                "zero": {"$cond": [0, "yes", "no"]},
                "empty": {"$cond": ["", "yes", "no"]}
            }},
            {"$project": {"_id": 0}}
        ]))
        .unwrap();
    assert_eq!(
        out,
        vec![
            // "n" from a missing source is gone; the others read the input's n
            json!({"one": 1, "lit": "$n", "copy": 3, "meta": {"d": "none"},
                    "big": "yes", "zero": "no", "empty": "yes"})
        ]
    );

    for bad in [
        json!([{"$addFields": {}}]),
        json!([{"$addFields": {"$x": 1}}]),
        json!([{"$addFields": {"c": {"$cond": [1, 2]}}}]),
        json!([{"$addFields": {"c": {"$ifNull": ["$a"]}}}]),
        json!([{"$addFields": {"a": 1, "a.b": 2}}]),
    ] {
        assert!(coll.aggregate(&bad).is_err(), "accepted {bad}");
    }
}
