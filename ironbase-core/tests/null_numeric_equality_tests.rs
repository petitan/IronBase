// null_numeric_equality_tests.rs
// MongoDB equality semantics (audit 2026-10-07 O4, O5, Q2, Q3, Q4):
// - `{f: null}` matches null AND missing fields; `$ne: null` is "has a value"
// - numbers compare by value: 1 == 1.0
// Every query must return the same documents with and without an index.

use ironbase_core::storage::MemoryStorage;
use ironbase_core::DatabaseCore;
use serde_json::{json, Value};
use std::collections::HashMap;

fn db_with(docs: &[Value]) -> DatabaseCore<MemoryStorage> {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    for d in docs {
        let fields: HashMap<String, Value> = d.as_object().unwrap().clone().into_iter().collect();
        db.insert_one("c", fields).unwrap();
    }
    db
}

/// Sorted `n` tags matched by find; count_documents and find_one agree.
fn matching(db: &DatabaseCore<MemoryStorage>, query: &Value) -> Vec<String> {
    let coll = db.collection("c").unwrap();
    let mut tags: Vec<String> = coll
        .find(query)
        .unwrap()
        .iter()
        .map(|d| d["n"].as_str().unwrap().to_string())
        .collect();
    tags.sort();
    assert_eq!(
        coll.count_documents(query).unwrap() as usize,
        tags.len(),
        "count != find for {query}"
    );
    assert_eq!(
        coll.find_one(query).unwrap().is_some(),
        !tags.is_empty(),
        "find_one disagrees with find for {query}"
    );
    tags
}

/// Run every case without an index, then with each index set; all must give
/// the expected documents.
fn check(docs: &[Value], index_sets: &[&[&str]], cases: &[(Value, &[&str])]) {
    let mut setups: Vec<Vec<&str>> = vec![vec![]];
    setups.extend(index_sets.iter().map(|s| s.to_vec()));
    for (k, indexes) in setups.iter().enumerate() {
        let db = db_with(docs);
        let coll = db.collection("c").unwrap();
        for spec in indexes {
            match *spec {
                "sparse:a" => {
                    coll.create_index("a".to_string(), false, true).unwrap();
                }
                "unique:a" => {
                    coll.create_index("a".to_string(), true, false).unwrap();
                }
                "compound:a,b" => {
                    coll.create_compound_index(
                        vec!["a".to_string(), "b".to_string()],
                        false,
                        false,
                    )
                    .unwrap();
                }
                field => {
                    coll.create_index(field.to_string(), false, false).unwrap();
                }
            }
        }
        for (q, expected) in cases {
            assert_eq!(
                matching(&db, q),
                expected.to_vec(),
                "query {q} with indexes {indexes:?} (setup {k})"
            );
        }
    }
}

fn null_docs() -> Vec<Value> {
    vec![
        json!({"n": "V", "a": 5, "b": 1}),
        json!({"n": "N", "a": null, "b": 2}),
        json!({"n": "M", "b": 3}),
    ]
}

#[test]
fn null_matches_null_and_missing() {
    check(
        &null_docs(),
        &[&["a"], &["sparse:a"], &["compound:a,b"]],
        &[
            (json!({"a": null}), &["M", "N"]),
            (json!({"a": {"$eq": null}}), &["M", "N"]),
            (json!({"a": {"$in": [null, 5]}}), &["M", "N", "V"]),
            (json!({"a": {"$in": [null]}}), &["M", "N"]),
            (json!({"$or": [{"a": null}, {"a": 5}]}), &["M", "N", "V"]),
            (json!({"a": {"$gte": null}}), &["M", "N"]),
            (json!({"a": {"$lte": null}}), &["M", "N"]),
        ],
    );
}

#[test]
fn ne_and_nin_null_mean_has_a_value() {
    check(
        &null_docs(),
        &[&["a"], &["sparse:a"]],
        &[
            (json!({"a": {"$ne": null}}), &["V"]),
            (json!({"a": {"$nin": [null]}}), &["V"]),
            (json!({"a": {"$nin": [null, 5]}}), &[]),
            (json!({"a": {"$not": {"$eq": null}}}), &["V"]),
        ],
    );
}

