// tests/aggregation_context_tests.rs
// Integration tests for context-aware aggregation (2026-01 refactoring)
//
// Tests the new AggregationLimitContext system for unified limit tracking.

use ironbase_core::aggregation::{AggregationLimitContext, AggregationLimits};
use ironbase_core::storage::MemoryStorage;
use ironbase_core::DatabaseCore;
use serde_json::{json, Value};
use std::collections::HashMap;

// ========== Helper Functions ==========

fn create_test_db() -> DatabaseCore<MemoryStorage> {
    DatabaseCore::<MemoryStorage>::open_memory().unwrap()
}

fn json_to_hashmap(v: Value) -> HashMap<String, Value> {
    match v {
        Value::Object(map) => map.into_iter().collect(),
        _ => HashMap::new(),
    }
}

fn insert_test_docs(db: &DatabaseCore<MemoryStorage>, collection: &str, count: usize) {
    for i in 0..count {
        db.insert_one(
            collection,
            json_to_hashmap(json!({
                "i": i,
                "group": format!("group_{}", i % 10),
                "value": i * 10,
                "tags": ["a", "b", "c"]
            })),
        )
        .unwrap();
    }
}

// ========== Context Creation Tests ==========

#[test]
fn test_context_new_with_default_limits() {
    let ctx = AggregationLimitContext::new(AggregationLimits::default());
    assert_eq!(ctx.docs_processed(), 0);
    assert_eq!(ctx.groups_created(), 0);
    assert!(!ctx.has_leading_match());
}

#[test]
fn test_context_with_custom_limits() {
    let limits = AggregationLimits {
        max_docs_without_match: 100,
        max_docs_with_match: 1000,
        max_group_count: 50,
        max_push_elements: 20,
        max_addtoset_elements: 20,
        max_unwind_output: 500,
        max_memory_mb: 64,
    };
    let ctx = AggregationLimitContext::new(limits);
    assert_eq!(ctx.limits().max_docs_without_match, 100);
    assert_eq!(ctx.limits().max_group_count, 50);
}

#[test]
fn test_context_clone_shares_state() {
    let ctx1 = AggregationLimitContext::new(AggregationLimits::default());
    let ctx2 = ctx1.clone();

    ctx1.increment_docs().unwrap();
    ctx1.increment_docs().unwrap();

    // Both contexts should see the same count
    assert_eq!(ctx1.docs_processed(), 2);
    assert_eq!(ctx2.docs_processed(), 2);
}

#[test]
fn test_context_with_leading_match() {
    let limits = AggregationLimits {
        max_docs_without_match: 10,
        max_docs_with_match: 100,
        ..Default::default()
    };
    let ctx = AggregationLimitContext::new(limits).with_leading_match(true);

    assert!(ctx.has_leading_match());

    // Should allow more docs with leading match
    for _ in 0..50 {
        ctx.increment_docs().unwrap();
    }
    assert_eq!(ctx.docs_processed(), 50);
}

// ========== Document Limit Tests ==========

#[test]
fn test_context_doc_limit_without_match() {
    let limits = AggregationLimits {
        max_docs_without_match: 5,
        max_docs_with_match: 100,
        ..Default::default()
    };
    let ctx = AggregationLimitContext::new(limits);

    // First 5 should succeed
    for _ in 0..5 {
        ctx.increment_docs().unwrap();
    }

    // 6th should fail
    let result = ctx.increment_docs();
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("document limit"));
}

#[test]
fn test_context_doc_limit_with_match() {
    let limits = AggregationLimits {
        max_docs_without_match: 5,
        max_docs_with_match: 20,
        ..Default::default()
    };
    let ctx = AggregationLimitContext::new(limits).with_leading_match(true);

    // Should allow up to 20 with match
    for _ in 0..20 {
        ctx.increment_docs().unwrap();
    }

    // 21st should fail
    let result = ctx.increment_docs();
    assert!(result.is_err());
}

// ========== Group Limit Tests ==========

#[test]
fn test_context_group_limit() {
    let limits = AggregationLimits {
        max_group_count: 3,
        ..Default::default()
    };
    let ctx = AggregationLimitContext::new(limits);

    ctx.register_new_group(1).unwrap();
    ctx.register_new_group(2).unwrap();
    ctx.register_new_group(3).unwrap();

    // 4th group should fail
    let result = ctx.register_new_group(4);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("group limit"));
}

