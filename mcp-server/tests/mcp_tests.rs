//! MCP Server Integration Tests
//!
//! Tests the full MCP tool dispatch pipeline with a real database.
//! Response format uses snake_case: inserted_id, matched_count, deleted_count, etc.

use mcp_ironbase::{dispatch_tool, get_tools_list, IronBaseAdapter};
use serde_json::json;
use std::sync::Arc;

mod common;
use common::{create_test_adapter, dispatch_err, dispatch_ok};

// ============================================================================
// Tools List Tests
// ============================================================================

#[test]
fn test_tools_list_returns_valid_json() {
    let tools = get_tools_list();
    assert!(tools.is_object());
    assert!(tools.get("tools").is_some());

    let tools_array = tools["tools"].as_array().expect("tools should be array");
    assert!(!tools_array.is_empty(), "Should have at least one tool");

    // Verify each tool has required fields
    for tool in tools_array {
        assert!(tool.get("name").is_some(), "Tool should have name");
        assert!(
            tool.get("description").is_some(),
            "Tool should have description"
        );
        assert!(
            tool.get("inputSchema").is_some(),
            "Tool should have inputSchema"
        );
    }
}

#[test]
fn test_tools_list_contains_crud_tools() {
    let tools = get_tools_list();
    let tools_array = tools["tools"].as_array().unwrap();

    let tool_names: Vec<&str> = tools_array
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();

    // Verify essential CRUD tools exist
    assert!(tool_names.contains(&"insert_one"), "Missing insert_one");
    assert!(tool_names.contains(&"find"), "Missing find");
    assert!(tool_names.contains(&"update_one"), "Missing update_one");
    assert!(tool_names.contains(&"delete_one"), "Missing delete_one");
    assert!(tool_names.contains(&"count"), "Missing count_documents");
    assert!(tool_names.contains(&"aggregate"), "Missing aggregate");
}

// ============================================================================
// CRUD Operation Tests
// ============================================================================

#[test]
fn test_insert_one_and_find() {
    let (adapter, _temp) = create_test_adapter();

    // Insert a document
    let result = dispatch_ok(
        &adapter,
        "insert_one",
        json!({
            "collection": "users",
            "document": {"name": "Alice", "age": 30}
        }),
    );

    assert!(
        result.get("inserted_id").is_some(),
        "Should return inserted_id"
    );

    // Find the document
    let result = dispatch_ok(
        &adapter,
        "find",
        json!({
            "collection": "users",
            "filter": {"name": "Alice"}
        }),
    );

    let docs = result["documents"]
        .as_array()
        .expect("Should return documents array");
    assert_eq!(docs.len(), 1, "Should find exactly one document");
    assert_eq!(docs[0]["name"], "Alice");
    assert_eq!(docs[0]["age"], 30);
    // Verify _id field exists in found document
    assert!(
        docs[0].get("_id").is_some(),
        "Found document should have _id"
    );
}

#[test]
fn test_insert_many() {
    let (adapter, _temp) = create_test_adapter();

    let result = dispatch_ok(
        &adapter,
        "insert_many",
        json!({
            "collection": "products",
            "documents": [
                {"name": "Apple", "price": 1.50},
                {"name": "Banana", "price": 0.75},
                {"name": "Cherry", "price": 3.00}
            ]
        }),
    );

    assert_eq!(result["inserted_count"], 3);

    // Verify count
    let count = dispatch_ok(
        &adapter,
        "count",
        json!({
            "collection": "products",
            "filter": {}
        }),
    );
    assert_eq!(count["count"], 3);
}

#[test]
fn test_update_one() {
    let (adapter, _temp) = create_test_adapter();

    // Insert
    dispatch_ok(
        &adapter,
        "insert_one",
        json!({
            "collection": "users",
            "document": {"name": "Bob", "age": 25}
        }),
    );

    // Update
    let result = dispatch_ok(
        &adapter,
        "update_one",
        json!({
            "collection": "users",
            "filter": {"name": "Bob"},
            "update": {"$set": {"age": 26}}
        }),
    );

    assert_eq!(result["matched_count"], 1);
    assert_eq!(result["modified_count"], 1);

    // Verify update
    let found = dispatch_ok(
        &adapter,
        "find_one",
        json!({
            "collection": "users",
            "filter": {"name": "Bob"}
        }),
    );
    assert_eq!(found["document"]["age"], 26);
}

