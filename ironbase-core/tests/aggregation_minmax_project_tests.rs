// aggregation_minmax_project_tests.rs
// $min/$max over null, missing and mixed-type values (audit 2026-10-07 A8)
// and $project dot paths: exclusion and inclusion through arrays (A9).

use ironbase_core::storage::MemoryStorage;
use ironbase_core::DatabaseCore;
use serde_json::{json, Value};
use std::collections::HashMap;

fn aggregate(docs: &[Value], pipeline: Value) -> Vec<Value> {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    for d in docs {
        let fields: HashMap<String, Value> = d.as_object().unwrap().clone().into_iter().collect();
        db.insert_one("c", fields).unwrap();
    }
    db.collection("c").unwrap().aggregate(&pipeline).unwrap()
}

fn min_max(docs: &[Value]) -> (Value, Value) {
    let out = aggregate(
        docs,
        json!([{"$group": {"_id": null, "lo": {"$min": "$v"}, "hi": {"$max": "$v"}}}]),
    );
    (out[0]["lo"].clone(), out[0]["hi"].clone())
}

/// A8: null and missing values are ignored, wherever they come.
#[test]
fn min_max_ignore_null_and_missing() {
    let docs = [
        json!({"_id": 1, "v": null}),
        json!({"_id": 2, "v": 5}),
        json!({"_id": 3}),
        json!({"_id": 4, "v": 2}),
        json!({"_id": 5, "v": 9}),
    ];
    assert_eq!(min_max(&docs), (json!(2), json!(9)));
    // only null/missing: null
    assert_eq!(
        min_max(&[json!({"_id": 1, "v": null}), json!({"_id": 2})]),
        (Value::Null, Value::Null)
    );
}

/// A8: mixed types follow the $sort order, independent of document order.
#[test]
fn min_max_mixed_types_do_not_depend_on_order() {
    let forward = [
        json!({"_id": 1, "v": "b"}),
        json!({"_id": 2, "v": 3}),
        json!({"_id": 3, "v": "a"}),
        json!({"_id": 4, "v": 7}),
    ];
    let mut backward = forward.clone();
    backward.reverse();
    // numbers sort before strings
    assert_eq!(min_max(&forward), (json!(3), json!("b")));
    assert_eq!(min_max(&backward), (json!(3), json!("b")));
}

/// A9: a dot-path exclusion removes the nested field, also inside arrays.
#[test]
fn project_dot_path_exclusion() {
    let out = aggregate(
        &[json!({"_id": 1, "a": {"b": 1, "c": 2}, "items": [{"x": 1, "y": 2}, {"x": 3, "y": 4}]})],
        json!([{"$project": {"a.b": 0, "items.y": 0}}]),
    );
    assert_eq!(
        out[0],
        json!({"_id": 1, "a": {"c": 2}, "items": [{"x": 1}, {"x": 3}]})
    );
}

/// A9: a dot-path inclusion through an array keeps every element's field.
#[test]
fn project_dot_path_inclusion_through_array() {
    let out = aggregate(
        &[json!({"_id": 1, "items": [{"x": 1, "y": 2}, {"x": 3, "y": 4}], "z": 0})],
        json!([{"$project": {"items.x": 1}}]),
    );
    assert_eq!(out[0], json!({"_id": 1, "items": [{"x": 1}, {"x": 3}]}));

    // two paths under the same array merge element by element
    let out = aggregate(
        &[json!({"_id": 1, "items": [{"x": 1, "y": 2, "w": 0}, {"x": 3, "y": 4, "w": 0}]})],
        json!([{"$project": {"items.x": 1, "items.y": 1, "_id": 0}}]),
    );
    assert_eq!(
        out[0],
        json!({"items": [{"x": 1, "y": 2}, {"x": 3, "y": 4}]})
    );
}
