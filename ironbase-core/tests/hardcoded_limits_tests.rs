// hardcoded_limits_tests.rs
// Tests for hardcoded limits in IronBase to ensure they work correctly
//
// These tests verify that the various hardcoded limits in the codebase
// are enforced properly and provide meaningful error messages.

use ironbase_core::aggregation::AggregationLimits;
use ironbase_core::storage::MemoryStorage;
use ironbase_core::DatabaseCore;
use ironbase_core::FindOptions;
use serde_json::{json, Value};
use std::collections::HashMap;

// ============================================================================
// Helper Functions
// ============================================================================

fn create_test_db() -> DatabaseCore<MemoryStorage> {
    DatabaseCore::<MemoryStorage>::open_memory().unwrap()
}

fn json_to_hashmap(v: Value) -> HashMap<String, Value> {
    match v {
        Value::Object(map) => map.into_iter().collect(),
        _ => HashMap::new(),
    }
}

fn insert_doc(db: &DatabaseCore<MemoryStorage>, coll: &str, doc: Value) {
    db.insert_one(coll, json_to_hashmap(doc)).unwrap();
}

// ============================================================================
// Storage Limits
// ============================================================================

/// Test that documents with large content can be inserted (within 64 MB limit)
#[test]
fn test_large_document_within_limit() {
    let db = create_test_db();

    // Create a document with a 1 MB string field (well under 64 MB limit)
    let large_string = "x".repeat(1024 * 1024); // 1 MB string
    let doc = json!({
        "data": large_string
    });

    // This should succeed
    db.insert_one("test", json_to_hashmap(doc)).unwrap();

    // Verify document was inserted
    let coll = db.collection("test").unwrap();
    let count = coll.count_documents(&json!({})).unwrap();
    assert_eq!(count, 1);
}

/// Test that very deeply nested documents work up to reasonable depth
#[test]
fn test_nested_document_depth() {
    let db = create_test_db();

    // Create a 50-level deep nested document (should work, MAX_DEPTH=100)
    let mut doc = json!({"value": "deep"});
    for i in 0..50 {
        doc = json!({ format!("level_{}", i): doc });
    }

    db.insert_one("test", json_to_hashmap(doc)).unwrap();

    // Verify insertion worked
    let coll = db.collection("test").unwrap();
    let count = coll.count_documents(&json!({})).unwrap();
    assert_eq!(count, 1);
}

// ============================================================================
// B+ Tree Index Limits (MAX_KEYS_PER_NODE = 128)
// ============================================================================

/// Test that B+ tree handles many keys correctly (triggers node splits)
#[test]
fn test_btree_node_split_with_many_keys() {
    let db = create_test_db();
    let coll = db.collection("test").unwrap();

    // Create index on 'key' field
    coll.create_index("key".to_string(), false, false).unwrap();

    // Insert 200 documents (should trigger multiple node splits at MAX_KEYS_PER_NODE=128)
    for i in 0..200 {
        insert_doc(
            &db,
            "test",
            json!({
                "key": format!("key_{:05}", i),
                "value": i
            }),
        );
    }

    // Verify all documents are findable via index
    let results = coll.find(&json!({"key": {"$gte": "key_00000"}})).unwrap();
    assert_eq!(
        results.len(),
        200,
        "All 200 documents should be found via index"
    );

    // Verify specific lookups work after splits
    let result = coll.find_one(&json!({"key": "key_00100"})).unwrap();
    assert!(result.is_some(), "Document key_00100 should be found");
}

