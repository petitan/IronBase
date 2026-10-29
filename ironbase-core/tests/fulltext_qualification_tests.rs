//! Document-level AND (`CollectionCore::fulltext_qualify_documents`): a
//! document qualifies when every query token appears in its indexed text,
//! possibly spread over several chunks.

use ironbase_core::storage::MemoryStorage;
use ironbase_core::{DatabaseCore, DocumentQualification};
use serde_json::{json, Value};
use std::collections::HashMap;

fn insert(db: &DatabaseCore<MemoryStorage>, collection: &str, doc: Value) {
    let map: HashMap<String, Value> = serde_json::from_value(doc).unwrap();
    db.insert_one(collection, map).unwrap();
}

fn sorted_strings(ids: &[Value]) -> Vec<String> {
    let mut out: Vec<String> = ids.iter().map(|v| v.to_string()).collect();
    out.sort();
    out
}

/// RAG chunks: `alpha` and `beta` are in different chunks of doc A; doc B
/// only has `alpha`, doc C only `beta`.
fn rag_db() -> DatabaseCore<MemoryStorage> {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    insert(&db, "kb", json!({"doc_id": "A", "content": "alpha one"}));
    insert(&db, "kb", json!({"doc_id": "A", "content": "beta two"}));
    insert(&db, "kb", json!({"doc_id": "B", "content": "alpha three"}));
    insert(&db, "kb", json!({"doc_id": "C", "content": "beta four"}));
    db.collection("kb")
        .unwrap()
        .create_fulltext_index("content".to_string(), "none", None, None)
        .unwrap();
    db
}

#[test]
fn rag_tokens_may_be_in_different_chunks_of_one_document() {
    let db = rag_db();
    let coll = db.collection("kb").unwrap();
    let q = coll
        .fulltext_qualify_documents(&["content"], "alpha beta")
        .unwrap();
    assert_eq!(
        q,
        DocumentQualification::Documents {
            field: "doc_id".to_string(),
            ids: vec![json!("A")],
        }
    );
    // The filter selects every chunk of the qualified document.
    let chunks = coll.find(&q.to_filter().unwrap()).unwrap();
    assert_eq!(chunks.len(), 2);
}

#[test]
fn a_single_token_restricts_nothing() {
    let db = rag_db();
    let coll = db.collection("kb").unwrap();
    let q = coll
        .fulltext_qualify_documents(&["content"], "alpha")
        .unwrap();
    assert_eq!(q, DocumentQualification::NotRequired);
    assert_eq!(q.to_filter(), None);
}

#[test]
fn a_token_no_document_has_qualifies_none() {
    let db = rag_db();
    let coll = db.collection("kb").unwrap();
    let q = coll
        .fulltext_qualify_documents(&["content"], "alpha missing")
        .unwrap();
    match &q {
        DocumentQualification::Documents { ids, .. } => assert!(ids.is_empty()),
        other => panic!("expected an empty restriction, got {:?}", other),
    }
    assert!(coll.find(&q.to_filter().unwrap()).unwrap().is_empty());
}

#[test]
fn non_rag_documents_qualify_on_their_own_id() {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    insert(&db, "notes", json!({"_id": 1, "text": "alpha beta"}));
    insert(
        &db,
        "notes",
        json!({"_id": "two", "text": "alpha beta gamma"}),
    );
    insert(&db, "notes", json!({"_id": 3, "text": "alpha only"}));
    let coll = db.collection("notes").unwrap();
    coll.create_fulltext_index("text".to_string(), "none", None, None)
        .unwrap();

    let q = coll
        .fulltext_qualify_documents(&["text"], "alpha beta")
        .unwrap();
    let DocumentQualification::Documents { field, ids } = &q else {
        panic!("expected a restriction, got {:?}", q);
    };
    assert_eq!(field, "_id");
    // Native id values, so numeric and string ids both match.
    assert_eq!(sorted_strings(ids), vec!["\"two\"", "1"]);
    assert_eq!(coll.find(&q.to_filter().unwrap()).unwrap().len(), 2);
}

#[test]
fn several_fields_union_the_documents_qualified_on_each() {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    insert(
        &db,
        "kb",
        json!({"doc_id": "X", "title": "alpha beta", "body": "none here"}),
    );
    insert(
        &db,
        "kb",
        json!({"doc_id": "Y", "title": "nothing", "body": "alpha and beta"}),
    );
    // Tokens split across the two fields do not qualify: each field must hold all.
    insert(
        &db,
        "kb",
        json!({"doc_id": "Z", "title": "alpha", "body": "beta"}),
    );
    let coll = db.collection("kb").unwrap();
    coll.create_fulltext_index("title".to_string(), "none", None, None)
        .unwrap();
    coll.create_fulltext_index("body".to_string(), "none", None, None)
        .unwrap();

    let q = coll
        .fulltext_qualify_documents(&["title", "body"], "alpha beta")
        .unwrap();
    let DocumentQualification::Documents { field, ids } = &q else {
        panic!("expected a restriction, got {:?}", q);
    };
    assert_eq!(field, "doc_id");
    assert_eq!(sorted_strings(ids), vec!["\"X\"", "\"Y\""]);
}

#[test]
fn a_field_without_a_fulltext_index_is_an_error() {
    let db = rag_db();
    let coll = db.collection("kb").unwrap();
    assert!(coll
        .fulltext_qualify_documents(&["missing"], "alpha beta")
        .is_err());
}
