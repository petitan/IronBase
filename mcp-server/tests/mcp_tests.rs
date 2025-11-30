//! MCP Server Integration Tests
//!
//! Tests for the mcp-ironbase-server library components:
//! - Tools (get_tools_list, dispatch_tool)
//! - Prompts (get_prompts_list, get_prompt_content)
//! - Adapter (IronBaseAdapter CRUD operations)

use mcp_docjl::{
    dispatch_tool, get_prompt_content, get_prompts_list, get_tools_list, IronBaseAdapter,
};
use serde_json::json;
use tempfile::TempDir;

// ============================================================
// Helper Functions
// ============================================================

fn create_test_adapter() -> (IronBaseAdapter, TempDir) {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let db_path = temp_dir.path().join("test.mlite");
    let adapter = IronBaseAdapter::new(&db_path).expect("Failed to create adapter");
    (adapter, temp_dir)
}

// ============================================================
// Tools List Tests
// ============================================================

#[test]
fn test_get_tools_list_returns_tools() {
    let result = get_tools_list();
    assert!(result.is_object());
    assert!(result.get("tools").is_some());
}

#[test]
fn test_tools_list_has_expected_count() {
    let result = get_tools_list();
    let tools = result.get("tools").unwrap().as_array().unwrap();
    // Expected: 19 tools (db_stats, db_compact, db_checkpoint, collection_list, collection_create,
    // collection_drop, insert_one, insert_many, find, find_one, update_one, update_many,
    // delete_one, delete_many, count_documents, distinct, aggregate, index_create, index_list,
    // schema_set, schema_get)
    assert!(
        tools.len() >= 19,
        "Expected at least 19 tools, got {}",
        tools.len()
    );
}

#[test]
fn test_tools_list_contains_crud_tools() {
    let result = get_tools_list();
    let tools = result.get("tools").unwrap().as_array().unwrap();
    let tool_names: Vec<&str> = tools
        .iter()
        .filter_map(|t| t.get("name").and_then(|n| n.as_str()))
        .collect();

    assert!(tool_names.contains(&"insert_one"));
    assert!(tool_names.contains(&"insert_many"));
    assert!(tool_names.contains(&"find"));
    assert!(tool_names.contains(&"find_one"));
    assert!(tool_names.contains(&"update_one"));
    assert!(tool_names.contains(&"update_many"));
    assert!(tool_names.contains(&"delete_one"));
    assert!(tool_names.contains(&"delete_many"));
}

#[test]
fn test_tools_have_input_schema() {
    let result = get_tools_list();
    let tools = result.get("tools").unwrap().as_array().unwrap();

    for tool in tools {
        assert!(tool.get("name").is_some(), "Tool missing name");
        assert!(
            tool.get("description").is_some(),
            "Tool missing description"
        );
        assert!(
            tool.get("inputSchema").is_some(),
            "Tool missing inputSchema"
        );
    }
}

// ============================================================
// Prompts List Tests
// ============================================================

#[test]
fn test_get_prompts_list_returns_prompts() {
    let result = get_prompts_list();
    assert!(result.is_object());
    assert!(result.get("prompts").is_some());
}

#[test]
fn test_prompts_list_has_expected_count() {
    let result = get_prompts_list();
    let prompts = result.get("prompts").unwrap().as_array().unwrap();
    // Expected: 9 prompts
    assert_eq!(prompts.len(), 9);
}

#[test]
fn test_prompts_list_contains_expected_prompts() {
    let result = get_prompts_list();
    let prompts = result.get("prompts").unwrap().as_array().unwrap();
    let prompt_names: Vec<&str> = prompts
        .iter()
        .filter_map(|p| p.get("name").and_then(|n| n.as_str()))
        .collect();

    assert!(prompt_names.contains(&"discover-schema"));
    assert!(prompt_names.contains(&"query-operators"));
    assert!(prompt_names.contains(&"aggregation-guide"));
    assert!(prompt_names.contains(&"query-examples"));
    assert!(prompt_names.contains(&"date-query"));
}

// ============================================================
// Prompt Content Tests
// ============================================================