/// Test B+ tree with 1000+ entries (well above MAX_KEYS_PER_NODE)
#[test]
fn test_btree_many_entries() {
    let db = create_test_db();
    let coll = db.collection("test").unwrap();

    // Create index on 'num' field (not 'id' which conflicts with auto-created _id index)
    coll.create_index("num".to_string(), false, false).unwrap();

    // Insert 1000 documents
    for i in 0..1000 {
        insert_doc(
            &db,
            "test",
            json!({
                "num": i,
                "name": format!("item_{}", i)
            }),
        );
    }

    // Verify range query works
    let results = coll
        .find(&json!({"num": {"$gte": 500, "$lt": 510}}))
        .unwrap();
    assert_eq!(results.len(), 10, "Range query should return 10 documents");

    // Verify first and last entries
    assert!(coll.find_one(&json!({"num": 0})).unwrap().is_some());
    assert!(coll.find_one(&json!({"num": 999})).unwrap().is_some());
}

// ============================================================================
// Query Cache Limits (QUERY_CACHE_CAPACITY = 1000)
// ============================================================================

/// Test that query cache handles many different queries
#[test]
fn test_query_cache_many_queries() {
    let db = create_test_db();
    let coll = db.collection("test").unwrap();

    // Insert some data
    for i in 0..100 {
        insert_doc(&db, "test", json!({"x": i}));
    }

    // Execute 1500 different queries (exceeds QUERY_CACHE_CAPACITY=1000)
    // This tests LRU eviction behavior
    for i in 0..1500 {
        let query = json!({"x": {"$gte": i % 100}});
        let _ = coll.find(&query).unwrap();
    }

    // Re-run some queries to verify cache still works
    for i in 0..10 {
        let query = json!({"x": i});
        let results = coll.find(&query).unwrap();
        assert_eq!(
            results.len(),
            1,
            "Query should still work after cache eviction"
        );
    }
}

// ============================================================================
// Recursion/DoS Limits (MAX_DEPTH = 100 for $** operator)
// ============================================================================

/// Test $** wildcard operator with deeply nested documents
#[test]
fn test_wildcard_operator_depth_limit() {
    let db = create_test_db();
    let coll = db.collection("test").unwrap();

    // Create a document with 50 levels of nesting (within limit)
    let mut nested = json!({"target": "found"});
    for _ in 0..50 {
        nested = json!({"child": nested});
    }
    let doc = json!({"root": nested});
    db.insert_one("test", json_to_hashmap(doc)).unwrap();

    // $** should find the deeply nested field
    let results = coll.find(&json!({"$**.target": "found"})).unwrap();
    assert_eq!(results.len(), 1, "$** should find deeply nested field");
}

/// Test $** operator with document at depth limit boundary
#[test]
fn test_wildcard_operator_at_max_depth() {
    let db = create_test_db();
    let coll = db.collection("test").unwrap();

    // Create a document with 95 levels of nesting (just under MAX_DEPTH=100)
    let mut nested = json!({"deepest": "value"});
    for i in 0..95 {
        nested = json!({ format!("level_{}", i): nested });
    }
    db.insert_one("test", json_to_hashmap(nested)).unwrap();

    // $** should still work at this depth
    let results = coll.find(&json!({"$**.deepest": "value"})).unwrap();
    assert_eq!(results.len(), 1, "$** should work at depth 95");
}

// ============================================================================
// Cursor Batch Size (DEFAULT_CURSOR_BATCH_SIZE = 100)
// ============================================================================

/// Test cursor iteration with default batch size
#[test]
fn test_cursor_batch_size() {
    let db = create_test_db();
    let coll = db.collection("test").unwrap();

    // Insert 250 documents (spans multiple batches)
    for i in 0..250 {
        insert_doc(&db, "test", json!({"id": i}));
    }

    // Use streaming to iterate
    let mut cursor = coll.find_streaming(&json!({})).unwrap();
    let mut count = 0;
    while let Ok(Some(_doc)) = cursor.next() {
        count += 1;
    }
    assert_eq!(count, 250, "Cursor should return all 250 documents");
}

// ============================================================================
// Response Size Limits
// ============================================================================

