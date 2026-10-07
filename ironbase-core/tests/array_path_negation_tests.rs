// array_path_negation_tests.rs
// Negated operators ($ne, $nin, $not) and $all on paths that traverse arrays,
// and $elemMatch sub-queries on object elements (MongoDB semantics).

use ironbase_core::storage::MemoryStorage;
use ironbase_core::DatabaseCore;
use serde_json::{json, Value};
use std::collections::HashMap;

fn db_with(docs: Vec<Value>) -> DatabaseCore<MemoryStorage> {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    for d in docs {
        let fields: HashMap<String, Value> = d.as_object().unwrap().clone().into_iter().collect();
        db.insert_one("c", fields).unwrap();
    }
    db
}

/// Sorted `n` tags of the documents matching `query` (find and count agree).
fn matching(db: &DatabaseCore<MemoryStorage>, query: Value) -> Vec<String> {
    let coll = db.collection("c").unwrap();
    let mut tags: Vec<String> = coll
        .find(&query)
        .unwrap()
        .iter()
        .map(|d| d["n"].as_str().unwrap().to_string())
        .collect();
    tags.sort();
    assert_eq!(
        coll.count_documents(&query).unwrap() as usize,
        tags.len(),
        "count != find for {query}"
    );
    tags
}

fn items_docs() -> Vec<Value> {
    vec![
        json!({"n": "AB", "items": [{"name": "a"}, {"name": "b"}]}),
        json!({"n": "B", "items": [{"name": "b"}]}),
        json!({"n": "A", "items": [{"name": "a"}]}),
        json!({"n": "NONE", "items": [{"other": 1}]}),
    ]
}

#[test]
fn ne_through_array_path_excludes_docs_with_any_equal_value() {
    let db = db_with(items_docs());
    assert_eq!(
        matching(&db, json!({"items.name": {"$ne": "a"}})),
        vec!["B", "NONE"]
    );
}

#[test]
fn nin_through_array_path_excludes_docs_with_any_listed_value() {
    let db = db_with(items_docs());
    assert_eq!(
        matching(&db, json!({"items.name": {"$nin": ["a"]}})),
        vec!["B", "NONE"]
    );
}

#[test]
fn not_through_array_path_negates_the_any_match() {
    let db = db_with(items_docs());
    assert_eq!(
        matching(&db, json!({"items.name": {"$not": {"$eq": "a"}}})),
        vec!["B", "NONE"]
    );
    assert_eq!(
        matching(&db, json!({"items.name": {"$not": {"$regex": "^a"}}})),
        vec!["B", "NONE"]
    );
}

#[test]
fn ne_through_wildcard_path() {
    let db = db_with(vec![
        json!({"n": "ZQ", "x": {"name": "Z"}, "name": "Q"}),
        json!({"n": "Q", "name": "Q"}),
    ]);
    assert_eq!(matching(&db, json!({"$**.name": {"$ne": "Z"}})), vec!["Q"]);
}

/// The data-loss case: delete_many with a negated filter must keep every
/// document that has the excluded value in any element.
#[test]
fn delete_many_with_ne_through_array_path_keeps_matching_docs() {
    let db = db_with(items_docs());
    let deleted = db
        .delete_many("c", &json!({"items.name": {"$ne": "a"}}))
        .unwrap();
    assert_eq!(deleted, 2);
    assert_eq!(matching(&db, json!({})), vec!["A", "AB"]);
}

#[test]
fn all_through_array_path_uses_the_whole_value_set() {
    let db = db_with(items_docs());
    assert_eq!(
        matching(&db, json!({"items.name": {"$all": ["a", "b"]}})),
        vec!["AB"]
    );
    assert_eq!(
        matching(&db, json!({"items.name": {"$all": ["a"]}})),
        vec!["A", "AB"]
    );
}