#[test]
fn test_get_prompt_content_query_operators() {
    let result = get_prompt_content("query-operators", &json!({}));
    assert!(result.is_some());
    let content = result.unwrap();
    assert!(content.get("messages").is_some());
}

#[test]
fn test_get_prompt_content_aggregation_guide() {
    let result = get_prompt_content("aggregation-guide", &json!({}));
    assert!(result.is_some());
}

#[test]
fn test_get_prompt_content_unknown_returns_none() {
    let result = get_prompt_content("nonexistent-prompt", &json!({}));
    assert!(result.is_none());
}

#[test]
fn test_get_prompt_content_discover_schema_with_args() {
    let args = json!({
        "collection": "users",
        "sample_size": 5
    });
    let result = get_prompt_content("discover-schema", &args);
    assert!(result.is_some());
}

// ============================================================
// Adapter Basic Tests
// ============================================================

#[test]
fn test_adapter_creation() {
    let (adapter, _temp) = create_test_adapter();
    let collections = adapter.list_collections();
    assert!(collections.is_empty());
}

#[test]
fn test_adapter_stats() {
    let (adapter, _temp) = create_test_adapter();
    let stats = adapter.stats();
    assert!(stats.get("collections").is_some());
    assert!(stats.get("collection_count").is_some());
}

// ============================================================
// Tool Dispatch Tests - Database Management
// ============================================================

#[test]
fn test_dispatch_db_stats() {
    let (adapter, _temp) = create_test_adapter();
    let result = dispatch_tool("db_stats", json!({}), &adapter);
    assert!(result.is_ok());
}

#[test]
fn test_dispatch_db_checkpoint() {
    let (adapter, _temp) = create_test_adapter();
    let result = dispatch_tool("db_checkpoint", json!({}), &adapter);
    assert!(result.is_ok());
    let value = result.unwrap();
    assert_eq!(value.get("success"), Some(&json!(true)));
}

#[test]
fn test_dispatch_collection_list() {
    let (adapter, _temp) = create_test_adapter();
    let result = dispatch_tool("collection_list", json!({}), &adapter);
    assert!(result.is_ok());
    let value = result.unwrap();
    assert!(value.get("collections").is_some());
}

// ============================================================
// Tool Dispatch Tests - CRUD Operations
// ============================================================

#[test]
fn test_dispatch_insert_one() {
    let (adapter, _temp) = create_test_adapter();
    let params = json!({
        "collection": "users",
        "document": {"name": "Alice", "age": 30}
    });
    let result = dispatch_tool("insert_one", params, &adapter);
    assert!(result.is_ok());
    let value = result.unwrap();
    assert!(value.get("inserted_id").is_some());
}

#[test]
fn test_dispatch_insert_many() {
    let (adapter, _temp) = create_test_adapter();
    let params = json!({
        "collection": "users",
        "documents": [
            {"name": "Alice", "age": 30},
            {"name": "Bob", "age": 25}
        ]
    });
    let result = dispatch_tool("insert_many", params, &adapter);
    assert!(result.is_ok());
    let value = result.unwrap();
    assert_eq!(value.get("inserted_count"), Some(&json!(2)));
}

#[test]
fn test_dispatch_find() {
    let (adapter, _temp) = create_test_adapter();

    // Insert test document
    dispatch_tool(
        "insert_one",
        json!({"collection": "users", "document": {"name": "Alice", "age": 30}}),
        &adapter,
    )
    .unwrap();

    // Find it
    let result = dispatch_tool(
        "find",
        json!({"collection": "users", "query": {"name": "Alice"}}),
        &adapter,
    );
    assert!(result.is_ok());
    let value = result.unwrap();
    assert_eq!(value.get("count"), Some(&json!(1)));
}

#[test]
fn test_dispatch_find_with_options() {
    let (adapter, _temp) = create_test_adapter();

    // Insert multiple documents
    dispatch_tool(
        "insert_many",
        json!({
            "collection": "users",
            "documents": [
                {"name": "Alice", "age": 30},
                {"name": "Bob", "age": 25},
                {"name": "Carol", "age": 35}
            ]
        }),
        &adapter,
    )
    .unwrap();

    // Find with limit and sort
    let result = dispatch_tool(
        "find",
        json!({
            "collection": "users",
            "query": {},
            "sort": [["age", 1]],
            "limit": 2
        }),
        &adapter,
    );
    assert!(result.is_ok());
    let value = result.unwrap();
    assert_eq!(value.get("count"), Some(&json!(2)));
}

