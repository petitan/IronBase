// index_semantics_tests.rs
// Sort, range, distinct and $group through a B+ tree index must give the same
// result as without the index (audit 2026-10-07 Q1, Q12, Q13, Q14, A4): an
// index does not hold null / missing / object values, holds an array once per
// element (multikey), and orders every Int key before every Float key.

use ironbase_core::find_options::FindOptions;
use ironbase_core::storage::MemoryStorage;
use ironbase_core::DatabaseCore;
use serde_json::{json, Value};
use std::collections::HashMap;

fn db_with(docs: &[Value], index: Option<&str>) -> DatabaseCore<MemoryStorage> {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    for d in docs {
        let f: HashMap<String, Value> = d.as_object().unwrap().clone().into_iter().collect();
        db.insert_one("c", f).unwrap();
    }
    if let Some(field) = index {
        db.collection("c")
            .unwrap()
            .create_index(field.to_string(), false, false)
            .unwrap();
    }
    db
}

fn tags(docs: &[Value]) -> Vec<String> {
    docs.iter()
        .map(|d| d["n"].as_str().unwrap().to_string())
        .collect()
}

/// Run `f` without and with an index on `field`; both must agree.
fn same<T: PartialEq + std::fmt::Debug>(
    docs: &[Value],
    field: &str,
    f: impl Fn(&DatabaseCore<MemoryStorage>) -> T,
) -> T {
    let plain = f(&db_with(docs, None));
    let indexed = f(&db_with(docs, Some(field)));
    assert_eq!(indexed, plain, "indexed result differs from the scan");
    plain
}

fn sorted_find(
    db: &DatabaseCore<MemoryStorage>,
    q: Value,
    dir: i32,
    skip: usize,
    limit: usize,
) -> Vec<String> {
    let opts = FindOptions::new()
        .with_sort(vec![("v".to_string(), dir)])
        .with_skip(skip)
        .with_limit(limit);
    tags(
        &db.collection("c")
            .unwrap()
            .find_with_options(&q, opts)
            .unwrap(),
    )
}

fn mixed_docs() -> Vec<Value> {
    vec![
        json!({"n": "i10", "v": 10}),
        json!({"n": "f1.5", "v": 1.5}),
        json!({"n": "i2", "v": 2}),
        json!({"n": "null", "v": null}),
        json!({"n": "missing"}),
        json!({"n": "s", "v": "abc"}),
        json!({"n": "obj", "v": {"x": 1}}),
        json!({"n": "f20.5", "v": 20.5}),
    ]
}

#[test]
fn q1_empty_filter_sort_matches_memory_sort() {
    for dir in [1, -1] {
        for (skip, limit) in [(0, 100), (0, 3), (2, 3)] {
            same(&mixed_docs(), "v", |db| {
                sorted_find(db, json!({}), dir, skip, limit)
            });
        }
    }
    // homogeneous numbers, still Int and Float mixed
    let nums: Vec<Value> = [3.5f64, 1.0, 2.0, 10.0, 0.5]
        .iter()
        .enumerate()
        .map(|(i, x)| {
            if x.fract() == 0.0 {
                json!({"n": format!("d{i}"), "v": *x as i64})
            } else {
                json!({"n": format!("d{i}"), "v": x})
            }
        })
        .collect();
    same(&nums, "v", |db| sorted_find(db, json!({}), 1, 0, 3));
}

#[test]
fn q1_q14_multikey_sort_matches_memory_sort() {
    let docs = vec![
        json!({"n": "a", "v": [5, 1]}),
        json!({"n": "b", "v": [3]}),
        json!({"n": "c", "v": 4}),
        json!({"n": "d", "v": [2, 9]}),
    ];
    for dir in [1, -1] {
        same(&docs, "v", |db| sorted_find(db, json!({}), dir, 0, 100));
        same(&docs, "v", |db| {
            sorted_find(db, json!({"v": {"$gte": 2}}), dir, 0, 100)
        });
        same(&docs, "v", |db| {
            sorted_find(db, json!({"v": {"$gte": 2}}), dir, 1, 2)
        });
    }
}

#[test]
fn q12_multikey_two_sided_range_keeps_documents() {
    let docs = vec![
        json!({"n": "split", "v": [3, 12]}),
        json!({"n": "inside", "v": [7]}),
        json!({"n": "out", "v": [1, 2]}),
    ];
    let q = json!({"v": {"$gt": 5, "$lt": 10}});
    let mut found = same(&docs, "v", |db| {
        let coll = db.collection("c").unwrap();
        let mut t = tags(&coll.find(&q).unwrap());
        t.sort();
        (t, coll.count_documents(&q).unwrap())
    });
    found.0.sort();
    assert_eq!(found.0, vec!["inside", "split"]);
}

#[test]
fn q13_indexed_distinct_matches_scan() {
    for docs in [
        mixed_docs(),
        vec![json!({"n": "a", "v": [1, 2]}), json!({"n": "b", "v": 1})],
        vec![json!({"n": "a", "v": 1}), json!({"n": "b", "v": 2})],
    ] {
        same(&docs, "v", |db| {
            let mut vals: Vec<String> = db
                .collection("c")
                .unwrap()
                .distinct("v", &json!({}))
                .unwrap()
                .iter()
                .map(|v| v.to_string())
                .collect();
            vals.sort();
            vals
        });
    }
}

#[test]
fn a4_indexed_group_count_matches_scan() {
    for docs in [
        mixed_docs(),
        vec![json!({"n": "a", "v": [1, 2]}), json!({"n": "b", "v": 1})],
        vec![
            json!({"n": "a", "v": 1}),
            json!({"n": "b", "v": 1}),
            json!({"n": "c", "v": 2}),
        ],
    ] {
        same(&docs, "v", |db| {
            let mut out: Vec<String> = db
                .collection("c")
                .unwrap()
                .aggregate(&json!([{"$group": {"_id": "$v", "count": {"$sum": 1}}}]))
                .unwrap()
                .iter()
                .map(|d| format!("{}={}", d["_id"], d["count"]))
                .collect();
            out.sort();
            out
        });
    }
}

/// The cached "index covers every document" fact is recomputed after a write:
/// a document without the field, inserted after an index-sorted query, still
/// shows up in the next one.
#[test]
fn q1_index_sort_fact_follows_writes() {
    let docs = vec![
        json!({"n": "a", "v": 1}),
        json!({"n": "b", "v": 2}),
        json!({"n": "c", "v": 3}),
    ];
    let db = db_with(&docs, Some("v"));
    assert_eq!(sorted_find(&db, json!({}), 1, 0, 2), vec!["a", "b"]);
    let f: HashMap<String, Value> = json!({"n": "missing"})
        .as_object()
        .unwrap()
        .clone()
        .into_iter()
        .collect();
    db.insert_one("c", f).unwrap();
    assert_eq!(sorted_find(&db, json!({}), 1, 0, 2), vec!["missing", "a"]);
    db.delete_one("c", &json!({"n": "missing"})).unwrap();
    assert_eq!(sorted_find(&db, json!({}), 1, 0, 2), vec!["a", "b"]);
}
