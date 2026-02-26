//! Regression tests for count_documents stale index entry validation.
//!
//! When a btree index contains stale entries (doc_ids not in the document_catalog),
//! count_documents must validate entries against the catalog instead of trusting
//! the raw index count. This prevents overcounting caused by dirty-marking bugs
//! (265f0c2e) or hard restarts.

use ironbase_core::document::DocumentId;
use ironbase_core::index::IndexKey;
use ironbase_core::storage::MemoryStorage;
use ironbase_core::DatabaseCore;
use serde_json::json;

/// Helper: create a test DB with documents and a "year" index, then inject stale entries.
fn create_db_with_stale_entries() -> DatabaseCore<MemoryStorage> {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    let _ = db.collection("items").unwrap();

    // Insert 10 real documents: 5 with year=2024, 5 with year=2025
    for i in 0..5 {
        db.insert_one(
            "items",
            std::collections::HashMap::from([
                ("year".to_string(), json!(2024)),
                ("name".to_string(), json!(format!("doc_2024_{}", i))),
            ]),
        )
        .unwrap();
    }
    for i in 0..5 {
        db.insert_one(
            "items",
            std::collections::HashMap::from([
                ("year".to_string(), json!(2025)),
                ("name".to_string(), json!(format!("doc_2025_{}", i))),
            ]),
        )
        .unwrap();
    }

    // Create index on "year"
    let coll = db.get_collection("items").unwrap();
    coll.create_index("year".to_string(), false, false).unwrap();

    // Inject stale entries: doc_ids that do NOT exist in the catalog
    {
        let mut indexes = coll.indexes.write();
        let btree = indexes.get_btree_index_mut("items_year").unwrap();
        // 3 stale entries for year=2024
        for i in 100..103 {
            btree
                .insert(IndexKey::Int(2024), DocumentId::Int(i))
                .unwrap();
        }
        // 2 stale entries for year=2025
        for i in 200..202 {
            btree
                .insert(IndexKey::Int(2025), DocumentId::Int(i))
                .unwrap();
        }
    }

    db
}

/// Test: IndexScan (exact match) validates against catalog.
/// Query: {"year": 2024} — should return 5 (not 5+3=8 stale).
#[test]
fn test_count_stale_index_scan_validates_against_catalog() {
    let db = create_db_with_stale_entries();
    let coll = db.get_collection("items").unwrap();

    let count = coll.count_documents(&json!({"year": 2024})).unwrap();
    assert_eq!(
        count, 5,
        "IndexScan count must validate against catalog — expected 5 real docs, got {} (stale entries not filtered)",
        count
    );

    let count_2025 = coll.count_documents(&json!({"year": 2025})).unwrap();
    assert_eq!(
        count_2025, 5,
        "IndexScan count for 2025 must be 5 — got {} (stale entries not filtered)",
        count_2025
    );
}

/// Test: IndexRangeScan ($gte) validates against catalog.
/// Query: {"year": {"$gte": 2025}} — should return 5 (not 5+2=7 stale).
#[test]
fn test_count_stale_range_scan_validates_against_catalog() {
    let db = create_db_with_stale_entries();
    let coll = db.get_collection("items").unwrap();

    let count = coll
        .count_documents(&json!({"year": {"$gte": 2025}}))
        .unwrap();
    assert_eq!(
        count, 5,
        "IndexRangeScan count must validate against catalog — expected 5, got {} (stale entries not filtered)",
        count
    );

    // Range covering both years
    let count_all = coll
        .count_documents(&json!({"year": {"$gte": 2024}}))
        .unwrap();
    assert_eq!(
        count_all, 10,
        "IndexRangeScan count for $gte 2024 must be 10 — got {} (stale entries not filtered)",
        count_all
    );
}

/// Test: MultiValueScan ($in) validates against catalog.
/// Query: {"year": {"$in": [2024, 2025]}} — should return 10 (not 10+5=15 stale).
#[test]
fn test_count_stale_multi_value_validates_against_catalog() {
    let db = create_db_with_stale_entries();
    let coll = db.get_collection("items").unwrap();

    let count = coll
        .count_documents(&json!({"year": {"$in": [2024, 2025]}}))
        .unwrap();
    assert_eq!(
        count, 10,
        "MultiValueScan count must validate against catalog — expected 10, got {} (stale entries not filtered)",
        count
    );

    // Single value via $in
    let count_single = coll
        .count_documents(&json!({"year": {"$in": [2024]}}))
        .unwrap();
    assert_eq!(
        count_single, 5,
        "MultiValueScan single-value count must be 5 — got {} (stale entries not filtered)",
        count_single
    );
}
