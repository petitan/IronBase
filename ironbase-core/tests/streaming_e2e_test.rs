//! End-to-end streaming test for aggregate_with_context

use ironbase_core::aggregation::{AggregationLimitContext, AggregationLimits};
use ironbase_core::{storage::MemoryStorage, DatabaseCore};
use serde_json::json;
use std::collections::HashMap;

fn json_to_hashmap(v: serde_json::Value) -> HashMap<String, serde_json::Value> {
    match v {
        serde_json::Value::Object(map) => map.into_iter().collect(),
        _ => panic!("Expected object"),
    }
}

/// Test: If streaming works, we can process more docs than the doc limit
/// because documents are dropped immediately after processing in $group.
#[test]
fn test_aggregate_with_context_is_truly_streaming() {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();

    // Insert 10,000 documents with 100 unique groups
    println!("Inserting 10,000 documents...");
    for i in 0..10_000 {
        db.insert_one(
            "test",
            json_to_hashmap(json!({"group": i % 100, "value": i})),
        )
        .unwrap();
    }
    println!("Done inserting.");

    let coll = db.collection("test").unwrap();

    // Set a VERY LOW document limit (100) but allow enough groups
    let limits = AggregationLimits {
        max_docs_without_match: 100, // Only 100 docs allowed!
        max_docs_with_match: 100,    // Even with match
        max_group_count: 200,        // But 200 groups allowed
        ..Default::default()
    };

    let ctx = AggregationLimitContext::new(limits);

    // This pipeline groups 10K docs into 100 groups
    let pipeline = json!([
        {"$group": {"_id": "$group", "count": {"$sum": 1}}}
    ]);

    println!("Running aggregate_with_context with 100 doc limit on 10K docs...");
    let result = coll.aggregate_with_context(&pipeline, &ctx);

    match result {
        Ok(groups) => {
            println!("SUCCESS! Got {} groups", groups.len());
            assert_eq!(groups.len(), 100, "Should have 100 groups");

            // Verify total count is 10000
            let total: i64 = groups
                .iter()
                .map(|g| g.get("count").and_then(|v| v.as_i64()).unwrap_or(0))
                .sum();
            assert_eq!(total, 10000, "Total count should be 10000");

            println!("PROOF: 10,000 documents processed with 100 doc limit!");
            println!("This is only possible because streaming works correctly.");
        }
        Err(e) => {
            panic!(
                "FAILED: Streaming is NOT working!\n\
                 Error: {}\n\n\
                 If you see 'exceeded document limit', it means documents are being\n\
                 collected into Vec BEFORE $group, which defeats streaming.",
                e
            );
        }
    }
}

/// Test that aggregate respects deadline timeout
///
/// FIX: Timeout propagation from AggregationLimitContext to find_streaming
/// This ensures long-running aggregate queries can be cancelled.
#[test]
fn test_aggregate_with_context_respects_deadline() {
    use std::time::{Duration, Instant};

    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();

    // Insert 5,000 documents - enough to make collection scan take time
    for i in 0..5_000 {
        db.insert_one(
            "test_timeout",
            json_to_hashmap(json!({
                "x": i,
                "name": format!("item_{}", i),
                // Add some bulk to slow down processing
                "data": "x".repeat(100)
            })),
        )
        .unwrap();
    }

    let coll = db.collection("test_timeout").unwrap();
    let limits = AggregationLimits::default();

    // Set a deadline that's already in the past (1ms ago)
    // This simulates an expired timeout
    let ctx = AggregationLimitContext::new(limits)
        .with_deadline(Instant::now() - Duration::from_millis(1));

    // Pipeline that requires full collection scan (no index on "nonexistent")
    // The $exists check forces document-by-document evaluation
    let pipeline = json!([
        {"$match": {"nonexistent_field": {"$exists": true}}},
        {"$limit": 5}
    ]);

    let result = coll.aggregate_with_context(&pipeline, &ctx);

    // Should fail with timeout error
    assert!(result.is_err(), "Should fail due to deadline timeout");
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("timed out") || err.contains("deadline"),
        "Error should mention timeout: {}",
        err
    );
}

/// Test that aggregate completes normally when deadline is far in the future
#[test]
fn test_aggregate_with_context_completes_before_deadline() {
    use std::time::{Duration, Instant};

    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();

    // Insert some documents
    for i in 0..100 {
        db.insert_one("test_no_timeout", json_to_hashmap(json!({"x": i})))
            .unwrap();
    }

    let coll = db.collection("test_no_timeout").unwrap();
    let limits = AggregationLimits::default();

    // Set a generous deadline (60 seconds)
    let ctx = AggregationLimitContext::new(limits)
        .with_deadline(Instant::now() + Duration::from_secs(60));

    let pipeline = json!([
        {"$match": {"x": {"$gte": 50}}},
        {"$group": {"_id": null, "count": {"$sum": 1}}}
    ]);

    let result = coll.aggregate_with_context(&pipeline, &ctx);

    // Should succeed
    assert!(result.is_ok(), "Should complete before deadline");
    let groups = result.unwrap();
    assert_eq!(groups.len(), 1);

    // Should have counted 50 docs (x >= 50 means 50..99 = 50 docs)
    let count = groups[0].get("count").and_then(|v| v.as_i64()).unwrap();
    assert_eq!(count, 50);
}

/// Test that group limit still works
#[test]
fn test_aggregate_with_context_respects_group_limit() {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();

    // Insert 100 documents with 100 unique groups
    for i in 0..100 {
        db.insert_one("test", json_to_hashmap(json!({"unique_id": i})))
            .unwrap();
    }

    let coll = db.collection("test").unwrap();

    // Only allow 50 groups
    let limits = AggregationLimits {
        max_docs_without_match: 1_000_000, // High doc limit
        max_group_count: 50,               // Low group limit
        ..Default::default()
    };

    let ctx = AggregationLimitContext::new(limits);

    let pipeline = json!([
        {"$group": {"_id": "$unique_id", "count": {"$sum": 1}}}
    ]);

    let result = coll.aggregate_with_context(&pipeline, &ctx);

    assert!(result.is_err(), "Should fail due to group limit");
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("group") || err.contains("limit"),
        "Error should mention group limit: {}",
        err
    );
}
