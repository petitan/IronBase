//! MCP Server Integration Tests
//!
//! Tests the full MCP tool dispatch pipeline with a real database.
//! Response format uses snake_case: inserted_id, matched_count, deleted_count, etc.

use mcp_ironbase::{dispatch_tool, get_tools_list, IronBaseAdapter};
use serde_json::{json, Value};
use std::sync::Arc;
use tempfile::TempDir;

/// Create a test adapter with a temporary database
fn create_test_adapter() -> (Arc<IronBaseAdapter>, TempDir) {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let db_path = temp_dir.path().join("test.mlite");
    let adapter = IronBaseAdapter::new(db_path.to_string_lossy().to_string())
        .expect("Failed to create adapter");
    (Arc::new(adapter), temp_dir)
}

/// Helper to dispatch a tool and expect success
fn dispatch_ok(adapter: &Arc<IronBaseAdapter>, name: &str, params: Value) -> Value {
    dispatch_tool(name, params, adapter, None, None, None, None, &None, &None)
        .unwrap_or_else(|e| panic!("Tool '{}' should succeed: {:?}", name, e))
}

/// Helper to dispatch a tool and expect error
fn dispatch_err(
    adapter: &Arc<IronBaseAdapter>,
    name: &str,
    params: Value,
) -> mcp_ironbase::McpError {
    match dispatch_tool(name, params, adapter, None, None, None, None, &None, &None) {
        Ok(_) => panic!("Tool '{}' should fail", name),
        Err(e) => e,
    }
}

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
    assert!(
        tool_names.contains(&"count_documents"),
        "Missing count_documents"
    );
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
        "count_documents",
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
        "count_documents",
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
        "count_documents",
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
        "count_documents",
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
        "count_documents",
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
        "count_documents",
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
        "index_create_fulltext",
        json!({"collection": "docs", "field": "content", "language": "hungarian"}),
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

/// #68: fulltext_search hit shape is now flat (consistent with hybrid_search):
/// document fields at the top level, engine metadata under `_`-prefix.
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
        "index_create_fulltext",
        json!({"collection":"docs","field":"content"}),
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