#[test]
fn test_dispatch_find_one() {
    let (adapter, _temp) = create_test_adapter();

    dispatch_tool(
        "insert_one",
        json!({"collection": "users", "document": {"name": "Alice", "age": 30}}),
        &adapter,
    )
    .unwrap();

    let result = dispatch_tool(
        "find_one",
        json!({"collection": "users", "query": {"name": "Alice"}}),
        &adapter,
    );
    assert!(result.is_ok());
    let value = result.unwrap();
    assert!(value.get("document").is_some());
}

#[test]
fn test_dispatch_update_one() {
    let (adapter, _temp) = create_test_adapter();

    dispatch_tool(
        "insert_one",
        json!({"collection": "users", "document": {"name": "Alice", "age": 30}}),
        &adapter,
    )
    .unwrap();

    let result = dispatch_tool(
        "update_one",
        json!({
            "collection": "users",
            "filter": {"name": "Alice"},
            "update": {"$set": {"age": 31}}
        }),
        &adapter,
    );
    assert!(result.is_ok());
    let value = result.unwrap();
    assert_eq!(value.get("matched_count"), Some(&json!(1)));
    assert_eq!(value.get("modified_count"), Some(&json!(1)));
}

#[test]
fn test_dispatch_delete_one() {
    let (adapter, _temp) = create_test_adapter();

    dispatch_tool(
        "insert_one",
        json!({"collection": "users", "document": {"name": "Alice", "age": 30}}),
        &adapter,
    )
    .unwrap();

    let result = dispatch_tool(
        "delete_one",
        json!({"collection": "users", "filter": {"name": "Alice"}}),
        &adapter,
    );
    assert!(result.is_ok());
    let value = result.unwrap();
    assert_eq!(value.get("deleted_count"), Some(&json!(1)));
}

// ============================================================
// Tool Dispatch Tests - Query Features
// ============================================================

#[test]
fn test_dispatch_count_documents() {
    let (adapter, _temp) = create_test_adapter();

    dispatch_tool(
        "insert_many",
        json!({
            "collection": "users",
            "documents": [
                {"name": "Alice", "age": 30},
                {"name": "Bob", "age": 25}
            ]
        }),
        &adapter,
    )
    .unwrap();

    let result = dispatch_tool(
        "count_documents",
        json!({"collection": "users", "query": {}}),
        &adapter,
    );
    assert!(result.is_ok());
    let value = result.unwrap();
    assert_eq!(value.get("count"), Some(&json!(2)));
}

#[test]
fn test_dispatch_distinct() {
    let (adapter, _temp) = create_test_adapter();

    dispatch_tool(
        "insert_many",
        json!({
            "collection": "users",
            "documents": [
                {"name": "Alice", "city": "NYC"},
                {"name": "Bob", "city": "LA"},
                {"name": "Carol", "city": "NYC"}
            ]
        }),
        &adapter,
    )
    .unwrap();

    let result = dispatch_tool(
        "distinct",
        json!({"collection": "users", "field": "city"}),
        &adapter,
    );
    assert!(result.is_ok());
    let value = result.unwrap();
    assert_eq!(value.get("count"), Some(&json!(2)));
}

#[test]
fn test_dispatch_aggregate() {
    let (adapter, _temp) = create_test_adapter();

    dispatch_tool(
        "insert_many",
        json!({
            "collection": "orders",
            "documents": [
                {"product": "A", "amount": 100},
                {"product": "B", "amount": 200},
                {"product": "A", "amount": 150}
            ]
        }),
        &adapter,
    )
    .unwrap();

    let result = dispatch_tool(
        "aggregate",
        json!({
            "collection": "orders",
            "pipeline": [
                {"$group": {"_id": "$product", "total": {"$sum": "$amount"}}}
            ]
        }),
        &adapter,
    );
    assert!(result.is_ok());
    let value = result.unwrap();
    assert_eq!(value.get("count"), Some(&json!(2)));
}

