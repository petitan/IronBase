//! MCP Server Integration Tests
//!
//! Tests for the mcp-ironbase-server library components:
//! - Tools (get_tools_list, dispatch_tool)
//! - Prompts (get_prompts_list, get_prompt_content)
//! - Adapter (IronBaseAdapter CRUD operations)

use mcp_ironbase::{
    dispatch_tool, get_prompt_content, get_prompts_list, get_tools_list, IronBaseAdapter,
};
use serde_json::json;
use std::sync::Arc;
use tempfile::TempDir;

// ============================================================
// Helper Functions
// ============================================================

fn create_test_adapter() -> (Arc<IronBaseAdapter>, TempDir) {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let db_path = temp_dir.path().join("test.mlite");
    let adapter = Arc::new(IronBaseAdapter::new(&db_path).expect("Failed to create adapter"));
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
    // Expected: 18 prompts (including fulltext-search, acl-guide, database-admin, security-guide, listener-config)
    assert_eq!(prompts.len(), 18);
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
    assert!(prompt_names.contains(&"fulltext-search"));
    assert!(prompt_names.contains(&"acl-guide"));
    assert!(prompt_names.contains(&"database-admin"));
    assert!(prompt_names.contains(&"security-guide"));
    assert!(prompt_names.contains(&"listener-config"));
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

#[test]
fn test_get_prompt_content_fulltext_search() {
    let result = get_prompt_content("fulltext-search", &json!({}));
    assert!(result.is_some());
    let content = result.unwrap();
    assert!(content.get("messages").is_some());
    // Verify content mentions TF-IDF
    let text = content["messages"][0]["content"]["text"].as_str().unwrap();
    assert!(text.contains("TF-IDF"));
}

#[test]
fn test_get_prompt_content_fulltext_search_with_language() {
    let args = json!({"language": "english"});
    let result = get_prompt_content("fulltext-search", &args);
    assert!(result.is_some());
    let content = result.unwrap();
    let text = content["messages"][0]["content"]["text"].as_str().unwrap();
    assert!(text.contains("english"));
}

#[test]
fn test_get_prompt_content_acl_guide() {
    let result = get_prompt_content("acl-guide", &json!({}));
    assert!(result.is_some());
    let content = result.unwrap();
    let text = content["messages"][0]["content"]["text"].as_str().unwrap();
    assert!(text.contains("interface:localhost"));
    assert!(text.contains("permissions"));
}

#[test]
fn test_get_prompt_content_database_admin() {
    let result = get_prompt_content("database-admin", &json!({}));
    assert!(result.is_some());
    let content = result.unwrap();
    let text = content["messages"][0]["content"]["text"].as_str().unwrap();
    assert!(text.contains("db_stats"));
    assert!(text.contains("compact"));
}

#[test]
fn test_get_prompt_content_security_guide() {
    let result = get_prompt_content("security-guide", &json!({}));
    assert!(result.is_some());
    let content = result.unwrap();
    let text = content["messages"][0]["content"]["text"].as_str().unwrap();
    assert!(text.contains("API Key"));
    assert!(text.contains("TLS"));
}

#[test]
fn test_get_prompt_content_listener_config() {
    let result = get_prompt_content("listener-config", &json!({}));
    assert!(result.is_some());
    let content = result.unwrap();
    let text = content["messages"][0]["content"]["text"].as_str().unwrap();
    assert!(text.contains("listener_add"));
    assert!(text.contains("HTTPS"));
}

// ============================================================
// Adapter Basic Tests
// ============================================================

#[test]
fn test_adapter_creation() {
    let (adapter, _temp) = create_test_adapter();
    let collections = adapter.list_collections();
    // _system.scripts and _system.script_versions are automatically created
    // They should be visible in list_collections (hidden: false by default)
    assert!(
        collections.len() <= 2,
        "Expected at most 2 system collections, got {:?}",
        collections
    );
}

#[test]
fn test_adapter_stats() {
    let (adapter, _temp) = create_test_adapter();
    let stats = adapter.stats();
    assert!(stats.get("collections").is_some());
    assert!(stats.get("database").is_some());
    assert!(stats.get("summary").is_some());
    assert!(stats["summary"].get("collection_count").is_some());
}

// ============================================================
// Tool Dispatch Tests - Database Management
// ============================================================

#[test]
fn test_dispatch_db_stats() {
    let (adapter, _temp) = create_test_adapter();
    let result = dispatch_tool("db_stats", json!({}), &adapter, None, None, None, None);
    assert!(result.is_ok());
}

#[test]
fn test_dispatch_db_checkpoint() {
    let (adapter, _temp) = create_test_adapter();
    let result = dispatch_tool("db_checkpoint", json!({}), &adapter, None, None, None, None);
    assert!(result.is_ok());
    let value = result.unwrap();
    assert_eq!(value.get("success"), Some(&json!(true)));
}

#[test]
fn test_dispatch_collection_list() {
    let (adapter, _temp) = create_test_adapter();
    let result = dispatch_tool("collection_list", json!({}), &adapter, None, None, None, None);
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
    let result = dispatch_tool("insert_one", params, &adapter, None, None, None, None);
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
    let result = dispatch_tool("insert_many", params, &adapter, None, None, None, None);
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
        None,
        None,
        None,
        None,
    )
    .unwrap();

    // Find it
    let result = dispatch_tool(
        "find",
        json!({"collection": "users", "query": {"name": "Alice"}}),
        &adapter,
        None,
        None,
        None,
        None,
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
        None,
        None,
        None,
        None,
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
        None,
        None,
        None,
        None,
    );
    assert!(result.is_ok());
    let value = result.unwrap();
    assert_eq!(value.get("count"), Some(&json!(2)));
}