/// Test that find with limit works correctly
#[test]
fn test_find_with_limit() {
    let db = create_test_db();
    let coll = db.collection("test").unwrap();

    // Insert documents with moderate size
    for i in 0..100 {
        insert_doc(
            &db,
            "test",
            json!({
                "id": i,
                "data": "x".repeat(1000) // 1KB each
            }),
        );
    }

    // Find with limit should work
    let options = FindOptions::new().with_limit(50);
    let results = coll.find_with_options(&json!({}), options).unwrap();
    assert_eq!(results.len(), 50, "Should return 50 documents");
}

/// Test estimate_json_size function accuracy
#[test]
fn test_estimate_json_size() {
    use ironbase_core::find_options::estimate_json_size;

    // Test simple values
    assert_eq!(estimate_json_size(&json!(null)), 4); // "null"
    assert_eq!(estimate_json_size(&json!(true)), 4); // "true"
    assert_eq!(estimate_json_size(&json!(false)), 5); // "false"
    assert_eq!(estimate_json_size(&json!(123)), 3); // "123"
    assert_eq!(estimate_json_size(&json!("hello")), 7); // "hello" + quotes

    // Test arrays
    let arr = json!([1, 2, 3]);
    let size = estimate_json_size(&arr);
    assert!(size > 0 && size < 20, "Array size should be reasonable");

    // Test objects
    let obj = json!({"a": 1, "b": 2});
    let size = estimate_json_size(&obj);
    assert!(size > 0 && size < 30, "Object size should be reasonable");
}

// ============================================================================
// Aggregation Limits
// ============================================================================

/// Test aggregation with document limit (max_docs_without_match)
/// The doc limit is enforced during streaming - we verify via $match comparison
#[test]
fn test_aggregation_doc_limit_with_match_relaxes_limit() {
    let db = create_test_db();
    let coll = db.collection("test").unwrap();

    // Insert 100 documents
    for i in 0..100 {
        insert_doc(&db, "test", json!({"x": i, "category": "A"}));
    }

    // Use limits where max_docs_without_match < doc count < max_docs_with_match
    let limits = AggregationLimits {
        max_docs_without_match: 50, // Would fail with 100 docs
        max_docs_with_match: 1000,  // But with $match, allows 1000
        max_group_count: 100,
        max_push_elements: 100,
        max_addtoset_elements: 100,
        max_total_push_elements: 100_000,
        max_total_addtoset_elements: 50_000,
        max_unwind_output: 1000,
        max_memory_mb: 64,
    };

    // Pipeline WITH $match should succeed because it uses the higher limit
    let pipeline = json!([
        {"$match": {"category": "A"}},
        {"$group": {"_id": null, "count": {"$sum": 1}}}
    ]);

    let result = coll.aggregate_with_limits(&pipeline, limits);
    assert!(
        result.is_ok(),
        "Should succeed with $match even if doc count > max_docs_without_match"
    );

    let docs = result.unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0]["count"], 100);
}

/// Test aggregation group count limit
#[test]
fn test_aggregation_group_limit() {
    let db = create_test_db();
    let coll = db.collection("test").unwrap();

    // Insert documents with many unique keys
    for i in 0..50 {
        insert_doc(&db, "test", json!({"key": format!("key_{}", i)}));
    }

    // Use restrictive group limit
    let limits = AggregationLimits {
        max_docs_without_match: 10000,
        max_docs_with_match: 10000,
        max_group_count: 10, // Only allow 10 groups
        max_push_elements: 100,
        max_addtoset_elements: 100,
        max_total_push_elements: 100_000,
        max_total_addtoset_elements: 50_000,
        max_unwind_output: 1000,
        max_memory_mb: 64,
    };

    // Group by key (50 unique keys > 10 group limit)
    let pipeline = json!([
        {"$group": {"_id": "$key", "count": {"$sum": 1}}}
    ]);

    let result = coll.aggregate_with_limits(&pipeline, limits);
    assert!(result.is_err(), "Should fail when exceeding group limit");
}

