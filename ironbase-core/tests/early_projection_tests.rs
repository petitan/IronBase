//! Tests for early projection optimization
//!
//! Early projection applies the projection immediately after document loading
//! (before size check and sort) when it's safe to do so. This reduces memory
//! usage and enables accurate response size estimation.

use ironbase_core::storage::MemoryStorage;
use ironbase_core::{DatabaseCore, FindOptions};
use serde_json::json;
use std::collections::HashMap;

fn create_memory_db() -> DatabaseCore<MemoryStorage> {
    DatabaseCore::<MemoryStorage>::open_memory().unwrap()
}

fn insert_doc(db: &DatabaseCore<MemoryStorage>, coll: &str, doc: serde_json::Value) {
    let map: HashMap<String, serde_json::Value> = doc
        .as_object()
        .unwrap()
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    db.insert_one(coll, map).unwrap();
}

// ========== EARLY PROJECTION BASIC TESTS ==========

#[test]
fn test_early_projection_no_sort() {
    // Early projection should be applied when there's no sort
    let db = create_memory_db();
    let coll_name = "test";

    // Insert documents with multiple fields
    for i in 0..10 {
        insert_doc(
            &db,
            coll_name,
            json!({
                "name": format!("User{}", i),
                "age": 20 + i,
                "email": format!("user{}@example.com", i),
                "large_data": "x".repeat(1000)
            }),
        );
    }

    let coll = db.collection(coll_name).unwrap();

    // Query with projection (no sort)
    let options = FindOptions::new()
        .with_projection(HashMap::from([
            ("name".to_string(), 1),
            ("age".to_string(), 1),
        ]))
        .with_limit(5);

    let results = coll.find_with_options(&json!({}), options).unwrap();

    assert_eq!(results.len(), 5);
    for doc in &results {
        assert!(doc.get("name").is_some());
        assert!(doc.get("age").is_some());
        assert!(doc.get("_id").is_some()); // _id included by default
        assert!(doc.get("email").is_none()); // Not in projection
        assert!(doc.get("large_data").is_none()); // Not in projection
    }
}

/// Fast-path projection (review fix): an `_id` query with a projection must APPLY
/// the projection — the `_id` fast path previously returned the FULL document —
/// and the result must be identical with vs without a sort (fast path vs slow path).
#[test]
fn test_id_fast_path_applies_projection() {
    let db = create_memory_db();
    let coll_name = "test";
    let id = db
        .insert_one(
            coll_name,
            HashMap::from([
                ("name".to_string(), json!("Alice")),
                ("secret".to_string(), json!("hidden")),
                ("age".to_string(), json!(30)),
            ]),
        )
        .unwrap();
    let id_q = match id {
        ironbase_core::DocumentId::Int(n) => json!(n),
        ironbase_core::DocumentId::String(s) => json!(s),
        other => panic!("unexpected id type: {:?}", other),
    };

    let coll = db.collection(coll_name).unwrap();
    let proj = HashMap::from([("name".to_string(), 1)]);

    // _id fast path (no sort): projection MUST apply → only _id + name.
    let fast = coll
        .find_with_options(
            &json!({ "_id": id_q }),
            FindOptions::new().with_projection(proj.clone()),
        )
        .unwrap();
    assert_eq!(fast.len(), 1);
    assert!(fast[0].get("name").is_some());
    assert!(fast[0].get("_id").is_some());
    assert!(
        fast[0].get("secret").is_none(),
        "secret must be projected out on the _id fast path"
    );
    assert!(fast[0].get("age").is_none());

    // Same query WITH a sort → slow path; the shape must be identical.
    let slow = coll
        .find_with_options(
            &json!({ "_id": id_q }),
            FindOptions::new()
                .with_projection(proj)
                .with_sort(vec![("name".to_string(), 1)]),
        )
        .unwrap();
    assert_eq!(
        slow, fast,
        "fast-path and slow-path _id projection must agree"
    );
}

#[test]
fn test_early_projection_sort_included_in_projection() {
    // Early projection should be applied when sort field is in projection
    let db = create_memory_db();
    let coll_name = "test";

    insert_doc(&db, coll_name, json!({"name": "Charlie", "score": 85}));
    insert_doc(&db, coll_name, json!({"name": "Alice", "score": 95}));
    insert_doc(&db, coll_name, json!({"name": "Bob", "score": 90}));

    let coll = db.collection(coll_name).unwrap();

    // Query with projection AND sort, where sort field IS in projection
    let options = FindOptions::new()
        .with_projection(HashMap::from([
            ("name".to_string(), 1),
            ("score".to_string(), 1),
        ]))
        .with_sort(vec![("score".to_string(), -1)]); // Descending by score

    let results = coll.find_with_options(&json!({}), options).unwrap();

    assert_eq!(results.len(), 3);
    // Should be sorted by score descending: Alice(95), Bob(90), Charlie(85)
    assert_eq!(results[0].get("name").unwrap(), "Alice");
    assert_eq!(results[1].get("name").unwrap(), "Bob");
    assert_eq!(results[2].get("name").unwrap(), "Charlie");
}