#[test]
fn test_dispatch_find_with_include_total() {
    let (adapter, _temp) = create_test_adapter();

    // Insert multiple documents
    dispatch_tool(
        "insert_many",
        json!({
            "collection": "users",
            "documents": [
                {"name": "Alice", "age": 30},
                {"name": "Bob", "age": 25},
                {"name": "Carol", "age": 35},
                {"name": "Dave", "age": 28},
                {"name": "Eve", "age": 32}
            ]
        }),
        &adapter,
        None,
        None,
        None,
        None,
    )
    .unwrap();

    // Find with limit and include_total for pagination
    let result = dispatch_tool(
        "find",
        json!({
            "collection": "users",
            "query": {},
            "limit": 2,
            "skip": 1,
            "include_total": true
        }),
        &adapter,
        None,
        None,
        None,
        None,
    );
    assert!(result.is_ok());
    let value = result.unwrap();

    // count = returned documents (2)
    assert_eq!(value.get("count"), Some(&json!(2)));

    // total = all matching documents before limit/skip (5)
    assert_eq!(value.get("total"), Some(&json!(5)));
}

#[test]
fn test_dispatch_find_without_include_total() {
    let (adapter, _temp) = create_test_adapter();

    // Insert documents
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
        None,
        None,
        None,
        None,
    )
    .unwrap();

    // Find without include_total (default behavior)
    let result = dispatch_tool(
        "find",
        json!({
            "collection": "users",
            "query": {},
            "limit": 2
        }),
        &adapter,
        None,
        None,
        None,
        None,
    );
    assert!(result.is_ok());
    let value = result.unwrap();

    // count should be present
    assert_eq!(value.get("count"), Some(&json!(2)));

    // total should NOT be present when include_total is false/missing
    assert!(value.get("total").is_none());
}

#[test]
fn test_dispatch_find_one() {
    let (adapter, _temp) = create_test_adapter();

    dispatch_tool(
        "insert_one",
        json!({"collection": "users", "document": {"name": "Alice", "age": 30}}),
        &adapter,
        None,
        None,
        None,
        None,
    )
    .unwrap();

    let result = dispatch_tool(
        "find_one",
        json!({"collection": "users", "query": {"name": "Alice"}}),
        &adapter,
        None,
        None,
        None,
        None,
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
        None,
        None,
        None,
        None,
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
        None,
        None,
        None,
        None,
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
        None,
        None,
        None,
        None,
    )
    .unwrap();

    let result = dispatch_tool(
        "delete_one",
        json!({"collection": "users", "filter": {"name": "Alice"}}),
        &adapter,
        None,
        None,
        None,
        None,
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
        None,
        None,
        None,
        None,
    )
    .unwrap();

    let result = dispatch_tool(
        "count_documents",
        json!({"collection": "users", "query": {}}),
        &adapter,
        None,
        None,
        None,
        None,
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
        None,
        None,
        None,
        None,
    )
    .unwrap();

    let result = dispatch_tool(
        "distinct",
        json!({"collection": "users", "field": "city"}),
        &adapter,
        None,
        None,
        None,
        None,
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
        None,
        None,
        None,
        None,
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
        None,
        None,
        None,
        None,
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
        None,
        None,
        None,
        None,
    )
    .unwrap();

    let result = dispatch_tool(
        "index_create",
        json!({"collection": "users", "field": "name"}),
        &adapter,
        None,
        None,
        None,
        None,
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
        None,
        None,
        None,
        None,
    )
    .unwrap();

    let result = dispatch_tool(
        "index_create",
        json!({"collection": "users", "fields": ["name", "age"]}),
        &adapter,
        None,
        None,
        None,
        None,
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
        None,
        None,
        None,
        None,
    )
    .unwrap();

    dispatch_tool(
        "index_create",
        json!({"collection": "users", "field": "name"}),
        &adapter,
        None,
        None,
        None,
        None,
    )
    .unwrap();

    let result = dispatch_tool(
        "index_list",
        json!({"collection": "users"}),
        &adapter,
        None,
        None,
        None,
        None,
    );
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
        None,
        None,
        None,
        None,
    )
    .unwrap();

    let result = dispatch_tool(
        "schema_get",
        json!({"collection": "users"}),
        &adapter,
        None,
        None,
        None,
        None,
    );
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
        None,
        None,
        None,
        None,
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
        None,
        None,
        None,
        None,
    );
    assert!(set_result.is_ok());
    assert_eq!(set_result.unwrap().get("schema_set"), Some(&json!(true)));

    let get_result = dispatch_tool(
        "schema_get",
        json!({"collection": "users"}),
        &adapter,
        None,
        None,
        None,
        None,
    );
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
    let result = dispatch_tool("nonexistent_tool", json!({}), &adapter, None, None, None, None);
    assert!(result.is_err());
}

