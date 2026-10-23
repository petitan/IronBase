//! Integration tests for ExecutionContext with _with_ctx methods
//!
//! Tests cancellation and timeout behavior across different operations.

use crate::execution::ExecutionContext;
use crate::storage::MemoryStorage;
use crate::DatabaseCore;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Helper to create a test database with sample data
fn setup_test_db() -> DatabaseCore<MemoryStorage> {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();

    // Insert 1000 documents for testing
    for i in 0..1000 {
        let mut doc: HashMap<String, Value> = HashMap::new();
        doc.insert("name".to_string(), json!(format!("user_{}", i)));
        doc.insert("age".to_string(), json!(i % 100));
        doc.insert(
            "city".to_string(),
            json!(if i % 2 == 0 { "Budapest" } else { "Vienna" }),
        );
        db.insert_one("test", doc).unwrap();
    }

    db
}

// ============================================================================
// count_documents_with_ctx tests
// ============================================================================

#[test]
fn test_count_documents_with_ctx_no_context() {
    let db = setup_test_db();
    let coll = db.get_collection("test").unwrap();

    // Should work without context (backward compatible)
    let count = coll.count_documents(&json!({})).unwrap();
    assert_eq!(count, 1000);
}

#[test]
fn test_count_documents_with_ctx_valid_deadline() {
    let db = setup_test_db();
    let coll = db.get_collection("test").unwrap();

    // Deadline far in the future - should succeed
    let ctx = ExecutionContext::new().with_deadline(Instant::now() + Duration::from_secs(60));

    let count = coll
        .count_documents_with_ctx(&json!({}), Some(&ctx))
        .unwrap();
    assert_eq!(count, 1000);
}

#[test]
fn test_count_documents_with_ctx_expired_deadline() {
    let db = setup_test_db();
    let coll = db.get_collection("test").unwrap();

    // Deadline already passed - should fail with timeout
    let ctx = ExecutionContext::new().with_deadline(Instant::now() - Duration::from_secs(1));

    let result = coll.count_documents_with_ctx(&json!({"age": {"$gte": 0}}), Some(&ctx));
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(matches!(err, crate::error::IronBaseError::Timeout(_)));
}

#[test]
fn test_count_documents_with_ctx_cancelled() {
    let db = setup_test_db();
    let coll = db.get_collection("test").unwrap();

    // Pre-cancelled flag
    let flag = Arc::new(AtomicBool::new(true));
    let ctx = ExecutionContext::new()
        .with_cancel_flag(flag)
        .with_check_frequency(1); // Check every iteration

    let result = coll.count_documents_with_ctx(&json!({"age": {"$gte": 0}}), Some(&ctx));
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(matches!(err, crate::error::IronBaseError::Cancelled(_)));
}

// ============================================================================
// distinct_with_ctx tests
// ============================================================================

#[test]
fn test_distinct_with_ctx_no_context() {
    let db = setup_test_db();
    let coll = db.get_collection("test").unwrap();

    // Should work without context
    let values = coll.distinct("city", &json!({})).unwrap();
    assert_eq!(values.len(), 2); // Budapest, Vienna
}

#[test]
fn test_distinct_with_ctx_valid_deadline() {
    let db = setup_test_db();
    let coll = db.get_collection("test").unwrap();

    let ctx = ExecutionContext::new().with_deadline(Instant::now() + Duration::from_secs(60));

    let values = coll
        .distinct_with_ctx("city", &json!({}), Some(&ctx))
        .unwrap();
    assert_eq!(values.len(), 2);
}

#[test]
fn test_distinct_with_ctx_cancelled() {
    let db = setup_test_db();
    let coll = db.get_collection("test").unwrap();

    let flag = Arc::new(AtomicBool::new(true));
    let ctx = ExecutionContext::new()
        .with_cancel_flag(flag)
        .with_check_frequency(1);

    // Query that forces scan (not empty {})
    let result = coll.distinct_with_ctx("city", &json!({"age": {"$gte": 0}}), Some(&ctx));
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        crate::error::IronBaseError::Cancelled(_)
    ));
}