fn elem_docs() -> Vec<Value> {
    vec![
        json!({"n": "BLUE", "items": [{"meta": {"c": "blue"}, "spec": {"color": "blue"}, "a": 1, "tags": ["y"]}]}),
        json!({"n": "RED", "items": [{"meta": {"c": "red"}, "spec": {"color": "red"}, "a": 2, "tags": ["x", "y"]}]}),
        json!({"n": "NOMETA", "items": [{"z": 1, "a": 5}]}),
    ]
}

#[test]
fn elem_match_object_valued_condition_is_deep_equality() {
    let db = db_with(elem_docs());
    let q = json!({"items": {"$elemMatch": {"meta": {"c": "red"}}}});
    assert_eq!(matching(&db, q.clone()), vec!["RED"]);
    assert_eq!(db.delete_many("c", &q).unwrap(), 1);
    assert_eq!(matching(&db, json!({})), vec!["BLUE", "NOMETA"]);
}

#[test]
fn elem_match_object_sub_query_semantics() {
    let db = db_with(elem_docs());
    assert_eq!(
        matching(&db, json!({"items": {"$elemMatch": {"spec.color": "red"}}})),
        vec!["RED"]
    );
    assert_eq!(
        matching(&db, json!({"items": {"$elemMatch": {"tags": "x"}}})),
        vec!["RED"]
    );
    assert_eq!(
        matching(
            &db,
            json!({"items": {"$elemMatch": {"$or": [{"a": 2}, {"a": 5}]}}})
        ),
        vec!["NOMETA", "RED"]
    );
    assert_eq!(
        matching(
            &db,
            json!({"items": {"$elemMatch": {"a": {"$not": {"$gt": 2}}}}})
        ),
        vec!["BLUE", "RED"]
    );
}

#[test]
fn elem_match_value_operators_on_scalar_elements() {
    let db = db_with(vec![
        json!({"n": "HIT", "scores": [75, 82, 90]}),
        json!({"n": "MISS", "scores": [70, 90]}),
    ]);
    assert_eq!(
        matching(
            &db,
            json!({"scores": {"$elemMatch": {"$gt": 80, "$lt": 85}}})
        ),
        vec!["HIT"]
    );
    assert_eq!(
        matching(
            &db,
            json!({"scores": {"$elemMatch": {"$not": {"$gte": 75}}}})
        ),
        vec!["MISS"]
    );
}

/// The same answers through a multikey index on the traversed path.
#[test]
fn negated_operators_through_indexed_array_path() {
    let db = db_with(items_docs());
    db.collection("c")
        .unwrap()
        .create_index("items.name".to_string(), false, false)
        .unwrap();
    assert_eq!(
        matching(&db, json!({"items.name": {"$ne": "a"}})),
        vec!["B", "NONE"]
    );
    assert_eq!(
        matching(&db, json!({"items.name": {"$nin": ["a"]}})),
        vec!["B", "NONE"]
    );
    assert_eq!(
        matching(&db, json!({"items.name": {"$nin": ["a"], "$exists": true}})),
        vec!["B"]
    );
    assert_eq!(
        matching(&db, json!({"items.name": {"$all": ["a", "b"]}})),
        vec!["AB"]
    );
}

#[test]
fn negated_operators_through_indexed_array_field() {
    let db = db_with(vec![
        json!({"n": "AB", "tags": ["a", "b"]}),
        json!({"n": "B", "tags": ["b"]}),
        json!({"n": "S", "tags": "a"}),
    ]);
    db.collection("c")
        .unwrap()
        .create_index("tags".to_string(), false, false)
        .unwrap();
    assert_eq!(matching(&db, json!({"tags": {"$ne": "a"}})), vec!["B"]);
    assert_eq!(matching(&db, json!({"tags": {"$nin": ["a"]}})), vec!["B"]);
    assert_eq!(
        matching(&db, json!({"tags": {"$ne": "a", "$gte": "a"}})),
        vec!["B"]
    );
}