#[test]
fn test_dispatch_missing_required_param() {
    let (adapter, _temp) = create_test_adapter();
    // Missing "collection" parameter
    let result = dispatch_tool(
        "insert_one",
        json!({"document": {}}),
        &adapter,
        None,
        None,
        None,
        None,
    );
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
        None,
        None,
        None,
        None,
    );
    assert!(result.is_err());
}

// ============================================================
// Concurrency Tests
// ============================================================

use std::thread;

#[test]
fn test_concurrent_inserts() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let db_path = temp_dir.path().join("test.mlite");
    let adapter = Arc::new(IronBaseAdapter::new(&db_path).expect("Failed to create adapter"));

    let mut handles = vec![];
    let num_threads = 10;
    let inserts_per_thread = 10;

    for thread_id in 0..num_threads {
        let adapter_clone = Arc::clone(&adapter);
        let handle = thread::spawn(move || {
            for i in 0..inserts_per_thread {
                let params = json!({
                    "collection": "concurrent_test",
                    "document": {"thread": thread_id, "index": i}
                });
                let result = dispatch_tool("insert_one", params, &adapter_clone, None, None, None, None);
                assert!(result.is_ok(), "Insert failed: {:?}", result.err());
            }
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.join().expect("Thread panicked");
    }

    // Verify all documents were inserted
    let count_result = dispatch_tool(
        "count_documents",
        json!({"collection": "concurrent_test", "query": {}}),
        &adapter,
        None,
        None,
        None,
        None,
    );
    assert!(count_result.is_ok());
    let value = count_result.unwrap();
    assert_eq!(
        value.get("count"),
        Some(&json!(num_threads * inserts_per_thread))
    );
}

#[test]
fn test_concurrent_reads() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let db_path = temp_dir.path().join("test.mlite");
    let adapter = Arc::new(IronBaseAdapter::new(&db_path).expect("Failed to create adapter"));

    // Insert test data first
    for i in 0..100 {
        dispatch_tool(
            "insert_one",
            json!({"collection": "read_test", "document": {"id": i, "value": i * 10}}),
            &adapter,
            None,
            None,
            None,
            None,
        )
        .unwrap();
    }

    let mut handles = vec![];
    let num_threads = 10;

    for _ in 0..num_threads {
        let adapter_clone = Arc::clone(&adapter);
        let handle = thread::spawn(move || {
            for _ in 0..10 {
                let result = dispatch_tool(
                    "find",
                    json!({"collection": "read_test", "query": {}}),
                    &adapter_clone,
                    None,
                    None,
                    None,
                    None,
                );
                assert!(result.is_ok());
                let value = result.unwrap();
                assert_eq!(value.get("count"), Some(&json!(100)));
            }
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.join().expect("Thread panicked");
    }
}

#[test]
fn test_concurrent_mixed_operations() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let db_path = temp_dir.path().join("test.mlite");
    let adapter = Arc::new(IronBaseAdapter::new(&db_path).expect("Failed to create adapter"));

    // Initial data
    for i in 0..50 {
        dispatch_tool(
            "insert_one",
            json!({"collection": "mixed_test", "document": {"id": i, "status": "initial"}}),
            &adapter,
            None,
            None,
            None,
            None,
        )
        .unwrap();
    }

    let mut handles = vec![];

    // Reader threads
    for _ in 0..5 {
        let adapter_clone = Arc::clone(&adapter);
        handles.push(thread::spawn(move || {
            for _ in 0..20 {
                let _ = dispatch_tool(
                    "find",
                    json!({"collection": "mixed_test", "query": {}}),
                    &adapter_clone,
                    None,
                    None,
                    None,
                    None,
                );
            }
        }));
    }

    // Writer threads
    for thread_id in 0..3 {
        let adapter_clone = Arc::clone(&adapter);
        handles.push(thread::spawn(move || {
            for i in 0..10 {
                let _ = dispatch_tool(
                    "insert_one",
                    json!({
                        "collection": "mixed_test",
                        "document": {"id": 100 + thread_id * 10 + i, "status": "new"}
                    }),
                    &adapter_clone,
                    None,
                    None,
                    None,
                    None,
                );
            }
        }));
    }

    // Update threads
    for _ in 0..2 {
        let adapter_clone = Arc::clone(&adapter);
        handles.push(thread::spawn(move || {
            for i in 0..10 {
                let _ = dispatch_tool(
                    "update_one",
                    json!({
                        "collection": "mixed_test",
                        "filter": {"id": i},
                        "update": {"$set": {"status": "updated"}}
                    }),
                    &adapter_clone,
                    None,
                    None,
                    None,
                    None,
                );
            }
        }));
    }

    for handle in handles {
        handle.join().expect("Thread panicked");
    }

    // Verify data integrity
    let count_result = dispatch_tool(
        "count_documents",
        json!({"collection": "mixed_test", "query": {}}),
        &adapter,
        None,
        None,
        None,
        None,
    );
    assert!(count_result.is_ok());
    let count = count_result
        .unwrap()
        .get("count")
        .unwrap()
        .as_i64()
        .unwrap();
    // 50 initial + 30 new (3 threads x 10)
    assert_eq!(count, 80);
}

#[test]
fn test_concurrent_different_collections() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let db_path = temp_dir.path().join("test.mlite");
    let adapter = Arc::new(IronBaseAdapter::new(&db_path).expect("Failed to create adapter"));

    let mut handles = vec![];

    // Each thread operates on its own collection
    for thread_id in 0..5 {
        let adapter_clone = Arc::clone(&adapter);
        let handle = thread::spawn(move || {
            let collection = format!("collection_{}", thread_id);
            for i in 0..20 {
                dispatch_tool(
                    "insert_one",
                    json!({
                        "collection": collection,
                        "document": {"id": i}
                    }),
                    &adapter_clone,
                    None,
                    None,
                    None,
                    None,
                )
                .unwrap();
            }
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.join().expect("Thread panicked");
    }

    // Verify each collection has 20 documents
    for thread_id in 0..5 {
        let collection = format!("collection_{}", thread_id);
        let count_result = dispatch_tool(
            "count_documents",
            json!({"collection": collection, "query": {}}),
            &adapter,
            None,
            None,
            None,
            None,
        );
        assert!(count_result.is_ok());
        assert_eq!(count_result.unwrap().get("count"), Some(&json!(20)));
    }
}

#[test]
fn test_concurrent_aggregation() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let db_path = temp_dir.path().join("test.mlite");
    let adapter = Arc::new(IronBaseAdapter::new(&db_path).expect("Failed to create adapter"));

    // Insert test data
    let categories = ["A", "B", "C"];
    for i in 0..60 {
        let cat = categories[i % 3];
        dispatch_tool(
            "insert_one",
            json!({
                "collection": "agg_test",
                "document": {"category": cat, "value": (i as i64) * 10}
            }),
            &adapter,
            None,
            None,
            None,
            None,
        )
        .unwrap();
    }

    let mut handles = vec![];

    // Multiple threads run the same aggregation concurrently
    for _ in 0..5 {
        let adapter_clone = Arc::clone(&adapter);
        handles.push(thread::spawn(move || {
            for _ in 0..5 {
                let result = dispatch_tool(
                    "aggregate",
                    json!({
                        "collection": "agg_test",
                        "pipeline": [
                            {"$group": {"_id": "$category", "total": {"$sum": "$value"}}}
                        ]
                    }),
                    &adapter_clone,
                    None,
                    None,
                    None,
                    None,
                );
                assert!(result.is_ok());
                let value = result.unwrap();
                // Should have 3 groups (A, B, C)
                assert_eq!(value.get("count"), Some(&json!(3)));
            }
        }));
    }

    for handle in handles {
        handle.join().expect("Thread panicked");
    }
}

#[test]
fn test_concurrent_index_operations() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let db_path = temp_dir.path().join("test.mlite");
    let adapter = Arc::new(IronBaseAdapter::new(&db_path).expect("Failed to create adapter"));

    // Create collection with data
    for i in 0..100 {
        dispatch_tool(
            "insert_one",
            json!({
                "collection": "index_test",
                "document": {"field_a": i, "field_b": i * 2, "field_c": format!("val_{}", i)}
            }),
            &adapter,
            None,
            None,
            None,
            None,
        )
        .unwrap();
    }

    let mut handles = vec![];

    // Create indexes concurrently
    let fields = ["field_a", "field_b", "field_c"];
    for field in fields {
        let adapter_clone = Arc::clone(&adapter);
        let field_owned = field.to_string();
        handles.push(thread::spawn(move || {
            let result = dispatch_tool(
                "index_create",
                json!({
                    "collection": "index_test",
                    "field": field_owned
                }),
                &adapter_clone,
                None,
                None,
                None,
                None,
            );
            // Index creation may succeed or fail if concurrent
            // Just verify no panic
            let _ = result;
        }));
    }

    for handle in handles {
        handle.join().expect("Thread panicked");
    }

    // Verify indexes were created
    let list_result = dispatch_tool(
        "index_list",
        json!({"collection": "index_test"}),
        &adapter,
        None,
        None,
        None,
        None,
    );
    assert!(list_result.is_ok());
}