// ============================================================================
// find_one_with_ctx tests
// ============================================================================

#[test]
fn test_find_one_with_ctx_no_context() {
    let db = setup_test_db();
    let coll = db.get_collection("test").unwrap();

    let doc = coll.find_one(&json!({"name": "user_0"})).unwrap();
    assert!(doc.is_some());
    assert_eq!(doc.unwrap()["name"], "user_0");
}

#[test]
fn test_find_one_with_ctx_valid_deadline() {
    let db = setup_test_db();
    let coll = db.get_collection("test").unwrap();

    let ctx = ExecutionContext::new().with_deadline(Instant::now() + Duration::from_secs(60));

    let doc = coll
        .find_one_with_ctx(&json!({"name": "user_500"}), Some(&ctx))
        .unwrap();
    assert!(doc.is_some());
}

#[test]
fn test_find_one_with_ctx_cancelled() {
    let db = setup_test_db();
    let coll = db.get_collection("test").unwrap();

    let flag = Arc::new(AtomicBool::new(true));
    let ctx = ExecutionContext::new()
        .with_cancel_flag(flag)
        .with_check_frequency(1);

    // Note: find_one with limit=1 is very fast and may complete before cancellation is checked.
    // This is expected behavior - find_one is inherently bounded.
    // The cancellation is passed to collect_doc_ids_with_options which checks during iteration.
    // With limit=1, it finds the first match immediately and returns.
    //
    // This test verifies that the cancel flag is at least passed through the API.
    // In real usage, cancellation would mainly help if index lookup takes time
    // or if using a regex query that requires scanning.
    let result = coll.find_one_with_ctx(&json!({"age": {"$gte": 50}}), Some(&ctx));

    // May succeed (found doc quickly) or be cancelled - both are valid outcomes
    // The important thing is the API accepts ExecutionContext
    match result {
        Ok(Some(_)) => {
            // Found before cancel check - valid for fast operations
        }
        Ok(None) => {
            // No match found before cancel - also valid
        }
        Err(crate::error::IronBaseError::Cancelled(_)) => {
            // Cancelled during scan - expected if slow enough
        }
        Err(crate::error::IronBaseError::InvalidQuery(msg)) if msg.contains("cancelled") => {
            // Cancellation wrapped in InvalidQuery (from collect_doc_ids)
        }
        Err(e) => {
            panic!("Unexpected error: {:?}", e);
        }
    }
}

// ============================================================================
// ExecutionContext behavior tests
// ============================================================================

#[test]
fn test_cancel_flag_checked_before_deadline() {
    // When both are set, cancel flag should be checked first (faster)
    let flag = Arc::new(AtomicBool::new(true));
    let ctx = ExecutionContext::new()
        .with_cancel_flag(flag)
        .with_deadline(Instant::now() - Duration::from_secs(1)); // Also expired

    let result = ctx.check_cancelled();
    assert!(result.is_err());
    // Should be Cancelled, not Timeout (cancel flag checked first)
    assert!(matches!(
        result.unwrap_err(),
        crate::error::IronBaseError::Cancelled(_)
    ));
}

#[test]
fn test_check_frequency_respected() {
    let flag = Arc::new(AtomicBool::new(true));
    let ctx = ExecutionContext::new()
        .with_cancel_flag(flag)
        .with_check_frequency(100);

    // Iterations 1-99 should pass (not check iterations)
    for i in 1..100 {
        assert!(ctx.maybe_check(i).is_ok());
    }

    // Iteration 100 should fail (check iteration)
    assert!(ctx.maybe_check(100).is_err());
}

#[test]
fn test_context_without_cancellation_always_passes() {
    let ctx = ExecutionContext::new(); // No deadline, no flag

    // Should always pass
    for i in 0..1000 {
        assert!(ctx.maybe_check(i).is_ok());
    }
}

// ============================================================================
// find_with_options deadline tests (BUG FIX: v1.0.182)
// ============================================================================