/// Test $unwind output limit
#[test]
fn test_aggregation_unwind_limit() {
    let db = create_test_db();
    let coll = db.collection("test").unwrap();

    // Insert document with array
    let doc = json!({
        "items": (0..100).collect::<Vec<_>>()
    });
    db.insert_one("test", json_to_hashmap(doc)).unwrap();

    // Use restrictive unwind limit
    let limits = AggregationLimits {
        max_docs_without_match: 10000,
        max_docs_with_match: 10000,
        max_group_count: 1000,
        max_push_elements: 100,
        max_addtoset_elements: 100,
        max_total_push_elements: 100_000,
        max_total_addtoset_elements: 50_000,
        max_unwind_output: 50, // Only allow 50 output docs
        max_memory_mb: 64,
    };

    // Unwind array with 100 elements > 50 limit
    let pipeline = json!([
        {"$unwind": "$items"}
    ]);

    let result = coll.aggregate_with_limits(&pipeline, limits);
    assert!(
        result.is_err(),
        "Should fail when exceeding unwind output limit"
    );
}

/// Test $push accumulator limit
#[test]
fn test_aggregation_push_limit() {
    let db = create_test_db();
    let coll = db.collection("test").unwrap();

    // Insert many documents with same group key
    for i in 0..50 {
        insert_doc(&db, "test", json!({"group": "A", "value": i}));
    }

    // Use restrictive push limit
    let limits = AggregationLimits {
        max_docs_without_match: 10000,
        max_docs_with_match: 10000,
        max_group_count: 1000,
        max_push_elements: 10, // Only allow 10 elements per push
        max_addtoset_elements: 100,
        max_total_push_elements: 100_000,
        max_total_addtoset_elements: 50_000,
        max_unwind_output: 1000,
        max_memory_mb: 64,
    };

    // Push 50 values > 10 limit
    let pipeline = json!([
        {"$group": {"_id": "$group", "values": {"$push": "$value"}}}
    ]);

    let result = coll.aggregate_with_limits(&pipeline, limits);
    assert!(
        result.is_err(),
        "Should fail when exceeding push element limit"
    );
}

// ============================================================================
// Compound Index Tests
// ============================================================================

/// Test compound index with many fields
#[test]
fn test_compound_index_multiple_fields() {
    let db = create_test_db();
    let coll = db.collection("test").unwrap();

    // Create compound index on 3 fields
    coll.create_compound_index(
        vec!["a".to_string(), "b".to_string(), "c".to_string()],
        false,
        false, // sparse
    )
    .unwrap();

    // Insert documents with predictable combinations
    // i=0: a=0, b=0, c=0
    // i=5: a=5, b=0, c=5
    // i=10: a=0, b=0, c=10
    // i=15: a=5, b=0, c=15
    for i in 0..100 {
        insert_doc(
            &db,
            "test",
            json!({
                "a": i % 10,
                "b": i % 5,
                "c": i
            }),
        );
    }

    // Query using compound index prefix - find a=0 (10 docs) and b=0 (i=0,10,20,30,40,50,60,70,80,90)
    // Documents where i%10=0 AND i%5=0: i=0,10,20,30,40,50,60,70,80,90 → all 10 have a=0, b=0
    let results = coll.find(&json!({"a": 0, "b": 0})).unwrap();
    assert_eq!(
        results.len(),
        10,
        "Compound index query should find 10 docs with a=0, b=0"
    );
}

// ============================================================================
// Fuzzy Search Threshold (DEFAULT_FUZZY_THRESHOLD = 0.8)
// ============================================================================

/// Test default fuzzy threshold
#[test]
fn test_fuzzy_default_threshold() {
    let db = create_test_db();
    let coll = db.collection("test").unwrap();

    insert_doc(&db, "test", json!({"name": "Johnson"}));
    insert_doc(&db, "test", json!({"name": "Johnsen"}));
    insert_doc(&db, "test", json!({"name": "Smith"}));

    // Default threshold is 0.8
    let results = coll.find(&json!({"name": {"$fuzzy": "Johnson"}})).unwrap();

    // Should match "Johnson" exactly and possibly "Johnsen" (very similar)
    assert!(
        !results.is_empty(),
        "Fuzzy search should find at least exact match"
    );
    assert!(results.len() <= 2, "Fuzzy search should not match 'Smith'");
}