#[test]
fn test_update_one_upsert() {
    let (adapter, _temp) = create_test_adapter();

    // Upsert (insert because no match)
    let result = dispatch_ok(
        &adapter,
        "update_one",
        json!({
            "collection": "users",
            "filter": {"name": "NewUser"},
            "update": {"$set": {"age": 20}},
            "upsert": true
        }),
    );

    assert_eq!(result["matched_count"], 0);
    assert!(
        result.get("upserted_id").is_some(),
        "Should have upserted_id"
    );

    // Verify document exists
    let count = dispatch_ok(
        &adapter,
        "count",
        json!({
            "collection": "users",
            "filter": {"name": "NewUser"}
        }),
    );
    assert_eq!(count["count"], 1);
}

#[test]
fn test_delete_one() {
    let (adapter, _temp) = create_test_adapter();

    // Insert two documents
    dispatch_ok(
        &adapter,
        "insert_many",
        json!({
            "collection": "items",
            "documents": [
                {"type": "A"},
                {"type": "B"}
            ]
        }),
    );

    // Delete one
    let result = dispatch_ok(
        &adapter,
        "delete_one",
        json!({
            "collection": "items",
            "filter": {"type": "A"}
        }),
    );
    assert_eq!(result["deleted_count"], 1);

    // Verify only one remains
    let count = dispatch_ok(
        &adapter,
        "count",
        json!({
            "collection": "items",
            "filter": {}
        }),
    );
    assert_eq!(count["count"], 1);
}

#[test]
fn test_delete_many() {
    let (adapter, _temp) = create_test_adapter();

    // Insert documents
    dispatch_ok(
        &adapter,
        "insert_many",
        json!({
            "collection": "logs",
            "documents": [
                {"level": "info", "msg": "a"},
                {"level": "info", "msg": "b"},
                {"level": "error", "msg": "c"}
            ]
        }),
    );

    // Delete all info logs
    let result = dispatch_ok(
        &adapter,
        "delete_many",
        json!({
            "collection": "logs",
            "filter": {"level": "info"}
        }),
    );
    assert_eq!(result["deleted_count"], 2);

    // Verify only error log remains
    let count = dispatch_ok(
        &adapter,
        "count",
        json!({
            "collection": "logs",
            "filter": {}
        }),
    );
    assert_eq!(count["count"], 1);
}

// ============================================================================
// Query Tests
// ============================================================================

#[test]
fn test_find_with_projection() {
    let (adapter, _temp) = create_test_adapter();

    dispatch_ok(
        &adapter,
        "insert_one",
        json!({
            "collection": "users",
            "document": {"name": "Alice", "age": 30, "email": "alice@test.com"}
        }),
    );

    let result = dispatch_ok(
        &adapter,
        "find",
        json!({
            "collection": "users",
            "filter": {},
            "projection": {"name": 1, "age": 1}
        }),
    );

    let doc = &result["documents"][0];
    assert!(doc.get("name").is_some());
    assert!(doc.get("age").is_some());
    assert!(doc.get("email").is_none(), "email should be excluded");
}

#[test]
fn test_find_with_sort_and_limit() {
    let (adapter, _temp) = create_test_adapter();

    dispatch_ok(
        &adapter,
        "insert_many",
        json!({
            "collection": "scores",
            "documents": [
                {"name": "A", "score": 100},
                {"name": "B", "score": 50},
                {"name": "C", "score": 75}
            ]
        }),
    );

    let result = dispatch_ok(
        &adapter,
        "find",
        json!({
            "collection": "scores",
            "filter": {},
            "sort": {"score": -1},
            "limit": 2
        }),
    );

    let docs = result["documents"].as_array().unwrap();
    assert_eq!(docs.len(), 2);
    assert_eq!(docs[0]["name"], "A"); // highest score
    assert_eq!(docs[1]["name"], "C"); // second highest
}

#[test]
fn test_distinct() {
    let (adapter, _temp) = create_test_adapter();

    dispatch_ok(
        &adapter,
        "insert_many",
        json!({
            "collection": "products",
            "documents": [
                {"category": "fruit", "name": "Apple"},
                {"category": "fruit", "name": "Banana"},
                {"category": "vegetable", "name": "Carrot"}
            ]
        }),
    );

    let result = dispatch_ok(
        &adapter,
        "distinct",
        json!({
            "collection": "products",
            "field": "category"
        }),
    );

    let values = result["values"].as_array().unwrap();
    assert_eq!(values.len(), 2);
    assert!(values.contains(&json!("fruit")));
    assert!(values.contains(&json!("vegetable")));
}

// ============================================================================
// Aggregation Tests
// ============================================================================