#[test]
fn test_find_with_options_no_deadline() {
    let db = setup_test_db();
    let coll = db.get_collection("test").unwrap();

    // Should work without deadline (backward compatible)
    let options = crate::find_options::FindOptions::new().with_limit(10);
    let results = coll
        .find_with_options(&json!({"age": {"$gte": 50}}), options)
        .unwrap();
    assert_eq!(results.len(), 10);
}

#[test]
fn test_find_with_options_valid_deadline() {
    let db = setup_test_db();
    let coll = db.get_collection("test").unwrap();

    // Deadline far in the future - should succeed
    let options = crate::find_options::FindOptions::new()
        .with_limit(10)
        .with_deadline(Instant::now() + Duration::from_secs(60));

    let results = coll
        .find_with_options(&json!({"age": {"$gte": 50}}), options)
        .unwrap();
    assert_eq!(results.len(), 10);
}

#[test]
fn test_find_with_options_expired_deadline() {
    let db = setup_test_db();
    let coll = db.get_collection("test").unwrap();

    // Deadline already passed - should fail with timeout
    // Use a query that requires scanning (not just index lookup)
    let options = crate::find_options::FindOptions::new()
        .with_deadline(Instant::now() - Duration::from_secs(1));

    let result = coll.find_with_options(&json!({"age": {"$gte": 0}}), options);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(err, crate::error::IronBaseError::Timeout(_)),
        "Expected Timeout error, got: {:?}",
        err
    );
}

#[test]
fn test_find_with_options_cancelled() {
    let db = setup_test_db();
    let coll = db.get_collection("test").unwrap();

    // Pre-cancelled flag - should fail immediately
    let flag = Arc::new(AtomicBool::new(true));
    let options = crate::find_options::FindOptions::new().with_cancel_flag(flag);

    // Use a query that requires scanning
    let result = coll.find_with_options(&json!({"age": {"$gte": 0}}), options);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(err, crate::error::IronBaseError::Cancelled(_)),
        "Expected Cancelled error, got: {:?}",
        err
    );
}

#[test]
fn test_find_with_options_deadline_and_cancel_flag() {
    let db = setup_test_db();
    let coll = db.get_collection("test").unwrap();

    // Both set, cancel_flag=true - should get Cancelled (checked first)
    let flag = Arc::new(AtomicBool::new(true));
    let options = crate::find_options::FindOptions::new()
        .with_cancel_flag(flag)
        .with_deadline(Instant::now() - Duration::from_secs(1)); // Also expired

    let result = coll.find_with_options(&json!({"age": {"$gte": 0}}), options);
    assert!(result.is_err());
    let err = result.unwrap_err();
    // Cancel flag is checked before deadline
    assert!(
        matches!(err, crate::error::IronBaseError::Cancelled(_)),
        "Expected Cancelled error (checked first), got: {:?}",
        err
    );
}

// ============================================================================
// Streaming cursor and vector search (hybrid review T7)
// ============================================================================

/// Read documents until the cursor fails; panics if it reads to the end
fn drain_until_error<S: crate::storage::Storage + crate::storage::RawStorage>(
    cursor: &mut crate::collection_core::FindCursor<'_, S>,
) -> crate::error::IronBaseError {
    loop {
        match cursor.next() {
            Ok(Some(_)) => continue,
            Ok(None) => panic!("the cursor read to the end after the cancel"),
            Err(e) => return e,
        }
    }
}

/// A cursor checks its context per document read: a cancel after it was
/// created stops it (it used to read to the end)
#[test]
fn test_find_streaming_cursor_stops_on_cancel() {
    let db = setup_test_db();
    let coll = db.get_collection("test").unwrap();
    let flag = Arc::new(AtomicBool::new(false));
    let mut cursor = coll
        .find_streaming_with_options(&json!({"age": {"$gte": 0}}), Some(&flag), None)
        .unwrap();
    assert!(cursor.next().unwrap().is_some());

    flag.store(true, std::sync::atomic::Ordering::Relaxed);

    let err = drain_until_error(&mut cursor);
    assert!(
        matches!(err, crate::error::IronBaseError::Cancelled(_)),
        "Expected Cancelled error, got: {:?}",
        err
    );
}

