// find_paths_consistency_tests.rs
// find / find_one / count_documents must agree for the same query, whatever
// path executes it (audit 2026-10-07 Q5-Q9, Q11):
// - `_id` fast paths (equality, `$in`) honour skip/limit and never duplicate;
// - find_one works for `_id` operator queries;
// - `$and` / `$or` per-clause paths apply skip/limit to the final result;
// - limit 0 means "no limit" on every path.

use ironbase_core::find_options::FindOptions;
use ironbase_core::storage::MemoryStorage;
use ironbase_core::DatabaseCore;
use serde_json::{json, Value};
use std::collections::HashMap;

fn insert(db: &DatabaseCore<MemoryStorage>, d: Value) {
    let fields: HashMap<String, Value> = d.as_object().unwrap().clone().into_iter().collect();
    db.insert_one("c", fields).unwrap();
}

fn db_with(docs: Vec<Value>) -> DatabaseCore<MemoryStorage> {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    for d in docs {
        insert(&db, d);
    }
    db
}

fn ids(docs: &[Value]) -> Vec<i64> {
    let mut v: Vec<i64> = docs.iter().map(|d| d["_id"].as_i64().unwrap()).collect();
    v.sort();
    v
}

fn find_opts(
    db: &DatabaseCore<MemoryStorage>,
    q: &Value,
    skip: Option<usize>,
    limit: Option<usize>,
) -> Vec<Value> {
    let mut o = FindOptions::new();
    if let Some(s) = skip {
        o = o.with_skip(s);
    }
    if let Some(l) = limit {
        o = o.with_limit(l);
    }
    db.collection("c").unwrap().find_with_options(q, o).unwrap()
}

fn five() -> Vec<Value> {
    (1..=5)
        .map(|i| json!({"_id": i, "a": i % 3, "b": i % 2}))
        .collect()
}

#[test]
fn find_one_handles_id_operators() {
    let db = db_with(five());
    let coll = db.collection("c").unwrap();
    for q in [
        json!({"_id": {"$in": [1, 2]}}),
        json!({"_id": {"$gt": 0}}),
        json!({"_id": {"$ne": 1}}),
        json!({"_id": {"$exists": true}}),
        json!({"_id": {"$gte": 5}}),
    ] {
        assert!(coll.find_one(&q).unwrap().is_some(), "find_one {q}");
        assert!(
            coll.find_one_with_ctx(&q, None).unwrap().is_some(),
            "find_one_with_ctx {q}"
        );
    }
    assert!(coll
        .find_one(&json!({"_id": {"$gt": 5}}))
        .unwrap()
        .is_none());
}

#[test]
fn id_fast_paths_honour_skip_limit_and_dedup() {
    let db = db_with(five());
    let coll = db.collection("c").unwrap();
    let q_in = json!({"_id": {"$in": [1, 2, 3]}});
    assert_eq!(find_opts(&db, &q_in, None, Some(1)).len(), 1);
    assert_eq!(find_opts(&db, &q_in, Some(2), None).len(), 1);
    assert_eq!(find_opts(&db, &q_in, Some(1), Some(1)).len(), 1);
    assert_eq!(find_opts(&db, &json!({"_id": 1}), Some(1), None).len(), 0);

    let dup = json!({"_id": {"$in": [1, 1, 2]}});
    assert_eq!(ids(&coll.find(&dup).unwrap()), vec![1, 2]);
    assert_eq!(coll.count_documents(&dup).unwrap(), 2);
    let mixed = json!({"_id": {"$in": [1, "1"]}});
    assert_eq!(ids(&coll.find(&mixed).unwrap()), vec![1]);
    assert_eq!(coll.count_documents(&mixed).unwrap(), 1);
}

#[test]
fn and_per_clause_path_applies_limit_after_intersection() {
    let db = db_with(vec![
        json!({"_id": 1, "a": 1}),
        json!({"_id": 2, "a": 2}),
        json!({"_id": 3, "a": 3}),
        json!({"_id": 4, "a": 2}),
    ]);
    let coll = db.collection("c").unwrap();
    coll.create_index("a".to_string(), false, false).unwrap();
    let q = json!({"$and": [{"a": {"$in": [1, 2]}}, {"a": {"$in": [2, 3]}}]});
    assert_eq!(ids(&coll.find(&q).unwrap()), vec![2, 4]);
    assert_eq!(coll.count_documents(&q).unwrap(), 2);
    assert!(coll.find_one(&q).unwrap().is_some());
    assert_eq!(find_opts(&db, &q, None, Some(1)).len(), 1);
    assert_eq!(find_opts(&db, &q, Some(1), Some(1)).len(), 1);
}