#[test]
fn test_aggregate_group_sum() {
    let (adapter, _temp) = create_test_adapter();

    dispatch_ok(
        &adapter,
        "insert_many",
        json!({
            "collection": "sales",
            "documents": [
                {"product": "A", "amount": 100},
                {"product": "A", "amount": 150},
                {"product": "B", "amount": 200}
            ]
        }),
    );

    let result = dispatch_ok(
        &adapter,
        "aggregate",
        json!({
            "collection": "sales",
            "pipeline": [
                {"$group": {"_id": "$product", "total": {"$sum": "$amount"}}},
                {"$sort": {"_id": 1}}
            ]
        }),
    );

    let docs = result["results"].as_array().unwrap();
    assert_eq!(docs.len(), 2);
    assert_eq!(docs[0]["_id"], "A");
    assert_eq!(docs[0]["total"], 250);
    assert_eq!(docs[1]["_id"], "B");
    assert_eq!(docs[1]["total"], 200);
}

#[test]
fn test_aggregate_match_and_count() {
    let (adapter, _temp) = create_test_adapter();

    dispatch_ok(
        &adapter,
        "insert_many",
        json!({
            "collection": "orders",
            "documents": [
                {"status": "completed", "amount": 100},
                {"status": "completed", "amount": 200},
                {"status": "pending", "amount": 50}
            ]
        }),
    );

    let result = dispatch_ok(
        &adapter,
        "aggregate",
        json!({
            "collection": "orders",
            "pipeline": [
                {"$match": {"status": "completed"}},
                {"$count": "completedOrders"}
            ]
        }),
    );

    let docs = result["results"].as_array().unwrap();
    assert_eq!(docs[0]["completedOrders"], 2);
}

// ============================================================================
// Index Tests
// ============================================================================

#[test]
fn test_index_create_and_list() {
    let (adapter, _temp) = create_test_adapter();

    // Create collection first
    dispatch_ok(
        &adapter,
        "insert_one",
        json!({
            "collection": "users",
            "document": {"name": "Test", "email": "test@test.com"}
        }),
    );

    // Create index
    let result = dispatch_ok(
        &adapter,
        "index_create",
        json!({
            "collection": "users",
            "field": "email",
            "unique": true
        }),
    );
    assert!(result.get("index_name").is_some() || result.get("indexName").is_some());

    // List indexes - response has btree_indexes, fulltext_indexes, vector_indexes
    let result = dispatch_ok(
        &adapter,
        "index_list",
        json!({
            "collection": "users"
        }),
    );
    let btree_indexes = result["btree_indexes"]
        .as_array()
        .expect("Should have btree_indexes array");
    assert!(
        !btree_indexes.is_empty(),
        "Should have at least the email index"
    );
}

#[test]
fn test_explain_query() {
    let (adapter, _temp) = create_test_adapter();

    // Insert and create index
    dispatch_ok(
        &adapter,
        "insert_one",
        json!({
            "collection": "users",
            "document": {"name": "Alice", "age": 30}
        }),
    );
    dispatch_ok(
        &adapter,
        "index_create",
        json!({
            "collection": "users",
            "field": "age"
        }),
    );

    // Explain query
    let result = dispatch_ok(
        &adapter,
        "explain",
        json!({
            "collection": "users",
            "filter": {"age": {"$gt": 20}}
        }),
    );

    // Should return some kind of plan
    assert!(result.is_object());
}

// ============================================================================
// Error Handling Tests
// ============================================================================

#[test]
fn test_collection_not_found_error() {
    let (adapter, _temp) = create_test_adapter();

    let err = dispatch_err(
        &adapter,
        "find",
        json!({
            "collection": "nonexistent",
            "filter": {}
        }),
    );

    // Should return CollectionNotFound error code (-32001)
    assert_eq!(err.code.code(), -32001);
}

#[test]
fn test_invalid_params_missing_collection() {
    let (adapter, _temp) = create_test_adapter();

    let err = dispatch_err(
        &adapter,
        "find",
        json!({
            "filter": {}
        }),
    );

    // Should return InvalidParams error
    assert_eq!(err.code.code(), -32602);
}

#[test]
fn test_invalid_aggregation_pipeline() {
    let (adapter, _temp) = create_test_adapter();

    dispatch_ok(
        &adapter,
        "insert_one",
        json!({
            "collection": "test",
            "document": {"x": 1}
        }),
    );

    let err = dispatch_err(
        &adapter,
        "aggregate",
        json!({
            "collection": "test",
            "pipeline": [
                {"$invalidStage": {}}
            ]
        }),
    );

    // Should return AggregationError (-32011)
    assert_eq!(err.code.code(), -32011);
}

// ============================================================================
// Edge Case Tests
// ============================================================================