#[test]
fn test_late_projection_sort_excluded_from_projection() {
    // Late projection should be used when sort field is NOT in projection
    let db = create_memory_db();
    let coll_name = "test";

    insert_doc(
        &db,
        coll_name,
        json!({"name": "Charlie", "score": 85, "city": "NYC"}),
    );
    insert_doc(
        &db,
        coll_name,
        json!({"name": "Alice", "score": 95, "city": "LA"}),
    );
    insert_doc(
        &db,
        coll_name,
        json!({"name": "Bob", "score": 90, "city": "SF"}),
    );

    let coll = db.collection(coll_name).unwrap();

    // Query: project only name, but sort by score (NOT in projection)
    let options = FindOptions::new()
        .with_projection(HashMap::from([("name".to_string(), 1)]))
        .with_sort(vec![("score".to_string(), -1)]); // Sort by score, but score not in projection

    let results = coll.find_with_options(&json!({}), options).unwrap();

    assert_eq!(results.len(), 3);
    // Should still be sorted by score (even though score not in output)
    assert_eq!(results[0].get("name").unwrap(), "Alice"); // score: 95
    assert_eq!(results[1].get("name").unwrap(), "Bob"); // score: 90
    assert_eq!(results[2].get("name").unwrap(), "Charlie"); // score: 85

    // Score should NOT be in output
    assert!(results[0].get("score").is_none());
}

// ========== MAX_RESPONSE_BYTES WITH EARLY PROJECTION ==========

#[test]
fn test_early_projection_respects_max_response_bytes() {
    // With early projection, size check should use projected size, not full doc size
    let db = create_memory_db();
    let coll_name = "test";

    // Insert documents with large data
    for i in 0..10 {
        insert_doc(
            &db,
            coll_name,
            json!({
                "id": i,
                "small_field": format!("small{}", i),
                "large_data": "x".repeat(100_000) // 100KB per doc
            }),
        );
    }

    let coll = db.collection(coll_name).unwrap();

    // Set a small max_response_bytes limit (10KB)
    // Without early projection: would fail after ~1 doc (100KB > 10KB)
    // With early projection: should succeed (projected doc is tiny)
    let options = FindOptions::new()
        .with_projection(HashMap::from([
            ("id".to_string(), 1),
            ("small_field".to_string(), 1),
        ]))
        .with_max_response_bytes(10_000) // 10KB limit
        .with_limit(5);

    let results = coll.find_with_options(&json!({}), options).unwrap();

    // Should succeed because projected docs are small
    assert_eq!(results.len(), 5);
    for doc in &results {
        assert!(doc.get("id").is_some());
        assert!(doc.get("small_field").is_some());
        assert!(doc.get("large_data").is_none()); // Excluded by projection
    }
}

#[test]
fn test_late_projection_large_docs_with_sort() {
    // When sort field is excluded, late projection is used
    // Size check uses full doc size, may fail with large docs
    let db = create_memory_db();
    let coll_name = "test";

    // Insert documents with large data
    for i in 0..5 {
        insert_doc(
            &db,
            coll_name,
            json!({
                "id": i,
                "score": 100 - i * 10,
                "large_data": "x".repeat(50_000) // 50KB per doc
            }),
        );
    }

    let coll = db.collection(coll_name).unwrap();

    // Sort by score (not in projection), with small max_response_bytes
    // This forces late projection, so size check sees full doc
    let options = FindOptions::new()
        .with_projection(HashMap::from([("id".to_string(), 1)]))
        .with_sort(vec![("score".to_string(), -1)])
        .with_max_response_bytes(100_000) // 100KB limit
        .with_limit(5);

    // Should fail because late projection checks full doc size
    let result = coll.find_with_options(&json!({}), options);

    // With 5 docs at 50KB each = 250KB, exceeds 100KB limit
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("Response size limit exceeded"));
}

// ========== EXCLUDE MODE PROJECTION ==========