#[test]
fn test_context_group_reregistration_allowed() {
    let limits = AggregationLimits {
        max_group_count: 2,
        ..Default::default()
    };
    let ctx = AggregationLimitContext::new(limits);

    ctx.register_new_group(1).unwrap();
    ctx.register_new_group(2).unwrap();

    // Re-registering existing groups should NOT fail
    ctx.register_new_group(1).unwrap();
    ctx.register_new_group(2).unwrap();

    assert_eq!(ctx.groups_created(), 2);
}

// ========== Per-Group $push/$addToSet Limit Tests ==========

#[test]
fn test_context_push_limit_per_group() {
    let limits = AggregationLimits {
        max_push_elements: 3,
        max_group_count: 10,
        ..Default::default()
    };
    let ctx = AggregationLimitContext::new(limits);

    // Register two groups
    ctx.register_new_group(1).unwrap();
    ctx.register_new_group(2).unwrap();

    // Group 1: 3 pushes OK
    ctx.increment_push(1).unwrap();
    ctx.increment_push(1).unwrap();
    ctx.increment_push(1).unwrap();

    // Group 1: 4th push should fail
    let result = ctx.increment_push(1);
    assert!(result.is_err());

    // Group 2: Still has its own quota
    ctx.increment_push(2).unwrap();
    ctx.increment_push(2).unwrap();
    ctx.increment_push(2).unwrap();
    assert!(ctx.increment_push(2).is_err());
}

#[test]
fn test_context_addtoset_limit_per_group() {
    let limits = AggregationLimits {
        max_addtoset_elements: 2,
        max_group_count: 10,
        ..Default::default()
    };
    let ctx = AggregationLimitContext::new(limits);

    ctx.register_new_group(1).unwrap();

    ctx.increment_addtoset(1).unwrap();
    ctx.increment_addtoset(1).unwrap();

    // 3rd should fail
    let result = ctx.increment_addtoset(1);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("$addToSet"));
}

// ========== $unwind Limit Tests ==========

#[test]
fn test_context_unwind_limit() {
    let limits = AggregationLimits {
        max_unwind_output: 10,
        ..Default::default()
    };
    let ctx = AggregationLimitContext::new(limits);

    ctx.increment_unwind(5).unwrap();
    ctx.increment_unwind(5).unwrap();

    // 11th should fail
    let result = ctx.increment_unwind(1);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("$unwind"));
}

// ========== Context Reset Tests ==========

#[test]
fn test_context_reset() {
    let ctx = AggregationLimitContext::new(AggregationLimits::default());

    ctx.increment_docs().unwrap();
    ctx.register_new_group(1).unwrap();
    ctx.increment_unwind(5).unwrap();

    assert_eq!(ctx.docs_processed(), 1);
    assert_eq!(ctx.groups_created(), 1);
    assert_eq!(ctx.unwind_outputs(), 5);

    ctx.reset();

    assert_eq!(ctx.docs_processed(), 0);
    assert_eq!(ctx.groups_created(), 0);
    assert_eq!(ctx.unwind_outputs(), 0);
}

// ========== Pipeline Integration Tests ==========

#[test]
fn test_pipeline_execute_with_context_simple() {
    use ironbase_core::aggregation::Pipeline;

    let docs = vec![
        Ok(json!({"x": 1})),
        Ok(json!({"x": 2})),
        Ok(json!({"x": 3})),
    ];

    let pipeline = Pipeline::from_json(&json!([
        {"$match": {"x": {"$gte": 2}}}
    ]))
    .unwrap();

    let ctx = AggregationLimitContext::new(AggregationLimits::default());
    let results = pipeline
        .execute_with_context(docs.into_iter(), &ctx)
        .unwrap();

    assert_eq!(results.len(), 2);
    assert!(ctx.docs_processed() > 0);
}

#[test]
fn test_pipeline_execute_with_context_group() {
    use ironbase_core::aggregation::Pipeline;

    let docs: Vec<Result<serde_json::Value, ironbase_core::error::IronBaseError>> = (0..20)
        .map(|i| Ok(json!({"group": format!("g{}", i % 5), "val": i})))
        .collect();

    let pipeline = Pipeline::from_json(&json!([
        {"$group": {"_id": "$group", "count": {"$sum": 1}}}
    ]))
    .unwrap();

    let limits = AggregationLimits {
        max_group_count: 10,
        ..Default::default()
    };
    let ctx = AggregationLimitContext::new(limits);
    let results = pipeline
        .execute_with_context(docs.into_iter(), &ctx)
        .unwrap();

    assert_eq!(results.len(), 5); // 5 unique groups
    assert_eq!(ctx.groups_created(), 5);
}