#[test]
fn test_empty_collection_operations() {
    let (adapter, _temp) = create_test_adapter();

    // Create empty collection via insert+delete
    dispatch_ok(
        &adapter,
        "insert_one",
        json!({
            "collection": "empty",
            "document": {"temp": true}
        }),
    );
    dispatch_ok(
        &adapter,
        "delete_many",
        json!({
            "collection": "empty",
            "filter": {}
        }),
    );

    // Find on empty collection
    let result = dispatch_ok(
        &adapter,
        "find",
        json!({
            "collection": "empty",
            "filter": {}
        }),
    );
    assert_eq!(result["documents"].as_array().unwrap().len(), 0);

    // Count on empty collection
    let result = dispatch_ok(
        &adapter,
        "count",
        json!({
            "collection": "empty",
            "filter": {}
        }),
    );
    assert_eq!(result["count"], 0);
}

#[test]
fn test_update_no_match() {
    let (adapter, _temp) = create_test_adapter();

    dispatch_ok(
        &adapter,
        "insert_one",
        json!({
            "collection": "users",
            "document": {"name": "Alice"}
        }),
    );

    let result = dispatch_ok(
        &adapter,
        "update_one",
        json!({
            "collection": "users",
            "filter": {"name": "NonExistent"},
            "update": {"$set": {"age": 99}}
        }),
    );

    assert_eq!(result["matched_count"], 0);
    assert_eq!(result["modified_count"], 0);
}

#[test]
fn test_delete_no_match() {
    let (adapter, _temp) = create_test_adapter();

    dispatch_ok(
        &adapter,
        "insert_one",
        json!({
            "collection": "items",
            "document": {"x": 1}
        }),
    );

    let result = dispatch_ok(
        &adapter,
        "delete_one",
        json!({
            "collection": "items",
            "filter": {"x": 999}
        }),
    );

    assert_eq!(result["deleted_count"], 0);
}

#[test]
fn test_nested_document_operations() {
    let (adapter, _temp) = create_test_adapter();

    // Insert nested document
    dispatch_ok(
        &adapter,
        "insert_one",
        json!({
            "collection": "profiles",
            "document": {
                "user": {
                    "name": "Alice",
                    "address": {
                        "city": "NYC",
                        "zip": "10001"
                    }
                }
            }
        }),
    );

    // Query with dot notation
    let result = dispatch_ok(
        &adapter,
        "find",
        json!({
            "collection": "profiles",
            "filter": {"user.address.city": "NYC"}
        }),
    );
    assert_eq!(result["documents"].as_array().unwrap().len(), 1);

    // Update nested field
    dispatch_ok(
        &adapter,
        "update_one",
        json!({
            "collection": "profiles",
            "filter": {},
            "update": {"$set": {"user.address.zip": "10002"}}
        }),
    );

    let result = dispatch_ok(
        &adapter,
        "find_one",
        json!({
            "collection": "profiles",
            "filter": {}
        }),
    );
    assert_eq!(result["document"]["user"]["address"]["zip"], "10002");
}

// ============================================================================
// Schema Tests
// ============================================================================

#[test]
fn test_schema_set_and_get() {
    let (adapter, _temp) = create_test_adapter();

    // Create collection
    dispatch_ok(
        &adapter,
        "insert_one",
        json!({
            "collection": "validated",
            "document": {"name": "test"}
        }),
    );

    // Set schema
    dispatch_ok(
        &adapter,
        "schema_set",
        json!({
            "collection": "validated",
            "schema": {
                "type": "object",
                "properties": {
                    "name": {"type": "string"},
                    "age": {"type": "integer"}
                },
                "required": ["name"]
            }
        }),
    );

    // Get schema
    let result = dispatch_ok(
        &adapter,
        "schema_get",
        json!({
            "collection": "validated"
        }),
    );
    assert!(result.get("schema").is_some());
}

// ============================================================================
// Concurrent Safety Test
// ============================================================================

#[test]
fn test_concurrent_inserts() {
    use std::thread;

    let (adapter, _temp) = create_test_adapter();
    let adapter = adapter.clone();

    let handles: Vec<_> = (0..10)
        .map(|i| {
            let adapter = adapter.clone();
            thread::spawn(move || {
                dispatch_tool(
                    "insert_one",
                    json!({
                        "collection": "concurrent",
                        "document": {"thread": i, "value": i * 10}
                    }),
                    &adapter,
                    None,
                    None,
                    None,
                    None,
                    &None,
                    &None,
                )
                .expect("Insert should succeed")
            })
        })
        .collect();

    for handle in handles {
        handle.join().expect("Thread should not panic");
    }

    // Verify all inserts succeeded
    let result = dispatch_ok(
        &adapter,
        "count",
        json!({
            "collection": "concurrent",
            "filter": {}
        }),
    );
    assert_eq!(result["count"], 10);
}

