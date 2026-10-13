// expr_type_operator_tests.rs
// $expr $eq/$ne across types, objects and missing fields (audit
// 2026-10-07 O8) and $type on array elements (O9).

use ironbase_core::storage::MemoryStorage;
use ironbase_core::DatabaseCore;
use serde_json::{json, Value};
use std::collections::HashMap;

fn ids(docs: &[Value], query: Value) -> Vec<i64> {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    for d in docs {
        let fields: HashMap<String, Value> = d.as_object().unwrap().clone().into_iter().collect();
        db.insert_one("c", fields).unwrap();
    }
    let mut out: Vec<i64> = db
        .collection("c")
        .unwrap()
        .find(&query)
        .unwrap_or_else(|e| panic!("{query}: {e}"))
        .iter()
        .map(|d| d["_id"].as_i64().unwrap())
        .collect();
    out.sort();
    out
}

/// O8: $ne is true across types, objects compare by value, and a missing
/// field equals only another missing field.
#[test]
fn expr_eq_ne_across_types_objects_and_missing() {
    let docs = [
        json!({"_id": 1, "a": 1, "b": "1"}),
        json!({"_id": 2, "a": {"x": 1}, "b": {"x": 1}}),
        json!({"_id": 3, "a": 1}),
        json!({"_id": 4}),
        json!({"_id": 5, "a": 2, "b": 2.0}),
    ];
    assert_eq!(
        ids(&docs, json!({"$expr": {"$eq": ["$a", "$b"]}})),
        vec![2, 4, 5]
    );
    assert_eq!(
        ids(&docs, json!({"$expr": {"$ne": ["$a", "$b"]}})),
        vec![1, 3]
    );
}

/// O9: $type matches when the field or any of its array elements has the
/// type.
#[test]
fn type_looks_into_arrays() {
    let docs = [
        json!({"_id": 1, "t": ["x", 1]}),
        json!({"_id": 2, "t": [true]}),
        json!({"_id": 3, "t": "s"}),
        json!({"_id": 4, "t": []}),
    ];
    assert_eq!(ids(&docs, json!({"t": {"$type": "string"}})), vec![1, 3]);
    assert_eq!(ids(&docs, json!({"t": {"$type": "number"}})), vec![1]);
    assert_eq!(ids(&docs, json!({"t": {"$type": "bool"}})), vec![2]);
    assert_eq!(ids(&docs, json!({"t": {"$type": "array"}})), vec![1, 2, 4]);
}