#[test]
fn test_pipeline_execute_with_context_group_limit_exceeded() {
    use ironbase_core::aggregation::Pipeline;

    // Create docs with 10 unique groups
    let docs: Vec<Result<serde_json::Value, ironbase_core::error::IronBaseError>> = (0..100)
        .map(|i| Ok(json!({"group": format!("g{}", i % 10), "val": i})))
        .collect();

    let pipeline = Pipeline::from_json(&json!([
        {"$group": {"_id": "$group", "count": {"$sum": 1}}}
    ]))
    .unwrap();

    let limits = AggregationLimits {
        max_group_count: 5, // Only allow 5 groups
        ..Default::default()
    };
    let ctx = AggregationLimitContext::new(limits);
    let result = pipeline.execute_with_context(docs.into_iter(), &ctx);

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("group limit"));
}

#[test]
fn test_pipeline_execute_with_context_match_project() {
    use ironbase_core::aggregation::Pipeline;

    let docs: Vec<Result<serde_json::Value, ironbase_core::error::IronBaseError>> = (0..10)
        .map(|i| Ok(json!({"x": i, "y": i * 2, "z": "extra"})))
        .collect();

    let pipeline = Pipeline::from_json(&json!([
        {"$match": {"x": {"$gte": 5}}},
        {"$project": {"x": 1, "y": 1}}
    ]))
    .unwrap();

    let ctx = AggregationLimitContext::new(AggregationLimits::default());
    let results = pipeline
        .execute_with_context(docs.into_iter(), &ctx)
        .unwrap();

    assert_eq!(results.len(), 5); // x >= 5 means 5, 6, 7, 8, 9
    for result in &results {
        assert!(result.get("x").is_some());
        assert!(result.get("y").is_some());
        assert!(result.get("z").is_none()); // Projected out
    }
}

// ========== Collection Integration Tests ==========

#[test]
fn test_collection_aggregate_with_context() {
    let db = create_test_db();
    insert_test_docs(&db, "test", 50);

    let coll = db.collection("test").unwrap();
    let limits = AggregationLimits {
        max_docs_with_match: 100,
        max_group_count: 20,
        ..Default::default()
    };
    let ctx = AggregationLimitContext::new(limits);

    let results = coll
        .aggregate_with_context(
            &json!([
                {"$match": {"i": {"$lt": 30}}},
                {"$group": {"_id": "$group", "total": {"$sum": "$value"}}}
            ]),
            &ctx,
        )
        .unwrap();

    assert_eq!(results.len(), 10); // 10 unique groups
    assert!(ctx.docs_processed() > 0 || ctx.groups_created() > 0);
}

#[test]
fn test_collection_aggregate_with_context_limit_exceeded_non_streaming() {
    // NOTE: $group with streaming does NOT hit doc limit (docs processed one at a time)
    // This test uses $sort which DOES require materialization (all docs in memory)
    let db = create_test_db();
    insert_test_docs(&db, "test", 100);

    let coll = db.collection("test").unwrap();
    let limits = AggregationLimits {
        max_docs_without_match: 50, // Only allow 50 docs without match
        max_docs_with_match: 50,
        ..Default::default()
    };
    let ctx = AggregationLimitContext::new(limits);

    // $sort requires materializing all documents, so doc limit applies
    let result = coll.aggregate_with_context(
        &json!([
            {"$sort": {"value": 1}}
        ]),
        &ctx,
    );

    assert!(
        result.is_err(),
        "Should fail because $sort needs to materialize 100 docs (limit: 50)"
    );
}

#[test]
fn test_collection_aggregate_streaming_group_bypasses_doc_limit() {
    // Streaming $group does NOT hit doc limit - docs are processed one at a time
    let db = create_test_db();
    insert_test_docs(&db, "test", 100);

    let coll = db.collection("test").unwrap();
    let limits = AggregationLimits {
        max_docs_without_match: 50, // Only 50 docs allowed if materialized
        max_docs_with_match: 50,
        max_group_count: 100, // But allow all groups
        ..Default::default()
    };
    let ctx = AggregationLimitContext::new(limits);

    // $group is streaming - should SUCCEED even with 100 docs and 50 doc limit
    let result = coll.aggregate_with_context(
        &json!([
            {"$group": {"_id": "$group", "count": {"$sum": 1}}}
        ]),
        &ctx,
    );

    assert!(
        result.is_ok(),
        "Streaming $group should bypass doc limit: {:?}",
        result
    );
    let groups = result.unwrap();
    assert_eq!(groups.len(), 10); // 100 docs with group = value % 10 = 10 groups
}