/// A unique index keeps null and missing values under the Null key, and
/// objects collapse to Null too: none of them may be counted for `{a: null}`
/// unless they really are null or missing.
#[test]
fn null_count_with_unique_and_compound_index() {
    // Unique: an object and a missing field would both be a Null key (a
    // duplicate), so the unique case has no missing field.
    check(
        &[
            json!({"n": "O", "a": {"x": 1}, "b": 1}),
            json!({"n": "V", "a": 5, "b": 2}),
        ],
        &[&["unique:a"]],
        &[(json!({"a": null}), &[])],
    );
    check(
        &[
            json!({"n": "O", "a": {"x": 1}, "b": 1}),
            json!({"n": "V", "a": 5, "b": 2}),
            json!({"n": "M", "b": 3}),
        ],
        &[&["compound:a,b"]],
        &[(json!({"a": null}), &["M"])],
    );
}

/// A sparse index omits null, `[]` and object values: it cannot answer
/// `$exists: true` on its own.
#[test]
fn exists_true_with_sparse_index_keeps_null_empty_array_and_object() {
    check(
        &[
            json!({"n": "NULL", "a": null}),
            json!({"n": "FIVE", "a": 5}),
            json!({"n": "MISSING"}),
            json!({"n": "EMPTY", "a": []}),
            json!({"n": "OBJ", "a": {"x": 1}}),
            json!({"n": "ARR", "a": [1]}),
        ],
        &[&["sparse:a"]],
        &[
            (
                json!({"a": {"$exists": true}}),
                &["ARR", "EMPTY", "FIVE", "NULL", "OBJ"],
            ),
            (json!({"a": {"$exists": false}}), &["MISSING"]),
        ],
    );
}

fn number_docs() -> Vec<Value> {
    vec![
        json!({"n": "F", "v": 1.0}),
        json!({"n": "I", "v": 1}),
        json!({"n": "T", "v": 2.5}),
        json!({"n": "ARR", "v": [3, 1.0]}),
    ]
}

#[test]
fn integer_and_float_with_the_same_value_are_equal() {
    check(
        &number_docs(),
        &[&["v"]],
        &[
            (json!({"v": 1}), &["ARR", "F", "I"]),
            (json!({"v": 1.0}), &["ARR", "F", "I"]),
            (json!({"v": {"$eq": 1}}), &["ARR", "F", "I"]),
            (json!({"v": {"$in": [1]}}), &["ARR", "F", "I"]),
            (json!({"v": {"$in": [1.0, 2.5]}}), &["ARR", "F", "I", "T"]),
            (json!({"v": {"$ne": 1}}), &["T"]),
            (json!({"v": {"$nin": [1.0]}}), &["T"]),
            (json!({"v": {"$all": [1.0, 3]}}), &["ARR"]),
            (json!({"v": 3.0}), &["ARR"]),
        ],
    );
}

#[test]
fn nested_numbers_compare_by_value() {
    check(
        &[
            json!({"n": "OBJ", "o": {"k": 1.0}}),
            json!({"n": "ARR", "o": [1, 2.0]}),
        ],
        &[],
        &[
            (json!({"o": {"k": 1}}), &["OBJ"]),
            (json!({"o": [1.0, 2]}), &["ARR"]),
            (json!({"o": {"$in": [{"k": 1}]}}), &["OBJ"]),
        ],
    );
}

/// `$pull` conditions use the same numeric equality as queries.
#[test]
fn pull_matches_numbers_by_value() {
    let db = db_with(&[json!({"n": "P", "xs": [1, 1.0, 2, 3.0]})]);
    db.update_one("c", &json!({"n": "P"}), &json!({"$pull": {"xs": 1}}))
        .unwrap();
    db.update_one(
        "c",
        &json!({"n": "P"}),
        &json!({"$pull": {"xs": {"$in": [3]}}}),
    )
    .unwrap();
    let doc = db
        .collection("c")
        .unwrap()
        .find_one(&json!({"n": "P"}))
        .unwrap()
        .unwrap();
    assert_eq!(doc["xs"], json!([2]));
}