/// #65: fulltext_analyze must inherit an index's real language (here: hungarian),
/// so its tokenization output reflects how that index actually stems — rather than
/// silently defaulting to "none" and reporting unstemmed tokens.
#[test]
fn test_fulltext_analyze_inherits_index_language_issue65() {
    let (adapter, _tmp) = create_test_adapter();

    dispatch_ok(
        &adapter,
        "insert_one",
        json!({"collection": "docs", "document": {"content": "fékpad"}}),
    );
    dispatch_ok(
        &adapter,
        "index_create",
        json!({"type": "fulltext", "collection": "docs", "field": "content", "language": "hungarian"}),
    );

    // No explicit language → inherits the index's hungarian config.
    let res = dispatch_ok(
        &adapter,
        "fulltext_analyze",
        json!({
            "text": "fékpadon fékpadot fékpad",
            "collection": "docs",
            "field": "content"
        }),
    );

    assert_eq!(res["inherited_from_index"], json!(true));
    assert_eq!(res["language"], json!("Hungarian"));

    // Hungarian stemming collapses the inflected forms to a single common stem.
    let tokens = res["tokens"].as_array().expect("tokens array");
    let stems: std::collections::HashSet<&str> = tokens
        .iter()
        .filter_map(|t| t["stemmed"].as_str())
        .collect();
    assert_eq!(
        stems.len(),
        1,
        "inflected forms should share one stem, got {:?}",
        stems
    );
}

/// #65: collection without field (or vice versa) is rejected — no silent fallback.
#[test]
fn test_fulltext_analyze_collection_without_field_errors() {
    let (adapter, _tmp) = create_test_adapter();
    let err = dispatch_err(
        &adapter,
        "fulltext_analyze",
        json!({"text": "fékpad", "collection": "docs"}),
    );
    assert!(err.to_string().contains("collection") || err.to_string().contains("field"));
}

/// #68: fulltext_search hit shape is flat: document fields at the top level,
/// engine metadata under `_`-prefix.
#[test]
fn test_fulltext_search_flat_shape_issue68() {
    let (adapter, _tmp) = create_test_adapter();

    dispatch_ok(
        &adapter,
        "insert_one",
        json!({"collection":"docs","document":{
            "title":"BKV Zrt árajánlat","year":2026,"content":"fékpad PEF-35"
        }}),
    );
    dispatch_ok(
        &adapter,
        "index_create",
        json!({"type": "fulltext", "collection":"docs","field":"content"}),
    );

    let res = dispatch_ok(
        &adapter,
        "fulltext_search",
        json!({"collection":"docs","field":"content","query":"PEF-35"}),
    );

    let results = res["results"].as_array().expect("results array");
    assert_eq!(results.len(), 1);
    let hit = &results[0];

    // Document fields at the TOP level — no nested {"document": {...}}.
    assert!(
        hit.get("document").is_none(),
        "old nested shape leaked: {hit}"
    );
    assert_eq!(hit["title"], json!("BKV Zrt árajánlat"));
    assert_eq!(hit["year"], json!(2026));
    assert!(hit["content"].as_str().unwrap().contains("PEF-35"));

    // Engine metadata under `_`-prefix.
    assert!(hit["_score"].as_f64().unwrap() > 0.0);
    assert!(hit["_matched_tokens"].is_array());
}

// ============================================================================
// rag_load_all_chunks tests (#73)
// ============================================================================

/// Helper: seed a RAG-style collection with 2 docs × 3 chunks each.
fn seed_rag_kb(adapter: &Arc<IronBaseAdapter>) {
    for doc_id in &["alpha", "beta"] {
        for idx in 0..3 {
            dispatch_ok(
                adapter,
                "insert_one",
                json!({"collection": "kb", "document": {
                    "doc_id": doc_id,
                    "chunk_index": idx,
                    "chunk_total": 3,
                    "content": format!("PEF-35 fékpad chunk {}-{}", doc_id, idx),
                    "title": format!("{} title", doc_id),
                }}),
            );
        }
    }
    dispatch_ok(
        adapter,
        "index_create",
        json!({"type": "fulltext", "collection": "kb", "field": "content"}),
    );
}

