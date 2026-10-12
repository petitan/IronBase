// text_operator_extended_form_tests.rs
// The documented extended forms of $text / $startsWith / $endsWith /
// $contains pass query validation (audit 2026-10-07 O6).

use ironbase_core::storage::MemoryStorage;
use ironbase_core::DatabaseCore;
use serde_json::{json, Value};
use std::collections::HashMap;

fn db() -> DatabaseCore<MemoryStorage> {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    for (name, bio) in [
        ("Alice", "Rust developer"),
        ("alfred", "likes rust"),
        ("Bob", "Python"),
    ] {
        let fields: HashMap<String, Value> = HashMap::from([
            ("name".to_string(), json!(name)),
            ("bio".to_string(), json!(bio)),
        ]);
        db.insert_one("c", fields).unwrap();
    }
    db
}

fn names(docs: Vec<Value>) -> Vec<String> {
    let mut v: Vec<String> = docs
        .iter()
        .map(|d| d["name"].as_str().unwrap().to_string())
        .collect();
    v.sort();
    v
}

#[test]
fn extended_forms_are_accepted_by_find() {
    let db = db();
    let coll = db.collection("c").unwrap();
    let cases = [
        (
            json!({"name": {"$startsWith": {"$value": "Al", "$caseSensitive": true}}}),
            vec!["Alice"],
        ),
        (
            json!({"name": {"$startsWith": {"$value": "al"}}}),
            vec!["Alice", "alfred"],
        ),
        (
            json!({"name": {"$endsWith": {"$value": "ED", "$caseSensitive": false}}}),
            vec!["alfred"],
        ),
        (
            json!({"bio": {"$contains": {"$value": "Rust", "$caseSensitive": true}}}),
            vec!["Alice"],
        ),
        (
            json!({"bio": {"$text": {"$search": "rust"}}}),
            vec!["Alice", "alfred"],
        ),
        (
            json!({"bio": {"$text": {"$search": "developers", "$language": "english", "$caseSensitive": false}}}),
            vec!["Alice"],
        ),
    ];
    for (query, expected) in cases {
        let docs = coll.find(&query).unwrap_or_else(|e| panic!("{query}: {e}"));
        assert_eq!(names(docs), expected, "{query}");
    }
}

#[test]
fn extended_forms_are_accepted_by_count_and_delete() {
    let db = db();
    let coll = db.collection("c").unwrap();
    let query = json!({"bio": {"$contains": {"$value": "rust"}}});
    assert_eq!(coll.count_documents(&query).unwrap(), 2);
    assert_eq!(db.delete_many("c", &query).unwrap(), 2);
    assert_eq!(coll.count_documents(&json!({})).unwrap(), 1);
}

/// Unknown option keys are still rejected, like unknown operators.
#[test]
fn unknown_option_key_is_rejected() {
    let db = db();
    let coll = db.collection("c").unwrap();
    let err = coll
        .find(&json!({"name": {"$startsWith": {"$value": "Al", "$bogus": 1}}}))
        .unwrap_err();
    assert!(err.to_string().contains("$bogus"), "{err}");
}