// ============================================================
// Tool Dispatch Tests - Index Management
// ============================================================

#[test]
fn test_dispatch_index_create_single() {
    let (adapter, _temp) = create_test_adapter();

    // Create collection first
    dispatch_tool(
        "insert_one",
        json!({"collection": "users", "document": {"name": "Test"}}),
        &adapter,
    )
    .unwrap();

    let result = dispatch_tool(
        "index_create",
        json!({"collection": "users", "field": "name"}),
        &adapter,
    );
    assert!(result.is_ok());
    let value = result.unwrap();
    assert!(value.get("index_name").is_some());
}

#[test]
fn test_dispatch_index_create_compound() {
    let (adapter, _temp) = create_test_adapter();

    dispatch_tool(
        "insert_one",
        json!({"collection": "users", "document": {"name": "Test", "age": 25}}),
        &adapter,
    )
    .unwrap();

    let result = dispatch_tool(
        "index_create",
        json!({"collection": "users", "fields": ["name", "age"]}),
        &adapter,
    );
    assert!(result.is_ok());
}

#[test]
fn test_dispatch_index_list() {
    let (adapter, _temp) = create_test_adapter();

    dispatch_tool(
        "insert_one",
        json!({"collection": "users", "document": {"name": "Test"}}),
        &adapter,
    )
    .unwrap();

    dispatch_tool(
        "index_create",
        json!({"collection": "users", "field": "name"}),
        &adapter,
    )
    .unwrap();

    let result = dispatch_tool("index_list", json!({"collection": "users"}), &adapter);
    assert!(result.is_ok());
    let value = result.unwrap();
    assert!(value.get("indexes").is_some());
}

// ============================================================
// Tool Dispatch Tests - Schema Management
// ============================================================

#[test]
fn test_dispatch_schema_get_empty() {
    let (adapter, _temp) = create_test_adapter();

    dispatch_tool(
        "insert_one",
        json!({"collection": "users", "document": {"name": "Test"}}),
        &adapter,
    )
    .unwrap();

    let result = dispatch_tool("schema_get", json!({"collection": "users"}), &adapter);
    assert!(result.is_ok());
    let value = result.unwrap();
    assert_eq!(value.get("schema"), Some(&json!(null)));
}

#[test]
fn test_dispatch_schema_set_and_get() {
    let (adapter, _temp) = create_test_adapter();

    dispatch_tool(
        "insert_one",
        json!({"collection": "users", "document": {"name": "Test"}}),
        &adapter,
    )
    .unwrap();

    let schema = json!({
        "type": "object",
        "required": ["name"],
        "properties": {
            "name": {"type": "string"}
        }
    });

    let set_result = dispatch_tool(
        "schema_set",
        json!({"collection": "users", "schema": schema}),
        &adapter,
    );
    assert!(set_result.is_ok());
    assert_eq!(set_result.unwrap().get("schema_set"), Some(&json!(true)));

    let get_result = dispatch_tool("schema_get", json!({"collection": "users"}), &adapter);
    assert!(get_result.is_ok());
    let value = get_result.unwrap();
    assert!(value.get("schema").unwrap().is_object());
}

// ============================================================
// Tool Dispatch Tests - Error Handling
// ============================================================

#[test]
fn test_dispatch_unknown_tool() {
    let (adapter, _temp) = create_test_adapter();
    let result = dispatch_tool("nonexistent_tool", json!({}), &adapter);
    assert!(result.is_err());
}

#[test]
fn test_dispatch_missing_required_param() {
    let (adapter, _temp) = create_test_adapter();
    // Missing "collection" parameter
    let result = dispatch_tool("insert_one", json!({"document": {}}), &adapter);
    assert!(result.is_err());
}

#[test]
fn test_dispatch_invalid_param_type() {
    let (adapter, _temp) = create_test_adapter();
    // "document" should be object, not string
    let result = dispatch_tool(
        "insert_one",
        json!({"collection": "test", "document": "not an object"}),
        &adapter,
    );
    assert!(result.is_err());
}