#[test]
fn test_concurrent_delete_and_read() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let db_path = temp_dir.path().join("test.mlite");
    let adapter = Arc::new(IronBaseAdapter::new(&db_path).expect("Failed to create adapter"));

    // Insert test data
    for i in 0..100 {
        dispatch_tool(
            "insert_one",
            json!({
                "collection": "delete_test",
                "document": {"id": i}
            }),
            &adapter,
            None,
            None,
            None,
            None,
        )
        .unwrap();
    }

    let mut handles = vec![];

    // Reader thread
    let adapter_read = Arc::clone(&adapter);
    handles.push(thread::spawn(move || {
        for _ in 0..50 {
            let result = dispatch_tool(
                "find",
                json!({"collection": "delete_test", "query": {}}),
                &adapter_read,
                None,
                None,
                None,
                None,
            );
            assert!(result.is_ok());
        }
    }));

    // Deleter thread
    let adapter_delete = Arc::clone(&adapter);
    handles.push(thread::spawn(move || {
        for i in 0..50 {
            let _ = dispatch_tool(
                "delete_one",
                json!({
                    "collection": "delete_test",
                    "filter": {"id": i}
                }),
                &adapter_delete,
                None,
                None,
                None,
                None,
            );
        }
    }));

    for handle in handles {
        handle.join().expect("Thread panicked");
    }

    // Final count should be around 50 (100 - 50 deleted)
    let count_result = dispatch_tool(
        "count_documents",
        json!({"collection": "delete_test", "query": {}}),
        &adapter,
        None,
        None,
        None,
        None,
    );
    assert!(count_result.is_ok());
    let count = count_result
        .unwrap()
        .get("count")
        .unwrap()
        .as_i64()
        .unwrap();
    assert_eq!(count, 50);
}