#[test]
fn test_collection_aggregate_with_context_limit_only() {
    let db = create_test_db();
    insert_test_docs(&db, "test", 20);

    let coll = db.collection("test").unwrap();
    let ctx = AggregationLimitContext::new(AggregationLimits::default());

    let results = coll
        .aggregate_with_context(&json!([{"$limit": 5}]), &ctx)
        .unwrap();

    assert_eq!(
        results.len(),
        5,
        "Limit stage without other operators should still return the first K docs"
    );
    assert!(results.iter().all(|doc| doc.get("i").is_some()));
}

#[test]
fn test_collection_aggregate_with_context_inspectable() {
    let db = create_test_db();
    insert_test_docs(&db, "test", 30);

    let coll = db.collection("test").unwrap();
    let ctx = AggregationLimitContext::new(AggregationLimits::default());

    let _results = coll
        .aggregate_with_context(
            &json!([
                {"$group": {"_id": "$group", "count": {"$sum": 1}}}
            ]),
            &ctx,
        )
        .unwrap();

    // Context should be inspectable after execution
    assert_eq!(ctx.groups_created(), 10); // 30 docs % 10 = 10 groups
}

// ========== Memory Profile Tests ==========

#[test]
fn test_memory_tier_tiny() {
    use ironbase_core::aggregation::{limits_from_tier, MemoryTier};

    let limits = limits_from_tier(MemoryTier::Tiny);
    assert_eq!(limits.max_docs_without_match, 10_000);
    assert_eq!(limits.max_group_count, 5_000);
    assert_eq!(limits.max_memory_mb, 64);
}

#[test]
fn test_memory_tier_xlarge() {
    use ironbase_core::aggregation::{limits_from_tier, MemoryTier};

    let limits = limits_from_tier(MemoryTier::XLarge);
    assert_eq!(limits.max_docs_without_match, 500_000);
    assert_eq!(limits.max_group_count, 250_000);
    assert_eq!(limits.max_memory_mb, 1024);
}

#[test]
fn test_memory_tier_scaling() {
    use ironbase_core::aggregation::{limits_from_tier, MemoryTier};

    let tiny = limits_from_tier(MemoryTier::Tiny);
    let small = limits_from_tier(MemoryTier::Small);
    let medium = limits_from_tier(MemoryTier::Medium);
    let large = limits_from_tier(MemoryTier::Large);
    let xlarge = limits_from_tier(MemoryTier::XLarge);

    // Each tier should have progressively higher limits
    assert!(tiny.max_docs_without_match < small.max_docs_without_match);
    assert!(small.max_docs_without_match < medium.max_docs_without_match);
    assert!(medium.max_docs_without_match < large.max_docs_without_match);
    assert!(large.max_docs_without_match < xlarge.max_docs_without_match);
}

#[test]
fn test_limits_from_system_returns_valid() {
    let limits = AggregationLimits::from_system_memory();

    // Should return valid limits regardless of system
    assert!(limits.max_docs_without_match > 0);
    assert!(limits.max_docs_with_match >= limits.max_docs_without_match);
    assert!(limits.max_group_count > 0);
    assert!(limits.max_memory_mb > 0);
}

// ========== Edge Case Tests ==========

#[test]
fn test_empty_pipeline_with_context() {
    use ironbase_core::aggregation::Pipeline;

    // Empty pipeline should be rejected at parse time (docs not needed)
    let pipeline = Pipeline::from_json(&json!([]));
    assert!(pipeline.is_err());
}

#[test]
fn test_context_multiple_stages() {
    use ironbase_core::aggregation::Pipeline;

    let docs: Vec<Result<serde_json::Value, ironbase_core::error::IronBaseError>> = (0..100)
        .map(|i| {
            Ok(json!({
                "category": format!("cat{}", i % 5),
                "value": i,
                "active": i % 2 == 0
            }))
        })
        .collect();

    let pipeline = Pipeline::from_json(&json!([
        {"$match": {"active": true}},
        {"$group": {"_id": "$category", "total": {"$sum": "$value"}}},
        {"$sort": {"total": -1}},
        {"$limit": 3}
    ]))
    .unwrap();

    let ctx = AggregationLimitContext::new(AggregationLimits::default());
    let results = pipeline
        .execute_with_context(docs.into_iter(), &ctx)
        .unwrap();

    assert_eq!(results.len(), 3); // Limited to 3
    assert_eq!(ctx.groups_created(), 5); // 5 categories
}