#[test]
fn test_early_projection_exclude_mode() {
    // Exclude mode: exclude specific fields
    let db = create_memory_db();
    let coll_name = "test";

    insert_doc(
        &db,
        coll_name,
        json!({
            "name": "Alice",
            "password": "secret123",
            "email": "alice@example.com"
        }),
    );

    let coll = db.collection(coll_name).unwrap();

    // Exclude password field (no sort)
    let options = FindOptions::new().with_projection(HashMap::from([("password".to_string(), 0)]));

    let results = coll.find_with_options(&json!({}), options).unwrap();

    assert_eq!(results.len(), 1);
    assert!(results[0].get("name").is_some());
    assert!(results[0].get("email").is_some());
    assert!(results[0].get("password").is_none()); // Excluded
}

#[test]
fn test_early_projection_exclude_mode_with_sort() {
    // Exclude mode with sort on non-excluded field
    let db = create_memory_db();
    let coll_name = "test";

    insert_doc(
        &db,
        coll_name,
        json!({"name": "Charlie", "age": 30, "secret": "xxx"}),
    );
    insert_doc(
        &db,
        coll_name,
        json!({"name": "Alice", "age": 25, "secret": "yyy"}),
    );
    insert_doc(
        &db,
        coll_name,
        json!({"name": "Bob", "age": 35, "secret": "zzz"}),
    );

    let coll = db.collection(coll_name).unwrap();

    // Exclude secret, sort by age (age is NOT excluded, so early projection is safe)
    let options = FindOptions::new()
        .with_projection(HashMap::from([("secret".to_string(), 0)]))
        .with_sort(vec![("age".to_string(), 1)]); // Ascending

    let results = coll.find_with_options(&json!({}), options).unwrap();

    assert_eq!(results.len(), 3);
    // Sorted by age: Alice(25), Charlie(30), Bob(35)
    assert_eq!(results[0].get("name").unwrap(), "Alice");
    assert_eq!(results[1].get("name").unwrap(), "Charlie");
    assert_eq!(results[2].get("name").unwrap(), "Bob");

    // Secret should be excluded
    for doc in &results {
        assert!(doc.get("secret").is_none());
    }
}

#[test]
fn test_late_projection_exclude_mode_sort_on_excluded() {
    // Exclude mode: sort on excluded field → late projection
    let db = create_memory_db();
    let coll_name = "test";

    insert_doc(&db, coll_name, json!({"name": "Charlie", "score": 85}));
    insert_doc(&db, coll_name, json!({"name": "Alice", "score": 95}));
    insert_doc(&db, coll_name, json!({"name": "Bob", "score": 90}));

    let coll = db.collection(coll_name).unwrap();

    // Exclude score, but sort BY score → late projection required
    let options = FindOptions::new()
        .with_projection(HashMap::from([("score".to_string(), 0)]))
        .with_sort(vec![("score".to_string(), -1)]); // Sort by excluded field

    let results = coll.find_with_options(&json!({}), options).unwrap();

    // Should still sort correctly (late projection)
    assert_eq!(results.len(), 3);
    assert_eq!(results[0].get("name").unwrap(), "Alice"); // score: 95
    assert_eq!(results[1].get("name").unwrap(), "Bob"); // score: 90
    assert_eq!(results[2].get("name").unwrap(), "Charlie"); // score: 85

    // Score should be excluded in output
    for doc in &results {
        assert!(doc.get("score").is_none());
    }
}

// ========== EDGE CASES ==========

#[test]
fn test_no_projection_no_early_project() {
    // Without projection, early_project should be false
    let db = create_memory_db();
    let coll_name = "test";

    insert_doc(&db, coll_name, json!({"name": "Alice", "age": 30}));

    let coll = db.collection(coll_name).unwrap();

    let options = FindOptions::new().with_limit(1);

    let results = coll.find_with_options(&json!({}), options).unwrap();

    assert_eq!(results.len(), 1);
    assert!(results[0].get("name").is_some());
    assert!(results[0].get("age").is_some());
}

#[test]
fn test_multi_field_sort_all_in_projection() {
    // Multi-field sort where ALL sort fields are in projection
    let db = create_memory_db();
    let coll_name = "test";

    insert_doc(
        &db,
        coll_name,
        json!({"name": "Alice", "dept": "Sales", "score": 90}),
    );
    insert_doc(
        &db,
        coll_name,
        json!({"name": "Bob", "dept": "Sales", "score": 85}),
    );
    insert_doc(
        &db,
        coll_name,
        json!({"name": "Charlie", "dept": "HR", "score": 95}),
    );

    let coll = db.collection(coll_name).unwrap();

    // Sort by dept, then score - both in projection
    let options = FindOptions::new()
        .with_projection(HashMap::from([
            ("name".to_string(), 1),
            ("dept".to_string(), 1),
            ("score".to_string(), 1),
        ]))
        .with_sort(vec![
            ("dept".to_string(), 1),   // Ascending
            ("score".to_string(), -1), // Descending
        ]);

    let results = coll.find_with_options(&json!({}), options).unwrap();

    assert_eq!(results.len(), 3);
    // HR first, then Sales (sorted by score desc within dept)
    assert_eq!(results[0].get("dept").unwrap(), "HR");
    assert_eq!(results[0].get("name").unwrap(), "Charlie");
    assert_eq!(results[1].get("dept").unwrap(), "Sales");
    assert_eq!(results[1].get("name").unwrap(), "Alice"); // 90 > 85
    assert_eq!(results[2].get("dept").unwrap(), "Sales");
    assert_eq!(results[2].get("name").unwrap(), "Bob");
}