/// Test fuzzy with custom threshold
#[test]
fn test_fuzzy_custom_threshold() {
    let db = create_test_db();
    let coll = db.collection("test").unwrap();

    insert_doc(&db, "test", json!({"name": "test"}));
    insert_doc(&db, "test", json!({"name": "tset"})); // typo
    insert_doc(&db, "test", json!({"name": "different"}));

    // Lower threshold should match more
    let results = coll
        .find(&json!({"name": {"$fuzzy": {"value": "test", "threshold": 0.5}}}))
        .unwrap();

    assert!(
        !results.is_empty(),
        "Fuzzy search with low threshold should find matches"
    );
}

// ============================================================================
// Limit/Skip Boundary Tests
// ============================================================================

/// Test find with limit at boundary values
#[test]
fn test_find_limit_boundaries() {
    let db = create_test_db();
    let coll = db.collection("test").unwrap();

    for i in 0..10 {
        insert_doc(&db, "test", json!({"x": i}));
    }

    // Limit 0 means no limit in MongoDB (returns all)
    let options = FindOptions::new().with_limit(0);
    let results = coll.find_with_options(&json!({}), options).unwrap();
    assert_eq!(
        results.len(),
        10,
        "Limit 0 should return all (MongoDB compat)"
    );

    // Limit 1 should return 1
    let options = FindOptions::new().with_limit(1);
    let results = coll.find_with_options(&json!({}), options).unwrap();
    assert_eq!(results.len(), 1, "Limit 1 should return 1");

    // Limit > count should return all
    let options = FindOptions::new().with_limit(100);
    let results = coll.find_with_options(&json!({}), options).unwrap();
    assert_eq!(results.len(), 10, "Limit 100 should return all 10");
}

/// Test skip at boundary values
#[test]
fn test_find_skip_boundaries() {
    let db = create_test_db();
    let coll = db.collection("test").unwrap();

    for i in 0..10 {
        insert_doc(&db, "test", json!({"x": i}));
    }

    // Skip 0 should return all
    let options = FindOptions::new().with_skip(0);
    let results = coll.find_with_options(&json!({}), options).unwrap();
    assert_eq!(results.len(), 10, "Skip 0 should return all");

    // Skip > count should return nothing
    let options = FindOptions::new().with_skip(100);
    let results = coll.find_with_options(&json!({}), options).unwrap();
    assert_eq!(results.len(), 0, "Skip 100 should return empty");

    // Skip = count should return nothing
    let options = FindOptions::new().with_skip(10);
    let results = coll.find_with_options(&json!({}), options).unwrap();
    assert_eq!(results.len(), 0, "Skip 10 should return empty");
}

// ============================================================================
// Index Build Batch Size (INDEX_BUILD_BATCH_SIZE = 1000)
// ============================================================================

/// Test index creation on large collection (tests batch building)
#[test]
fn test_index_build_batch_processing() {
    let db = create_test_db();
    let coll = db.collection("test").unwrap();

    // Insert 2500 documents (spans multiple batches at 1000 batch size)
    for i in 0..2500 {
        insert_doc(
            &db,
            "test",
            json!({"key": i, "value": format!("val_{}", i)}),
        );
    }

    // Create index after data insertion (tests batch building)
    let result = coll.create_index("key".to_string(), false, false);
    assert!(result.is_ok(), "Index creation should succeed");

    // Verify index works
    let results = coll.find(&json!({"key": 1234})).unwrap();
    assert_eq!(results.len(), 1, "Index should find document");
}

// ============================================================================
// Memory Profile Tests
// ============================================================================