#[test]
fn test_stress_many_concurrent_operations() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let db_path = temp_dir.path().join("test.mlite");
    let adapter = Arc::new(IronBaseAdapter::new(&db_path).expect("Failed to create adapter"));

    let mut handles = vec![];
    let num_threads = 20;

    for thread_id in 0..num_threads {
        let adapter_clone = Arc::clone(&adapter);
        handles.push(thread::spawn(move || {
            for i in 0..50 {
                // Mix of operations
                match i % 4 {
                    0 => {
                        let _ = dispatch_tool(
                            "insert_one",
                            json!({
                                "collection": "stress_test",
                                "document": {"thread": thread_id, "seq": i}
                            }),
                            &adapter_clone,
                            None,
                            None,
                            None,
                            None,
                        );
                    }
                    1 => {
                        let _ = dispatch_tool(
                            "find",
                            json!({"collection": "stress_test", "query": {}}),
                            &adapter_clone,
                            None,
                            None,
                            None,
                            None,
                        );
                    }
                    2 => {
                        let _ = dispatch_tool(
                            "update_one",
                            json!({
                                "collection": "stress_test",
                                "filter": {"thread": thread_id},
                                "update": {"$set": {"updated": true}}
                            }),
                            &adapter_clone,
                            None,
                            None,
                            None,
                            None,
                        );
                    }
                    _ => {
                        let _ = dispatch_tool(
                            "count_documents",
                            json!({"collection": "stress_test", "query": {}}),
                            &adapter_clone,
                            None,
                            None,
                            None,
                            None,
                        );
                    }
                }
            }
        }));
    }

    for handle in handles {
        handle.join().expect("Thread panicked");
    }

    // Verify no corruption - should be able to count
    let count_result = dispatch_tool(
        "count_documents",
        json!({"collection": "stress_test", "query": {}}),
        &adapter,
        None,
        None,
        None,
        None,
    );
    assert!(count_result.is_ok());
}

