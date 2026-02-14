//! Regression test for GitHub issue #36:
//! $regex ignored in count_documents, distinct, and find(include_total)
//!
//! When a query combines an indexed field with $regex on another field,
//! count_documents must apply the $regex filter instead of returning
//! the unfiltered index count.

use std::collections::HashMap;

use ironbase_core::find_options::FindOptions;
use ironbase_core::storage::MemoryStorage;
use ironbase_core::DatabaseCore;
use serde_json::json;

fn create_test_db() -> DatabaseCore<MemoryStorage> {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    let _ = db.collection("docs").unwrap();

    // Insert documents with doc_type + content fields
    let data = vec![
        ("report", "The quick brown fox jumps over the lazy dog"),
        ("report", "Rust is a systems programming language"),
        ("report", "IronBase is a MongoDB-compatible database"),
        ("invoice", "Invoice #1234 for consulting services"),
        ("invoice", "Invoice #5678 for software development"),
        ("contract", "Service agreement between parties"),
    ];

    for (doc_type, content) in data {
        db.insert_one(
            "docs",
            HashMap::from([
                ("doc_type".to_string(), json!(doc_type)),
                ("content".to_string(), json!(content)),
            ]),
        )
        .unwrap();
    }

    // Create index on doc_type (the field the planner will pick)
    let coll = db.get_collection("docs").unwrap();
    coll.create_index("doc_type".to_string(), false, false)
        .unwrap();

    db
}

/// FIX #36: count_documents with indexed field + $regex must apply both filters
#[test]
fn test_count_documents_regex_with_indexed_field() {
    let db = create_test_db();

    // count with indexed field only — should use fast index count
    let count_all_reports = db
        .get_collection("docs")
        .unwrap()
        .count_documents(&json!({"doc_type": "report"}))
        .unwrap();
    assert_eq!(count_all_reports, 3, "3 reports total");

    // count with indexed field + $regex — must apply $regex filter
    let count_regex = db
        .get_collection("docs")
        .unwrap()
        .count_documents(&json!({
            "doc_type": "report",
            "content": {"$regex": "Rust"}
        }))
        .unwrap();
    assert_eq!(
        count_regex, 1,
        "Only 1 report contains 'Rust' — was returning 3 (unfiltered) before fix"
    );

    // count with $regex that matches nothing
    let count_none = db
        .get_collection("docs")
        .unwrap()
        .count_documents(&json!({
            "doc_type": "report",
            "content": {"$regex": "XYZNONEXISTENT"}
        }))
        .unwrap();
    assert_eq!(
        count_none, 0,
        "No report contains 'XYZNONEXISTENT' — was returning 3 before fix"
    );
}

/// FIX #36: find(include_total) uses count_documents internally — total must be filtered
#[test]
fn test_find_include_total_with_regex() {
    let db = create_test_db();
    let coll = db.get_collection("docs").unwrap();

    let options = FindOptions::default().with_include_total(true);
    let result = coll
        .find_with_result(
            &json!({
                "doc_type": "report",
                "content": {"$regex": "Rust"}
            }),
            options,
        )
        .unwrap();

    assert_eq!(
        result.documents.len(),
        1,
        "find returns 1 matching document"
    );
    assert_eq!(
        result.total,
        Some(1),
        "total must match document count — was returning Some(3) before fix"
    );
}

/// FIX #36: distinct with indexed field + $regex must apply both filters
#[test]
fn test_distinct_with_regex_filter() {
    let db = create_test_db();
    let coll = db.get_collection("docs").unwrap();

    // distinct doc_type where content matches $regex
    let values = coll
        .distinct("doc_type", &json!({"content": {"$regex": "Invoice"}}))
        .unwrap();

    assert_eq!(values.len(), 1, "Only 'invoice' docs contain 'Invoice'");
    assert_eq!(values[0], json!("invoice"));
}

/// Ensure single-field queries still use the fast index count (no regression)
#[test]
fn test_count_single_field_still_uses_index() {
    let db = create_test_db();
    let coll = db.get_collection("docs").unwrap();

    // This should still be O(1) — fully covered by index
    let count = coll
        .count_documents(&json!({"doc_type": "report"}))
        .unwrap();
    assert_eq!(count, 3);

    let count_invoice = coll
        .count_documents(&json!({"doc_type": "invoice"}))
        .unwrap();
    assert_eq!(count_invoice, 2);
}

/// FIX #35/#36: $exists combined with indexed field must post-filter
#[test]
fn test_count_exists_with_indexed_field() {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    let _ = db.collection("mixed").unwrap();

    // Insert docs: some have "date" field, some don't
    for i in 0..20 {
        let mut doc = HashMap::from([
            ("category".to_string(), json!("A")),
            ("name".to_string(), json!(format!("doc_{}", i))),
        ]);
        if i % 2 == 0 {
            doc.insert("date".to_string(), json!("2026-01-01"));
        }
        db.insert_one("mixed", doc).unwrap();
    }

    let coll = db.get_collection("mixed").unwrap();
    coll.create_index("category".to_string(), false, false)
        .unwrap();

    // count category=A AND date exists — should be 10 (even-numbered docs)
    let count_with_date = coll
        .count_documents(&json!({
            "category": "A",
            "date": {"$exists": true}
        }))
        .unwrap();
    assert_eq!(
        count_with_date, 10,
        "10 docs have category=A AND date exists — was returning 20 before fix"
    );

    // count category=A AND date not exists — should be 10 (odd-numbered docs)
    let count_without_date = coll
        .count_documents(&json!({
            "category": "A",
            "date": {"$exists": false}
        }))
        .unwrap();
    assert_eq!(
        count_without_date, 10,
        "10 docs have category=A AND date NOT exists — was returning 20 before fix"
    );
}
