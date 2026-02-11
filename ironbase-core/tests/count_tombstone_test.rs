//! Regression test for GitHub issue #30:
//! count_documents includes tombstoned (deleted) documents in count
//!
//! Scenario: insert 40, delete all, insert 20 → count should be 20, not 22

use std::collections::HashMap;

use ironbase_core::storage::MemoryStorage;
use ironbase_core::DatabaseCore;
use serde_json::json;

fn create_memory_db() -> (DatabaseCore<MemoryStorage>, String) {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    let coll_name = "test".to_string();
    let _ = db.collection(&coll_name).unwrap();
    (db, coll_name)
}

/// Exact reproduction of issue #30:
/// 1. Insert 40 docs
/// 2. Delete all 40 with delete_many
/// 3. Insert 20 new docs
/// 4. count_documents({}) should equal find({}).len()
#[test]
fn test_count_after_delete_all_and_reinsert() {
    let (db, coll_name) = create_memory_db();

    // Step 1: Insert 40 documents
    for i in 0..40 {
        db.insert_one(
            &coll_name,
            HashMap::from([
                ("name".to_string(), json!(format!("doc_{}", i))),
                ("value".to_string(), json!(i)),
            ]),
        )
        .unwrap();
    }

    let count_after_insert = db
        .collection(&coll_name)
        .unwrap()
        .count_documents(&json!({}))
        .unwrap();
    assert_eq!(count_after_insert, 40, "Should have 40 docs after insert");

    // Step 2: Delete all 40
    let deleted = db.delete_many(&coll_name, &json!({})).unwrap();
    assert_eq!(deleted, 40, "Should delete 40 docs");

    let count_after_delete = db
        .collection(&coll_name)
        .unwrap()
        .count_documents(&json!({}))
        .unwrap();
    assert_eq!(
        count_after_delete, 0,
        "count_documents should be 0 after deleting all"
    );

    // Step 3: Insert 20 new documents
    for i in 0..20 {
        db.insert_one(
            &coll_name,
            HashMap::from([
                ("name".to_string(), json!(format!("new_doc_{}", i))),
                ("value".to_string(), json!(100 + i)),
            ]),
        )
        .unwrap();
    }

    // Step 4: count_documents({}) should match find({}).len()
    let count = db
        .collection(&coll_name)
        .unwrap()
        .count_documents(&json!({}))
        .unwrap();

    let find_results = db.collection(&coll_name).unwrap().find(&json!({})).unwrap();
    let find_count = find_results.len() as u64;

    assert_eq!(
        count, find_count,
        "count_documents({{}}) = {} but find({{}}).len() = {}. \
         Tombstoned docs are being counted!",
        count, find_count
    );
    assert_eq!(count, 20, "Should have exactly 20 docs");
}

/// Variant: delete_one in a loop instead of delete_many
#[test]
fn test_count_after_individual_deletes_and_reinsert() {
    let (db, coll_name) = create_memory_db();

    // Insert 10 docs
    for i in 0..10 {
        db.insert_one(&coll_name, HashMap::from([("value".to_string(), json!(i))]))
            .unwrap();
    }

    // Delete each one individually
    for i in 0..10 {
        let deleted = db.delete_one(&coll_name, &json!({"value": i})).unwrap();
        assert_eq!(deleted, 1, "Should delete doc with value={}", i);
    }

    let count_after_delete = db
        .collection(&coll_name)
        .unwrap()
        .count_documents(&json!({}))
        .unwrap();
    assert_eq!(count_after_delete, 0, "count should be 0 after all deletes");

    // Insert 5 new docs
    for i in 0..5 {
        db.insert_one(
            &coll_name,
            HashMap::from([("value".to_string(), json!(100 + i))]),
        )
        .unwrap();
    }

    let count = db
        .collection(&coll_name)
        .unwrap()
        .count_documents(&json!({}))
        .unwrap();
    let find_count = db
        .collection(&coll_name)
        .unwrap()
        .find(&json!({}))
        .unwrap()
        .len() as u64;

    assert_eq!(
        count, find_count,
        "count_documents = {} but find.len() = {} after individual deletes + reinsert",
        count, find_count
    );
    assert_eq!(count, 5);
}

/// Test that filtered count also handles tombstones correctly
#[test]
fn test_filtered_count_with_tombstones() {
    let (db, coll_name) = create_memory_db();
    let collection = db.collection(&coll_name).unwrap();

    // Insert 20 docs with status field
    for i in 0..20 {
        db.insert_one(
            &coll_name,
            HashMap::from([
                (
                    "status".to_string(),
                    json!(if i % 2 == 0 { "active" } else { "inactive" }),
                ),
                ("value".to_string(), json!(i)),
            ]),
        )
        .unwrap();
    }

    // Create index on status
    collection
        .create_index("status".to_string(), false, false)
        .unwrap();

    // Delete all inactive
    let deleted = db
        .delete_many(&coll_name, &json!({"status": "inactive"}))
        .unwrap();
    assert_eq!(deleted, 10);

    // Insert 5 more active docs
    for i in 0..5 {
        db.insert_one(
            &coll_name,
            HashMap::from([
                ("status".to_string(), json!("active")),
                ("value".to_string(), json!(100 + i)),
            ]),
        )
        .unwrap();
    }

    // Total count should be 15 (10 original active + 5 new active)
    let total_count = collection.count_documents(&json!({})).unwrap();
    let total_find = collection.find(&json!({})).unwrap().len() as u64;
    assert_eq!(
        total_count, total_find,
        "Total: count={} but find={}",
        total_count, total_find
    );

    // Filtered count for "active" should also be 15
    let active_count = collection
        .count_documents(&json!({"status": "active"}))
        .unwrap();
    let active_find = collection.find(&json!({"status": "active"})).unwrap().len() as u64;
    assert_eq!(
        active_count, active_find,
        "Active: count={} but find={}",
        active_count, active_find
    );
    assert_eq!(active_count, 15);
}

/// Test adjust_count_for_tombstones ratio with index-based count
#[test]
fn test_index_count_with_tombstones_in_catalog() {
    let (db, coll_name) = create_memory_db();
    let collection = db.collection(&coll_name).unwrap();

    // Insert 40 docs with category field
    for i in 0..40 {
        db.insert_one(
            &coll_name,
            HashMap::from([
                (
                    "category".to_string(),
                    json!(if i < 20 { "A" } else { "B" }),
                ),
                ("value".to_string(), json!(i)),
            ]),
        )
        .unwrap();
    }

    // Create index on category
    collection
        .create_index("category".to_string(), false, false)
        .unwrap();

    // Delete all category B (20 docs)
    let deleted = db
        .delete_many(&coll_name, &json!({"category": "B"}))
        .unwrap();
    assert_eq!(deleted, 20);

    // Count category A via index — should be 20, not affected by B tombstones
    let count_a = collection
        .count_documents(&json!({"category": "A"}))
        .unwrap();
    let find_a = collection.find(&json!({"category": "A"})).unwrap().len() as u64;
    assert_eq!(
        count_a, find_a,
        "Category A: count={} but find={} (tombstone ratio bug?)",
        count_a, find_a
    );
    assert_eq!(count_a, 20);
}