// ============================================================
// Script Management Tests
// ============================================================

#[test]
fn test_script_save_new() {
    let (adapter, _temp) = create_test_adapter();
    let result = dispatch_tool(
        "script_save",
        json!({
            "name": "my_script",
            "code": "let x = 1 + 2; x",
            "description": "Simple math script"
        }),
        &adapter,
        None,
        None,
        None,
        None,
    );
    assert!(result.is_ok());
    let value = result.unwrap();
    assert_eq!(value.get("success"), Some(&json!(true)));
    assert_eq!(value.get("name"), Some(&json!("my_script")));
}

#[test]
fn test_script_save_without_description() {
    let (adapter, _temp) = create_test_adapter();
    let result = dispatch_tool(
        "script_save",
        json!({
            "name": "minimal_script",
            "code": "42"
        }),
        &adapter,
        None,
        None,
        None,
        None,
    );
    assert!(result.is_ok());
    let value = result.unwrap();
    assert_eq!(value.get("success"), Some(&json!(true)));
}

#[test]
fn test_script_get_existing() {
    let (adapter, _temp) = create_test_adapter();

    // Save a script first
    dispatch_tool(
        "script_save",
        json!({
            "name": "test_script",
            "code": "let result = 10 * 5; result",
            "description": "Multiplication test"
        }),
        &adapter,
        None,
        None,
        None,
        None,
    )
    .unwrap();

    // Get it back
    let result = dispatch_tool(
        "script_get",
        json!({"name": "test_script"}),
        &adapter,
        None,
        None,
        None,
        None,
    );
    assert!(result.is_ok());
    let value = result.unwrap();

    // script_get returns script fields directly
    assert_eq!(value.get("name"), Some(&json!("test_script")));
    assert_eq!(
        value.get("code"),
        Some(&json!("let result = 10 * 5; result"))
    );
    assert_eq!(
        value.get("description"),
        Some(&json!("Multiplication test"))
    );
}

#[test]
fn test_script_get_nonexistent() {
    let (adapter, _temp) = create_test_adapter();
    let result = dispatch_tool(
        "script_get",
        json!({"name": "nonexistent"}),
        &adapter,
        None,
        None,
        None,
        None,
    );
    // script_get returns error for non-existent script
    assert!(result.is_err());
}

#[test]
fn test_script_list_empty() {
    let (adapter, _temp) = create_test_adapter();
    let result = dispatch_tool("script_list", json!({}), &adapter, None, None, None, None);
    assert!(result.is_ok());
    let value = result.unwrap();
    let scripts = value.get("scripts").unwrap().as_array().unwrap();
    assert!(scripts.is_empty());
}