#[test]
fn or_per_clause_path_applies_skip_limit_to_the_union() {
    for indexed in [false, true] {
        let db = db_with(vec![
            json!({"_id": 1, "a": 1, "b": 0}),
            json!({"_id": 2, "a": 0, "b": 1}),
            json!({"_id": 3, "a": 1, "b": 1}),
            json!({"_id": 4, "a": 0, "b": 0}),
            json!({"_id": 5, "a": 1, "b": 0}),
        ]);
        if indexed {
            let coll = db.collection("c").unwrap();
            coll.create_index("a".to_string(), false, false).unwrap();
            coll.create_index("b".to_string(), false, false).unwrap();
        }
        let q = json!({"$or": [{"a": 1}, {"b": 1}]});
        assert_eq!(ids(&find_opts(&db, &q, None, None)), vec![1, 2, 3, 5]);
        assert_eq!(find_opts(&db, &q, None, Some(0)).len(), 4, "limit 0");
        assert_eq!(
            find_opts(&db, &q, Some(2), Some(2)).len(),
            2,
            "skip 2 limit 2"
        );
        assert_eq!(find_opts(&db, &q, Some(3), None).len(), 1, "skip 3");
        let page1 = ids(&find_opts(&db, &q, None, Some(2)));
        let page2 = ids(&find_opts(&db, &q, Some(2), Some(2)));
        let mut all = [page1, page2].concat();
        all.sort();
        assert_eq!(
            all,
            vec![1, 2, 3, 5],
            "pages cover the union (indexed={indexed})"
        );
    }
}

#[test]
fn regex_prefix_with_limit_zero_means_no_limit() {
    let db = db_with(vec![
        json!({"_id": 1, "s": "apple"}),
        json!({"_id": 2, "s": "apricot"}),
        json!({"_id": 3, "s": "banana"}),
    ]);
    db.collection("c")
        .unwrap()
        .create_index("s".to_string(), false, false)
        .unwrap();
    let q = json!({"s": {"$regex": "^ap"}});
    assert_eq!(ids(&find_opts(&db, &q, None, Some(0))), vec![1, 2]);
    assert_eq!(ids(&find_opts(&db, &q, None, None)), vec![1, 2]);
}

/// A hint must not change the result: operator objects other than ranges
/// used to become an equality on the Null key and return [].
#[test]
fn find_with_hint_matches_unhinted_find() {
    let db = db_with(vec![
        json!({"_id": 1, "a": 1, "s": "abc"}),
        json!({"_id": 2, "a": 2, "s": "abd"}),
        json!({"_id": 3, "a": 3, "s": "xyz"}),
        json!({"_id": 4, "a": 1.0, "s": "abz"}),
    ]);
    let coll = db.collection("c").unwrap();
    let a_idx = coll.create_index("a".to_string(), false, false).unwrap();
    let s_idx = coll.create_index("s".to_string(), false, false).unwrap();
    for (q, hint) in [
        (json!({"a": {"$eq": 2}}), &a_idx),
        (json!({"a": {"$in": [1, 3]}}), &a_idx),
        (json!({"a": 1}), &a_idx),
        (json!({"a": {"$gte": 2}}), &a_idx),
        (json!({"s": {"$regex": "^ab"}}), &s_idx),
    ] {
        let hinted = coll.find_with_hint(&q, hint).unwrap();
        assert_eq!(
            ids(&hinted),
            ids(&coll.find(&q).unwrap()),
            "{q} hint {hint}"
        );
        assert!(!hinted.is_empty(), "{q}");
    }
    // An operator the index cannot serve is an explicit error, not [].
    assert!(coll
        .find_with_hint(&json!({"a": {"$ne": 1}}), &a_idx)
        .is_err());
}