/// #73 AC1: pure load (no query) — chunks sorted by (doc_id, chunk_index) ASC,
/// no _score / _matched_tokens / _highlights on hits.
#[test]
fn test_rag_load_all_chunks_pure_load_issue73_ac1() {
    let (adapter, _tmp) = create_test_adapter();
    seed_rag_kb(&adapter);

    let res = dispatch_ok(
        &adapter,
        "rag_chunks_load",
        json!({
            "collection": "kb",
            "doc_ids": ["alpha", "beta"],
            "merge_chunks": false,
        }),
    );

    assert_eq!(res["scored"], json!(false));
    let results = res["results"].as_array().expect("results array");
    assert_eq!(results.len(), 6, "expected 6 chunks (2 docs × 3): {res}");

    // Order: alpha chunks 0,1,2 then beta chunks 0,1,2
    let expected_order = [
        ("alpha", 0),
        ("alpha", 1),
        ("alpha", 2),
        ("beta", 0),
        ("beta", 1),
        ("beta", 2),
    ];
    for (i, (did, ci)) in expected_order.iter().enumerate() {
        assert_eq!(
            results[i]["doc_id"].as_str(),
            Some(*did),
            "chunk {i} doc_id mismatch: {res}"
        );
        assert_eq!(
            results[i]["chunk_index"].as_u64(),
            Some(*ci as u64),
            "chunk {i} chunk_index mismatch: {res}"
        );
    }

    // No scoring metadata on pure-load hits
    for hit in results {
        assert!(
            hit.get("_score").is_none(),
            "pure load leaked _score: {hit}"
        );
        assert!(
            hit.get("_matched_tokens").is_none(),
            "pure load leaked _matched_tokens: {hit}"
        );
        assert!(
            hit.get("_highlights").is_none(),
            "pure load leaked _highlights: {hit}"
        );
    }
}

/// #73 AC2: scored load — every hit carries `_score` and chunks come back
/// sorted by `_score` DESC.
#[test]
fn test_rag_load_all_chunks_scored_load_issue73_ac2() {
    let (adapter, _tmp) = create_test_adapter();
    seed_rag_kb(&adapter);

    let res = dispatch_ok(
        &adapter,
        "rag_chunks_load",
        json!({
            "collection": "kb",
            "doc_ids": ["alpha", "beta"],
            "query": "PEF-35",
            "merge_chunks": false,
        }),
    );

    assert_eq!(res["scored"], json!(true));
    let results = res["results"].as_array().expect("results array");
    assert!(!results.is_empty(), "expected scored hits: {res}");

    // Every hit must have _score, and scores must be DESC
    let mut prev_score = f64::INFINITY;
    for hit in results {
        let s = hit["_score"]
            .as_f64()
            .unwrap_or_else(|| panic!("missing _score: {hit}"));
        assert!(
            s <= prev_score,
            "scores not DESC: prev={prev_score} curr={s} hit={hit}"
        );
        prev_score = s;
    }
}

/// #73 AC4: empty `doc_ids` returns empty `results`, not an error.
#[test]
fn test_rag_load_all_chunks_empty_doc_ids_issue73_ac4() {
    let (adapter, _tmp) = create_test_adapter();
    seed_rag_kb(&adapter);

    let res = dispatch_ok(
        &adapter,
        "rag_chunks_load",
        json!({
            "collection": "kb",
            "doc_ids": [],
        }),
    );
    assert_eq!(res["count"], json!(0));
    assert_eq!(res["results"].as_array().map(|a| a.len()), Some(0));
}

/// #73 AC5: missing doc_ids (not in the collection) are silently skipped.
#[test]
fn test_rag_load_all_chunks_missing_doc_ids_silent_skip_issue73_ac5() {
    let (adapter, _tmp) = create_test_adapter();
    seed_rag_kb(&adapter);

    let res = dispatch_ok(
        &adapter,
        "rag_chunks_load",
        json!({
            "collection": "kb",
            "doc_ids": ["alpha", "nonexistent-doc"],
            "merge_chunks": false,
        }),
    );
    let results = res["results"].as_array().expect("results");
    assert_eq!(
        results.len(),
        3,
        "only alpha's 3 chunks should return: {res}"
    );
    for hit in results {
        assert_eq!(hit["doc_id"].as_str(), Some("alpha"));
    }
}