#[test]
fn test_script_list_multiple() {
    let (adapter, _temp) = create_test_adapter();

    // Save multiple scripts
    dispatch_tool(
        "script_save",
        json!({"name": "script_a", "code": "1", "description": "First"}),
        &adapter,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    dispatch_tool(
        "script_save",
        json!({"name": "script_b", "code": "2", "description": "Second"}),
        &adapter,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    dispatch_tool(
        "script_save",
        json!({"name": "script_c", "code": "3"}),
        &adapter,
        None,
        None,
        None,
        None,
    )
    .unwrap();

    // List them
    let result = dispatch_tool("script_list", json!({}), &adapter, None, None, None, None);
    assert!(result.is_ok());
    let value = result.unwrap();
    let scripts = value.get("scripts").unwrap().as_array().unwrap();
    assert_eq!(scripts.len(), 3);
    assert_eq!(value.get("count"), Some(&json!(3)));

    // Verify names are in the list
    let names: Vec<&str> = scripts
        .iter()
        .filter_map(|s| s.get("name").and_then(|n| n.as_str()))
        .collect();
    assert!(names.contains(&"script_a"));
    assert!(names.contains(&"script_b"));
    assert!(names.contains(&"script_c"));
}

#[test]
fn test_script_update_existing() {
    let (adapter, _temp) = create_test_adapter();

    // Save initial version
    dispatch_tool(
        "script_save",
        json!({
            "name": "updatable",
            "code": "version_1",
            "description": "Initial version"
        }),
        &adapter,
        None,
        None,
        None,
        None,
    )
    .unwrap();

    // Update it
    dispatch_tool(
        "script_save",
        json!({
            "name": "updatable",
            "code": "version_2",
            "description": "Updated version"
        }),
        &adapter,
        None,
        None,
        None,
        None,
    )
    .unwrap();

    // Verify update - script_get returns fields directly
    let result = dispatch_tool(
        "script_get",
        json!({"name": "updatable"}),
        &adapter,
        None,
        None,
        None,
        None,
    );
    let value = result.unwrap();
    assert_eq!(value.get("code"), Some(&json!("version_2")));
    assert_eq!(value.get("description"), Some(&json!("Updated version")));

    // Should still be only one script with this name
    let list_result = dispatch_tool("script_list", json!({}), &adapter, None, None, None, None);
    let count = list_result.unwrap().get("count").unwrap().as_i64().unwrap();
    assert_eq!(count, 1);
}

#[test]
fn test_script_delete_existing() {
    let (adapter, _temp) = create_test_adapter();

    // Save a script
    dispatch_tool(
        "script_save",
        json!({"name": "to_delete", "code": "delete me"}),
        &adapter,
        None,
        None,
        None,
        None,
    )
    .unwrap();

    // Delete it
    let result = dispatch_tool(
        "script_delete",
        json!({"name": "to_delete"}),
        &adapter,
        None,
        None,
        None,
        None,
    );
    assert!(result.is_ok());
    let value = result.unwrap();
    assert_eq!(value.get("success"), Some(&json!(true)));
    assert_eq!(value.get("deleted"), Some(&json!("to_delete")));

    // Verify it's gone (script_get returns error for non-existent)
    let get_result = dispatch_tool(
        "script_get",
        json!({"name": "to_delete"}),
        &adapter,
        None,
        None,
        None,
        None,
    );
    assert!(get_result.is_err());
}

#[test]
fn test_script_delete_nonexistent() {
    let (adapter, _temp) = create_test_adapter();
    let result = dispatch_tool(
        "script_delete",
        json!({"name": "nonexistent"}),
        &adapter,
        None,
        None,
        None,
        None,
    );
    // script_delete returns error for non-existent script
    assert!(result.is_err());
}

#[test]
fn test_script_save_missing_name() {
    let (adapter, _temp) = create_test_adapter();
    let result = dispatch_tool(
        "script_save",
        json!({"code": "some code"}),
        &adapter,
        None,
        None,
        None,
        None,
    );
    assert!(result.is_err());
}

#[test]
fn test_script_save_missing_code() {
    let (adapter, _temp) = create_test_adapter();
    let result = dispatch_tool(
        "script_save",
        json!({"name": "test"}),
        &adapter,
        None,
        None,
        None,
        None,
    );
    assert!(result.is_err());
}

#[test]
fn test_script_get_missing_name() {
    let (adapter, _temp) = create_test_adapter();
    let result = dispatch_tool("script_get", json!({}), &adapter, None, None, None, None);
    assert!(result.is_err());
}

#[test]
fn test_script_delete_missing_name() {
    let (adapter, _temp) = create_test_adapter();
    let result = dispatch_tool("script_delete", json!({}), &adapter, None, None, None, None);
    assert!(result.is_err());
}

#[test]
fn test_tools_list_contains_script_tools() {
    let result = get_tools_list();
    let tools = result.get("tools").unwrap().as_array().unwrap();
    let tool_names: Vec<&str> = tools
        .iter()
        .filter_map(|t| t.get("name").and_then(|n| n.as_str()))
        .collect();

    assert!(tool_names.contains(&"script_save"), "Missing script_save");
    assert!(tool_names.contains(&"script_list"), "Missing script_list");
    assert!(tool_names.contains(&"script_get"), "Missing script_get");
    assert!(
        tool_names.contains(&"script_delete"),
        "Missing script_delete"
    );
    assert!(tool_names.contains(&"script_run"), "Missing script_run");
}

// ============================================================
// Script Execution Tests (script_run)
// ============================================================

#[test]
fn test_script_run_simple() {
    let (adapter, _temp) = create_test_adapter();

    // Save a simple script
    dispatch_tool(
        "script_save",
        json!({
            "name": "add_numbers",
            "code": "1 + 2 + 3"
        }),
        &adapter,
        None,
        None,
        None,
        None,
    )
    .unwrap();

    // Run it
    let result = dispatch_tool(
        "script_run",
        json!({"name": "add_numbers"}),
        &adapter,
        None,
        None,
        None,
        None,
    );
    assert!(result.is_ok());
    let value = result.unwrap();
    assert_eq!(value.get("success"), Some(&json!(true)));
    assert_eq!(value.get("result"), Some(&json!(6)));
}

#[test]
fn test_script_run_with_params() {
    let (adapter, _temp) = create_test_adapter();

    dispatch_tool(
        "script_save",
        json!({
            "name": "multiply",
            "code": "params.a * params.b"
        }),
        &adapter,
        None,
        None,
        None,
        None,
    )
    .unwrap();

    let result = dispatch_tool(
        "script_run",
        json!({
            "name": "multiply",
            "params": {"a": 7, "b": 6}
        }),
        &adapter,
        None,
        None,
        None,
        None,
    );
    assert!(result.is_ok());
    let value = result.unwrap();
    assert_eq!(value.get("result"), Some(&json!(42)));
}

#[test]
fn test_script_run_with_print() {
    let (adapter, _temp) = create_test_adapter();

    dispatch_tool(
        "script_save",
        json!({
            "name": "logging_script",
            "code": r#"
                print("Starting...");
                let x = 10 + 20;
                print("Result calculated");
                x
            "#
        }),
        &adapter,
        None,
        None,
        None,
        None,
    )
    .unwrap();

    let result = dispatch_tool(
        "script_run",
        json!({"name": "logging_script"}),
        &adapter,
        None,
        None,
        None,
        None,
    );
    assert!(result.is_ok());
    let value = result.unwrap();
    assert_eq!(value.get("result"), Some(&json!(30)));

    let logs = value.get("logs").unwrap().as_array().unwrap();
    assert_eq!(logs.len(), 2);
    assert_eq!(logs[0], "Starting...");
    assert_eq!(logs[1], "Result calculated");
}

#[test]
fn test_script_run_db_operations() {
    let (adapter, _temp) = create_test_adapter();

    // Save a script that performs database operations
    dispatch_tool(
        "script_save",
        json!({
            "name": "db_script",
            "code": r#"
                // Insert some documents
                db_insert_one("products", #{ name: "Apple", price: 1.50 });
                db_insert_one("products", #{ name: "Banana", price: 0.75 });
                db_insert_one("products", #{ name: "Orange", price: 2.00 });

                // Count them
                let count = db_count("products", #{});

                // Find all
                let products = db_find("products", #{});

                #{ count: count, product_count: products.documents.len() }
            "#
        }),
        &adapter,
        None,
        None,
        None,
        None,
    )
    .unwrap();

    let result = dispatch_tool(
        "script_run",
        json!({"name": "db_script"}),
        &adapter,
        None,
        None,
        None,
        None,
    );
    assert!(result.is_ok());
    let value = result.unwrap();

    let result_obj = value.get("result").unwrap();
    assert_eq!(result_obj.get("count"), Some(&json!(3)));
    assert_eq!(result_obj.get("product_count"), Some(&json!(3)));
}

#[test]
fn test_script_run_db_update_delete() {
    let (adapter, _temp) = create_test_adapter();

    dispatch_tool(
        "script_save",
        json!({
            "name": "crud_script",
            "code": r#"
                // Insert
                db_insert_one("items", #{ id: 1, status: "active" });
                db_insert_one("items", #{ id: 2, status: "active" });
                db_insert_one("items", #{ id: 3, status: "inactive" });

                // Update
                let update_result = db_update_many("items", #{ status: "active" }, #{ "$set": #{ status: "processed" } });

                // Delete inactive
                let delete_count = db_delete_one("items", #{ status: "inactive" });

                // Final count
                let remaining = db_count("items", #{});

                #{
                    updated: update_result.modified_count,
                    deleted: delete_count,
                    remaining: remaining
                }
            "#
        }),
        &adapter,
        None,
        None,
        None,
        None,
    )
    .unwrap();

    let result = dispatch_tool(
        "script_run",
        json!({"name": "crud_script"}),
        &adapter,
        None,
        None,
        None,
        None,
    );
    assert!(result.is_ok());
    let value = result.unwrap();

    let result_obj = value.get("result").unwrap();
    assert_eq!(result_obj.get("updated"), Some(&json!(2)));
    assert_eq!(result_obj.get("deleted"), Some(&json!(1)));
    assert_eq!(result_obj.get("remaining"), Some(&json!(2)));
}

#[test]
fn test_script_run_nonexistent() {
    let (adapter, _temp) = create_test_adapter();
    let result = dispatch_tool(
        "script_run",
        json!({"name": "nonexistent_script"}),
        &adapter,
        None,
        None,
        None,
        None,
    );
    assert!(result.is_err());
}

#[test]
fn test_script_run_syntax_error() {
    let (adapter, _temp) = create_test_adapter();

    dispatch_tool(
        "script_save",
        json!({
            "name": "broken_script",
            "code": "let x = "  // Syntax error
        }),
        &adapter,
        None,
        None,
        None,
        None,
    )
    .unwrap();

    let result = dispatch_tool(
        "script_run",
        json!({"name": "broken_script"}),
        &adapter,
        None,
        None,
        None,
        None,
    );
    assert!(result.is_err());
}

#[test]
fn test_script_run_missing_name() {
    let (adapter, _temp) = create_test_adapter();
    let result = dispatch_tool("script_run", json!({}), &adapter, None, None, None, None);
    assert!(result.is_err());
}