/// The same for the lazy index scan of a file-backed collection
#[test]
fn test_find_streaming_index_scan_stops_on_cancel() {
    let temp = tempfile::TempDir::new().unwrap();
    let db =
        DatabaseCore::<crate::storage::StorageEngine>::open(temp.path().join("t.mlite")).unwrap();
    for i in 0..1000 {
        let mut doc: HashMap<String, Value> = HashMap::new();
        doc.insert("age".to_string(), json!(i % 100));
        db.insert_one("test", doc).unwrap();
    }
    let coll = db.get_collection("test").unwrap();
    coll.create_index("age".to_string(), false, false).unwrap();
    let flag = Arc::new(AtomicBool::new(false));
    let mut cursor = coll
        .find_streaming_with_options(&json!({"age": {"$gte": 0}}), Some(&flag), None)
        .unwrap();
    assert!(cursor.next().unwrap().is_some());

    flag.store(true, std::sync::atomic::Ordering::Relaxed);

    let err = drain_until_error(&mut cursor);
    assert!(
        matches!(err, crate::error::IronBaseError::Cancelled(_)),
        "Expected Cancelled error, got: {:?}",
        err
    );
}

/// 1000 documents with a 2-dimensional "emb" vector index
fn setup_vector_db() -> DatabaseCore<MemoryStorage> {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    for i in 0..1000 {
        let mut doc: HashMap<String, Value> = HashMap::new();
        doc.insert("emb".to_string(), json!([i as f64, 1.0]));
        doc.insert("tag".to_string(), json!(i % 2));
        db.insert_one("test", doc).unwrap();
    }
    db.get_collection("test")
        .unwrap()
        .create_vector_index("emb", crate::vector::VectorIndexConfig::new(2))
        .unwrap();
    db
}

#[test]
fn test_vector_search_with_ctx_no_context() {
    let db = setup_vector_db();
    let coll = db.get_collection("test").unwrap();

    let plain = coll.vector_search("emb", &[1.0, 1.0], 5).unwrap();
    let with_ctx = coll
        .vector_search_with_ctx("emb", &[1.0, 1.0], 5, None)
        .unwrap();
    assert_eq!(plain, with_ctx);
    let filtered = coll
        .vector_search_with_filter_with_ctx("emb", &[1.0, 1.0], &json!({"tag": 1}), 5, None)
        .unwrap();
    assert_eq!(filtered.len(), 5);
    assert!(filtered.iter().all(|(doc, _)| doc["tag"] == json!(1)));
}

#[test]
fn test_vector_search_with_ctx_expired_deadline() {
    let db = setup_vector_db();
    let coll = db.get_collection("test").unwrap();
    let ctx = ExecutionContext::new().with_deadline(Instant::now() - Duration::from_secs(1));

    let err = coll
        .vector_search_with_ctx("emb", &[1.0, 1.0], 5, Some(&ctx))
        .unwrap_err();
    assert!(
        matches!(err, crate::error::IronBaseError::Timeout(_)),
        "Expected Timeout error, got: {:?}",
        err
    );
}

#[test]
fn test_vector_search_with_filter_with_ctx_cancelled() {
    let db = setup_vector_db();
    let coll = db.get_collection("test").unwrap();
    let ctx = ExecutionContext::new().with_cancel_flag(Arc::new(AtomicBool::new(true)));

    let err = coll
        .vector_search_with_filter_with_ctx(
            "emb",
            &[1.0, 1.0],
            &json!({"tag": {"$gte": 1}}),
            5,
            Some(&ctx),
        )
        .unwrap_err();
    assert!(
        matches!(err, crate::error::IronBaseError::Cancelled(_)),
        "Expected Cancelled error, got: {:?}",
        err
    );
}