/// #73 AC3: `merge_chunks=true` triggers adjacent-merge and emits
/// `chunks_in_merge` on the merged hits.
#[test]
fn test_rag_load_all_chunks_merge_chunks_issue73_ac3() {
    let (adapter, _tmp) = create_test_adapter();
    seed_rag_kb(&adapter);

    let res = dispatch_ok(
        &adapter,
        "rag_chunks_load",
        json!({
            "collection": "kb",
            "doc_ids": ["alpha"],
            "merge_chunks": true,
        }),
    );
    // 3 chunks of "alpha" with chunk_index 0,1,2 → consecutive → merged to 1
    let results = res["results"].as_array().expect("results");
    assert_eq!(results.len(), 1, "expected one merged hit: {res}");
    assert_eq!(results[0]["chunk_merged"], json!(true));
    assert_eq!(results[0]["chunks_in_merge"], json!(3));
    assert_eq!(res["chunks_merged"], json!(2)); // 2 chunks removed (3 merged into 1)
}

/// `max_chunks_per_doc` cap on rag_load_all_chunks — keeps the first N
/// chunks per doc (post-sort).
#[test]
fn test_rag_load_all_chunks_max_chunks_per_doc() {
    let (adapter, _tmp) = create_test_adapter();
    seed_rag_kb(&adapter);

    let res = dispatch_ok(
        &adapter,
        "rag_chunks_load",
        json!({
            "collection": "kb",
            "doc_ids": ["alpha", "beta"],
            "merge_chunks": false,
            "max_chunks_per_doc": 2,
        }),
    );
    let results = res["results"].as_array().expect("results");
    let alpha = results
        .iter()
        .filter(|h| h["doc_id"].as_str() == Some("alpha"))
        .count();
    let beta = results
        .iter()
        .filter(|h| h["doc_id"].as_str() == Some("beta"))
        .count();
    assert_eq!(alpha, 2, "alpha cap violated: {res}");
    assert_eq!(beta, 2, "beta cap violated: {res}");
}

// ============================================================================
// `search` tool — Stage A (passage-anchored unit + compact contract)
// ============================================================================

/// Seed a RAG-style collection with 2 docs × 2 chunks + fulltext + vector index,
/// so `search` (auto-embed via hybrid) has both modalities available.
fn seed_search_kb(adapter: &Arc<IronBaseAdapter>) {
    // Both docs share the token "berendezés" so a single-token query qualifies
    // both under doc-scope AND (used by the equivalence test).
    let docs = vec![
        (
            "alpha",
            0,
            "fékpad PEF-35 berendezés leírás és specifikáció",
            vec![0.9_f64, 0.1, 0.0, 0.0],
        ),
        (
            "alpha",
            1,
            "fékpad PEF-35 ár 4 280 000 Ft",
            vec![0.8, 0.2, 0.0, 0.0],
        ),
        (
            "beta",
            0,
            "kombinált vizsgasori berendezés árajánlat",
            vec![0.1, 0.9, 0.0, 0.0],
        ),
        (
            "beta",
            1,
            "vizsgasori garancia és telepítés",
            vec![0.0, 0.8, 0.2, 0.0],
        ),
    ];
    for (doc_id, idx, content, emb) in &docs {
        dispatch_ok(
            adapter,
            "insert_one",
            json!({"collection": "kb", "document": {
                "doc_id": doc_id, "chunk_index": idx,
                "content": content, "title": format!("{} címe", doc_id),
                "embedding": emb,
            }}),
        );
    }
    dispatch_ok(
        adapter,
        "index_create",
        json!({"type": "fulltext", "collection": "kb", "field": "content"}),
    );
    dispatch_ok(
        adapter,
        "index_create",
        json!({"type": "vector", "collection": "kb", "field": "embedding", "dim": 4, "metric": "cosine"}),
    );
}