/// Test AggregationLimits default values
#[test]
fn test_aggregation_limits_defaults() {
    let default = AggregationLimits::default();
    assert_eq!(default.max_docs_without_match, 10_000);
    assert_eq!(default.max_docs_with_match, 1_000_000);
    assert_eq!(default.max_group_count, 50_000);
    assert_eq!(default.max_push_elements, 100_000);
    assert_eq!(default.max_addtoset_elements, 100_000);
    assert_eq!(default.max_unwind_output, 1_000_000);
    assert_eq!(default.max_memory_mb, 512);
}

/// Test AggregationLimits low_memory profile
#[test]
fn test_aggregation_limits_low_memory() {
    let low = AggregationLimits::low_memory();
    assert_eq!(low.max_docs_without_match, 1_000);
    assert_eq!(low.max_docs_with_match, 5_000);
    assert_eq!(low.max_group_count, 500);
    assert_eq!(low.max_push_elements, 1_000);
    assert_eq!(low.max_addtoset_elements, 1_000);
    assert_eq!(low.max_unwind_output, 10_000);
    assert_eq!(low.max_memory_mb, 128);

    // Verify low < default
    let default = AggregationLimits::default();
    assert!(low.max_docs_without_match < default.max_docs_without_match);
    assert!(low.max_memory_mb < default.max_memory_mb);
}

/// Test with_memory_budget creates sensible limits
#[test]
fn test_aggregation_limits_memory_budget() {
    // Small budget
    let small = AggregationLimits::with_memory_budget(64);
    assert!(small.max_docs_without_match > 0);
    assert!(small.max_memory_mb <= 64);

    // Large budget
    let large = AggregationLimits::with_memory_budget(1024);
    assert!(large.max_docs_without_match > small.max_docs_without_match);

    // Very small budget should be clamped to minimum
    let tiny = AggregationLimits::with_memory_budget(1);
    assert!(tiny.max_memory_mb >= 64, "Should clamp to minimum 64 MB");
}

/// Test from_system_memory creates reasonable limits
#[test]
fn test_aggregation_limits_from_system() {
    let limits = AggregationLimits::from_system_memory();

    // Should have reasonable non-zero values
    assert!(limits.max_docs_without_match > 0);
    assert!(limits.max_group_count > 0);
    assert!(limits.max_memory_mb > 0);

    // Should not exceed maximums
    assert!(limits.max_memory_mb <= 4096);
}

// ============================================================================
// Cumulative Unwind Limit (2026-01 fix)
// ============================================================================

/// Test that multiple $unwind stages share cumulative limit
#[test]
fn test_cumulative_unwind_limit() {
    let db = create_test_db();
    let coll = db.collection("test").unwrap();

    // Insert documents with nested arrays
    for i in 0..5 {
        insert_doc(
            &db,
            "test",
            json!({
                "id": i,
                "orders": [
                    {"items": [1, 2, 3]},
                    {"items": [4, 5, 6]}
                ]
            }),
        );
    }

    // Restrictive cumulative limit
    let limits = AggregationLimits {
        max_docs_without_match: 10000,
        max_docs_with_match: 10000,
        max_group_count: 1000,
        max_push_elements: 1000,
        max_addtoset_elements: 1000,
        max_total_push_elements: 100_000,
        max_total_addtoset_elements: 50_000,
        max_unwind_output: 20, // Very low cumulative limit
        max_memory_mb: 64,
    };

    // Pipeline with chained unwinding:
    // 5 docs × 2 orders = 10 docs (first unwind)
    // 10 docs × 3 items = 30 docs (second unwind) > 20 limit
    let pipeline = json!([
        {"$unwind": "$orders"},
        {"$unwind": "$orders.items"}
    ]);

    let result = coll.aggregate_with_limits(&pipeline, limits);
    assert!(
        result.is_err(),
        "Should fail when cumulative unwind exceeds limit"
    );
}