#[test]
fn test_multi_field_sort_one_excluded() {
    // Multi-field sort where ONE sort field is not in projection → late projection
    let db = create_memory_db();
    let coll_name = "test";

    insert_doc(
        &db,
        coll_name,
        json!({"name": "Alice", "dept": "Sales", "score": 90}),
    );
    insert_doc(
        &db,
        coll_name,
        json!({"name": "Bob", "dept": "Sales", "score": 85}),
    );
    insert_doc(
        &db,
        coll_name,
        json!({"name": "Charlie", "dept": "HR", "score": 95}),
    );

    let coll = db.collection(coll_name).unwrap();

    // Sort by dept and score, but only project name and dept (score excluded)
    let options = FindOptions::new()
        .with_projection(HashMap::from([
            ("name".to_string(), 1),
            ("dept".to_string(), 1),
            // score NOT in projection
        ]))
        .with_sort(vec![
            ("dept".to_string(), 1),
            ("score".to_string(), -1), // Sort by score but don't project it
        ]);

    let results = coll.find_with_options(&json!({}), options).unwrap();

    assert_eq!(results.len(), 3);
    // Should still be sorted correctly
    assert_eq!(results[0].get("dept").unwrap(), "HR");
    assert_eq!(results[1].get("dept").unwrap(), "Sales");
    assert_eq!(results[1].get("name").unwrap(), "Alice"); // 90 > 85
    assert_eq!(results[2].get("dept").unwrap(), "Sales");
    assert_eq!(results[2].get("name").unwrap(), "Bob");

    // Score should NOT be in output
    for doc in &results {
        assert!(doc.get("score").is_none());
    }
}

#[test]
fn test_projection_with_skip() {
    // Early projection with skip
    let db = create_memory_db();
    let coll_name = "test";

    for i in 0..10 {
        insert_doc(
            &db,
            coll_name,
            json!({
                "id": i,
                "large": "x".repeat(1000)
            }),
        );
    }

    let coll = db.collection(coll_name).unwrap();

    let options = FindOptions::new()
        .with_projection(HashMap::from([("id".to_string(), 1)]))
        .with_skip(5)
        .with_limit(3);

    let results = coll.find_with_options(&json!({}), options).unwrap();

    assert_eq!(results.len(), 3);
    for doc in &results {
        assert!(doc.get("id").is_some());
        assert!(doc.get("large").is_none());
    }
}

// ========== REGRESSION TEST FOR ORIGINAL ISSUE ==========

#[test]
fn test_original_issue_projection_with_max_response_bytes() {
    // This is the original issue: projection should reduce response size check
    // Before fix: full doc loaded → size check fails → projection never applied
    // After fix: projection applied first → size check uses small projected doc
    let db = create_memory_db();
    let coll_name = "test";

    // Simulate email documents with large attachments
    for i in 0..20 {
        insert_doc(
            &db,
            coll_name,
            json!({
                "_id": i,
                "message_id": format!("msg-{}", i),
                "subject": format!("Email {}", i),
                "attachment_data": "x".repeat(500_000) // 500KB "attachment"
            }),
        );
    }

    let coll = db.collection(coll_name).unwrap();

    // Original failing query: projection for small fields, skip to large docs
    let options = FindOptions::new()
        .with_projection(HashMap::from([
            ("_id".to_string(), 1),
            ("message_id".to_string(), 1),
        ]))
        .with_skip(10)
        .with_limit(10)
        .with_max_response_bytes(1_000_000); // 1MB limit

    // Before fix: would fail with "Response size limit exceeded"
    // After fix: should succeed because projected docs are tiny
    let results = coll.find_with_options(&json!({}), options).unwrap();

    assert_eq!(results.len(), 10);
    for doc in &results {
        assert!(doc.get("_id").is_some());
        assert!(doc.get("message_id").is_some());
        assert!(doc.get("subject").is_none()); // Not in projection
        assert!(doc.get("attachment_data").is_none()); // Not in projection
    }
}