/// Stage A: `search` is listed and returns the compact document/passage shape
/// with NO embedding, NO `_`-prefixed engine metadata, and verdict "unknown".
#[test]
fn test_search_compact_contract_stage_a() {
    let (adapter, _tmp) = create_test_adapter();
    seed_search_kb(&adapter);

    // Listed in tools/list
    let tools = get_tools_list();
    let names: Vec<&str> = tools["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    assert!(names.contains(&"search"), "`search` not listed");

    // No embedding provider in the test harness → search degrades to BM25-only
    // (P7), surfaced via the `degraded` field (P6). The contract is unchanged.
    let res = dispatch_ok(
        &adapter,
        "search",
        json!({"collection": "kb", "query": "PEF-35 fékpad"}),
    );

    assert_eq!(
        res["verdict"],
        json!("unknown"),
        "Stage A verdict must be unknown"
    );
    assert!(
        res.get("degraded").is_some(),
        "BM25-only degradation must be surfaced (P6)"
    );
    let docs = res["documents"].as_array().expect("documents array");
    assert!(!docs.is_empty(), "expected documents: {res}");

    for d in docs {
        assert!(d.get("doc_id").is_some(), "doc missing doc_id: {d}");
        assert!(d.get("relevance").is_some(), "doc missing relevance: {d}");
        let passages = d["passages"].as_array().expect("passages array");
        assert!(!passages.is_empty(), "doc has no passages: {d}");
        // Document level: no embedding, no engine metadata leaked.
        let obj = d.as_object().unwrap();
        for k in obj.keys() {
            assert!(
                !k.starts_with('_'),
                "engine metadata leaked at doc level: {k}"
            );
            assert_ne!(k, "embedding", "embedding leaked at doc level");
            assert_ne!(k, "chunks", "raw chunks leaked at doc level");
        }
        for p in passages {
            let po = p.as_object().expect("passage object");
            // Passage carries ONLY text — no embedding, no scores, no chunk_index.
            for k in po.keys() {
                assert_eq!(k, "text", "passage leaked non-text key: {k}");
            }
            assert!(p["text"].as_str().map(|s| !s.is_empty()).unwrap_or(false));
        }
    }
}

/// Stage A: mechanism parameters are rejected (P6 — not silently ignored).
#[test]
fn test_search_rejects_mechanism_params_stage_a() {
    let (adapter, _tmp) = create_test_adapter();
    seed_search_kb(&adapter);

    for bad in [
        "rrf_k",
        "vector_weight",
        "match_scope",
        "merge_chunks",
        "group_by_document",
    ] {
        let err = dispatch_err(
            &adapter,
            "search",
            json!({"collection": "kb", "query": "x", bad: 1}),
        );
        assert!(
            err.to_string().contains(bad) || err.to_string().contains("intent-only"),
            "param '{bad}' should be rejected with a pointer, got: {err}"
        );
    }
}

/// Determinism: many documents tied on the same fused score must resolve to the
/// SAME order every run (deterministic tie-break on chunk id), not vary with
/// HashMap-drain input order. Guards the shared `retrieve_and_fuse`/rerank sorts.
#[test]
fn test_search_deterministic_order_on_tied_scores() {
    let (adapter, _tmp) = create_test_adapter();
    // 8 single-chunk docs with IDENTICAL content → identical fulltext score → ties.
    for i in 0..8 {
        dispatch_ok(
            &adapter,
            "insert_one",
            json!({"collection": "kb", "document": {
                "doc_id": format!("doc{i}"),
                "chunk_index": 0,
                "content": "fékpad PEF-35 berendezés",
                "title": format!("doc{i}"),
            }}),
        );
    }
    dispatch_ok(
        &adapter,
        "index_create",
        json!({"type": "fulltext", "collection": "kb", "field": "content"}),
    );

    let run = || {
        let res = dispatch_ok(
            &adapter,
            "search",
            json!({"collection": "kb", "query": "fékpad PEF-35", "limit": 8}),
        );
        res["documents"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|d| d["doc_id"].as_str().map(|s| s.to_string()))
            .collect::<Vec<_>>()
    };

    let first = run();
    assert!(!first.is_empty(), "expected tied docs");
    for _ in 0..5 {
        assert_eq!(first, run(), "tied-score doc order must be deterministic");
    }
}

/// Stage A (review #2): a benign/unknown key (NOT a mechanism param) is tolerated,
/// not hard-rejected — matching every other tool's serde-ignores-unknowns behavior,
/// so protocol- or client-injected extras don't break `search`.
#[test]
fn test_search_tolerates_benign_unknown_key_stage_a() {
    let (adapter, _tmp) = create_test_adapter();
    seed_search_kb(&adapter);

    // `_meta`-style extra and an arbitrary unknown key must NOT be rejected.
    let res = dispatch_ok(
        &adapter,
        "search",
        json!({"collection": "kb", "query": "PEF-35 fékpad", "_meta": {"x": 1}, "foo": "bar"}),
    );
    assert_eq!(res["verdict"], json!("unknown"));
}

/// Stage A: `format: context_block` returns a citation-marked text block.
#[test]
fn test_search_context_block_format_stage_a() {
    let (adapter, _tmp) = create_test_adapter();
    seed_search_kb(&adapter);

    let res = dispatch_ok(
        &adapter,
        "search",
        json!({"collection": "kb", "query": "PEF-35 fékpad", "format": "context_block"}),
    );
    assert_eq!(res["verdict"], json!("unknown"));
    let ctx = res["context"].as_str().expect("context string");
    assert!(
        ctx.contains('['),
        "context block should carry citation markers: {ctx}"
    );
    assert!(
        res.get("documents").is_none(),
        "context_block must not also emit documents"
    );
}
