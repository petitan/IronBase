//! MCP Tool definitions and handlers for IronBase
//!
//! This module is organized by domain:
//! - `crud` - Document CRUD operations (find, insert, update, delete)
//! - `index` - Index management and search (B+ tree, fuzzy, fulltext)
//! - `collection` - Collection management and schema
//! - `acl` - Access Control List management
//! - `script` - Rhai script management
//! - `listener` - HTTP/HTTPS listener configuration
//! - `admin` - Admin operations (require IRONBASE_ADMIN_KEY)
//! - `transaction` - Transaction management (begin, commit, rollback)
//! - `helpers` - Common helper functions

pub mod acl;
pub mod admin;
pub mod collection;
pub mod crud;
pub mod helpers;
pub mod index;
pub mod listener;
pub mod script;
pub mod transaction;

use crate::adapter::IronBaseAdapter;
use crate::api_keys::ApiKeyCache;
use crate::error::{McpError, Result};
use crate::ServerInfo;
use serde_json::{json, Value};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;

/// Get the list of all available tools for MCP tools/list
pub fn get_tools_list() -> Value {
    get_tools_list_filtered(true) // Default: show all tools (localhost assumed)
}

/// Get filtered tools list based on caller context
/// SECURITY FIX #14: Non-localhost callers don't see admin_* tools
/// This prevents information disclosure about admin capabilities
pub fn get_tools_list_filtered(is_localhost: bool) -> Value {
    let all_tools = get_all_tools_json();

    if is_localhost {
        // Localhost sees everything
        all_tools
    } else {
        // Filter out admin_* tools for non-localhost callers
        if let Some(tools_array) = all_tools.get("tools").and_then(|t| t.as_array()) {
            let filtered: Vec<Value> = tools_array
                .iter()
                .filter(|tool| {
                    tool.get("name")
                        .and_then(|n| n.as_str())
                        .map(|name| !name.starts_with("admin_"))
                        .unwrap_or(true)
                })
                .cloned()
                .collect();
            json!({ "tools": filtered })
        } else {
            all_tools
        }
    }
}

/// Maximum number of documents for in-memory sort without index
/// Operations exceeding this will return an error instead of crashing
const MAX_UNINDEXED_SORT_DOCS: usize = 100_000;

/// Dispatch a tool call to the appropriate handler
///
/// SAFETY: All tool handlers are wrapped in catch_unwind to prevent panics
/// from crashing the server. Panics are converted to McpError::Panic.
pub fn dispatch_tool(
    name: &str,
    params: Value,
    adapter: &Arc<IronBaseAdapter>,
    api_key_cache: Option<&ApiKeyCache>,
    server_info: Option<&ServerInfo>,
) -> Result<Value> {
    let tool_start = std::time::Instant::now();

    // Log the tool call for debugging
    tracing::debug!(tool = %name, "dispatch_tool started");

    // Pre-flight check for potentially dangerous operations
    if let Err(e) = preflight_check(name, &params, adapter) {
        let elapsed = tool_start.elapsed();
        tracing::warn!(
            tool = %name,
            elapsed_ms = elapsed.as_millis(),
            "Preflight check failed: {}", e
        );
        return Err(e);
    }

    // Wrap the actual dispatch in catch_unwind
    // Note: We use AssertUnwindSafe because our handlers should not panic,
    // but if they do, we want to catch it gracefully
    let result = catch_unwind(AssertUnwindSafe(|| {
        dispatch_tool_inner(name, params, adapter, api_key_cache, server_info)
    }));

    let elapsed = tool_start.elapsed();

    match result {
        Ok(inner_result) => {
            // Log tool completion with timing
            match &inner_result {
                Ok(_) => {
                    tracing::info!(
                        tool = %name,
                        elapsed_ms = elapsed.as_millis(),
                        status = "success",
                        "Tool completed"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        tool = %name,
                        elapsed_ms = elapsed.as_millis(),
                        status = "error",
                        error = %e,
                        "Tool failed"
                    );
                }
            }
            inner_result
        }
        Err(panic_info) => {
            // Extract panic message
            let panic_msg = if let Some(s) = panic_info.downcast_ref::<&str>() {
                s.to_string()
            } else if let Some(s) = panic_info.downcast_ref::<String>() {
                s.clone()
            } else {
                "unknown panic".to_string()
            };

            tracing::error!(
                tool = %name,
                elapsed_ms = elapsed.as_millis(),
                status = "panic",
                "PANIC in tool: {} - Server continues running", panic_msg
            );

            Err(McpError::Panic(format!(
                "Tool '{}' panicked: {}. The server is still running. Please report this issue.",
                name, panic_msg
            )))
        }
    }
}

/// Pre-flight check for potentially dangerous operations
/// Returns error if the operation would likely cause OOM or take too long
fn preflight_check(name: &str, params: &Value, adapter: &Arc<IronBaseAdapter>) -> Result<()> {
    // Only check find/aggregate with sort but no limit
    match name {
        "find" => {
            let has_sort = params.get("sort").is_some();
            let has_limit = params.get("limit").is_some();

            if has_sort && !has_limit {
                // Check collection size
                if let Some(collection) = params.get("collection").and_then(|c| c.as_str()) {
                    if let Ok(count) = adapter.count_documents(collection, json!({})) {
                        if count as usize > MAX_UNINDEXED_SORT_DOCS {
                            // Extract sort field name from sort parameter
                            // Format: [["field", 1]] or [["field", -1]]
                            let sort_field = params.get("sort")
                                .and_then(|s| s.as_array())
                                .and_then(|arr| arr.first())
                                .and_then(|pair| {
                                    if let Some(arr) = pair.as_array() {
                                        arr.first().and_then(|f| f.as_str())
                                    } else {
                                        pair.as_str()
                                    }
                                });

                            if let Some(field) = sort_field {
                                // Check if field is indexed
                                // list_indexes returns Vec<String> of index names like "field_1" or "field_-1"
                                let has_index = adapter.list_indexes(collection)
                                    .map(|indexes| {
                                        indexes.iter().any(|idx_name| {
                                            // Index names are like "field_1" or "compound_field1_field2_1"
                                            idx_name.starts_with(&format!("{}_", field)) ||
                                            idx_name == &format!("idx_{}", field)
                                        })
                                    })
                                    .unwrap_or(false);

                                if !has_index {
                                    return Err(McpError::OperationTooLarge(format!(
                                        "Sorting {} documents by '{}' without an index would require loading all documents into memory. \
                                        Either: (1) Create an index with index_create, (2) Add a 'limit' parameter, \
                                        or (3) Use skip/limit pagination. Max unindexed sort: {} documents.",
                                        count, field, MAX_UNINDEXED_SORT_DOCS
                                    )));
                                }
                            }
                        }
                    }
                }
            }
        }
        "aggregate" => {
            // Check for $sort without $limit in pipeline
            if let Some(pipeline) = params.get("pipeline").and_then(|p| p.as_array()) {
                let has_sort = pipeline.iter().any(|stage| stage.get("$sort").is_some());
                let has_limit = pipeline.iter().any(|stage| stage.get("$limit").is_some());

                if has_sort && !has_limit {
                    if let Some(collection) = params.get("collection").and_then(|c| c.as_str()) {
                        if let Ok(count) = adapter.count_documents(collection, json!({})) {
                            if count as usize > MAX_UNINDEXED_SORT_DOCS {
                                return Err(McpError::OperationTooLarge(format!(
                                    "Aggregation with $sort on {} documents without $limit could cause memory issues. \
                                    Add a $limit stage to the pipeline. Max: {} documents.",
                                    count, MAX_UNINDEXED_SORT_DOCS
                                )));
                            }
                        }
                    }
                }
            }
        }
        _ => {}
    }

    Ok(())
}

/// Inner dispatch function (called inside catch_unwind)
fn dispatch_tool_inner(
    name: &str,
    params: Value,
    adapter: &Arc<IronBaseAdapter>,
    api_key_cache: Option<&ApiKeyCache>,
    server_info: Option<&ServerInfo>,
) -> Result<Value> {
    match name {
        // CRUD operations
        "insert_one" | "insert_many" | "find" | "find_one" | "update_one" | "update_many"
        | "delete_one" | "delete_many" | "count_documents" | "distinct" | "aggregate" => {
            crud::dispatch(name, params, adapter)
        }

        // Index operations
        "index_create" | "index_list" | "index_create_fuzzy" | "index_create_fulltext"
        | "index_list_fulltext" | "index_drop" | "fuzzy_search" | "fulltext_search" | "explain"
        | "find_with_hint" => index::dispatch(name, params, adapter),

        // Collection operations
        "collection_list" | "collection_create" | "collection_drop" | "schema_set"
        | "schema_get" => collection::dispatch(name, params, adapter),

        // ACL operations
        "acl_list" | "acl_get" | "acl_set" | "acl_delete" | "acl_cleanup" => {
            acl::dispatch(name, params, adapter)
        }

        // Script operations
        "script_save" | "script_list" | "script_get" | "script_delete" | "script_run"
        | "script_exec" | "script_history" | "script_rollback" | "script_version_get"
        | "script_tags_add" | "script_tags_remove" | "script_stats" => {
            script::dispatch(name, params, adapter)
        }

        // Listener operations
        "listener_list" | "listener_get" | "listener_add" | "listener_delete"
        | "listener_enable" | "listener_disable" => listener::dispatch(name, params, adapter),

        // Transaction operations
        "begin_transaction" | "commit_transaction" | "rollback_transaction" | "insert_one_tx"
        | "update_one_tx" | "delete_one_tx" | "transaction_status" => {
            transaction::dispatch(name, params, adapter)
        }

        // Admin operations (db_*, admin_*)
        "db_open" | "db_stats" | "db_compact" | "db_checkpoint" | "admin_list_all_collections"
        | "admin_create_system_collection" | "admin_set_collection_flags"
        | "admin_drop_protected" | "admin_apikey_create" | "admin_apikey_list"
        | "admin_apikey_revoke" | "admin_apikey_delete" => {
            admin::dispatch(name, params, adapter, api_key_cache, server_info)
        }

        _ => Err(McpError::InvalidParams(format!("Unknown tool: {}", name))),
    }
}

/// Internal function that returns the full tools JSON
fn get_all_tools_json() -> Value {
    json!({
        "tools": [
            // Database Management
            {
                "name": "db_open",
                "title": "Open Database",
                "description": "Open or create a database file. Closes the current database and switches to the new one.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Path to the database file (.mlite)"
                        },
                        "create": {
                            "type": "boolean",
                            "description": "If true, creates new database (path must not exist). If false, opens existing (path must exist).",
                            "default": false
                        }
                    },
                    "required": ["path"]
                }
            },
            {
                "name": "db_stats",
                "title": "Database Statistics",
                "description": "Get database statistics including collection count and names",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            },
            {
                "name": "db_compact",
                "title": "Compact Database",
                "description": "Compact the database file, removing deleted documents and freeing space",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            },
            {
                "name": "db_checkpoint",
                "title": "Force Checkpoint",
                "description": "Force a checkpoint - flush all pending writes to disk",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            },
            // Collection Management
            {
                "name": "collection_list",
                "title": "List Collections",
                "description": "List all collections in the database",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            },
            {
                "name": "collection_create",
                "title": "Create Collection",
                "description": "Create a new collection (implicitly created on first insert if not exists)",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "Collection name"
                        }
                    },
                    "required": ["name"]
                }
            },
            {
                "name": "collection_drop",
                "title": "Drop Collection",
                "description": "Drop (delete) a collection and all its documents",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "Collection name to drop"
                        }
                    },
                    "required": ["name"]
                }
            },
            // Document CRUD
            {
                "name": "insert_one",
                "title": "Insert Document",
                "description": "Insert a single document into a collection",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": { "type": "string", "description": "Collection name" },
                        "document": { "type": "object", "description": "Document to insert (JSON object)" }
                    },
                    "required": ["collection", "document"]
                }
            },
            {
                "name": "insert_many",
                "title": "Insert Multiple Documents",
                "description": "Insert multiple documents into a collection",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": { "type": "string", "description": "Collection name" },
                        "documents": { "type": "array", "items": { "type": "object" }, "description": "Array of documents to insert" }
                    },
                    "required": ["collection", "documents"]
                }
            },
            {
                "name": "find",
                "title": "Find Documents",
                "description": "Find documents matching a query. Use count_documents FIRST, then use 'limit' and 'projection' to avoid context overflow!",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": { "type": "string", "description": "Collection name" },
                        "query": { "type": "object", "description": "MongoDB-style query filter" },
                        "projection": { "type": "object", "description": "Fields to include (1) or exclude (0)" },
                        "sort": { "type": "array", "description": "Sort order as array of [field, direction] pairs" },
                        "limit": { "type": "integer", "description": "Maximum number of documents to return" },
                        "skip": { "type": "integer", "description": "Number of documents to skip" },
                        "include_total": { "type": "boolean", "description": "If true, also return total count of matching documents", "default": false }
                    },
                    "required": ["collection", "query"]
                }
            },
            {
                "name": "find_one",
                "title": "Find One Document",
                "description": "Find a single document matching the query with optional projection",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": { "type": "string", "description": "Collection name" },
                        "query": { "type": "object", "description": "MongoDB-style query filter" },
                        "projection": { "type": "object", "description": "Fields to include (1) or exclude (0)" }
                    },
                    "required": ["collection", "query"]
                }
            },
            {
                "name": "update_one",
                "title": "Update Document",
                "description": "Update a single document matching the filter",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": { "type": "string", "description": "Collection name" },
                        "filter": { "type": "object", "description": "Query filter to match documents" },
                        "update": { "type": "object", "description": "Update operations ($set, $inc, $unset, $push, $pull, $addToSet, $pop)" }
                    },
                    "required": ["collection", "filter", "update"]
                }
            },
            {
                "name": "update_many",
                "title": "Update Multiple Documents",
                "description": "Update all documents matching the filter",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": { "type": "string", "description": "Collection name" },
                        "filter": { "type": "object", "description": "Query filter to match documents" },
                        "update": { "type": "object", "description": "Update operations" }
                    },
                    "required": ["collection", "filter", "update"]
                }
            },
            {
                "name": "delete_one",
                "title": "Delete Document",
                "description": "Delete a single document matching the filter",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": { "type": "string", "description": "Collection name" },
                        "filter": { "type": "object", "description": "Query filter to match document to delete" }
                    },
                    "required": ["collection", "filter"]
                }
            },
            {
                "name": "delete_many",
                "title": "Delete Multiple Documents",
                "description": "Delete all documents matching the filter",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": { "type": "string", "description": "Collection name" },
                        "filter": { "type": "object", "description": "Query filter to match documents to delete" }
                    },
                    "required": ["collection", "filter"]
                }
            },
            {
                "name": "count_documents",
                "title": "Count Documents",
                "description": "Count documents matching a query",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": { "type": "string", "description": "Collection name" },
                        "query": { "type": "object", "description": "Query filter (empty {} counts all documents)" }
                    },
                    "required": ["collection"]
                }
            },
            {
                "name": "distinct",
                "title": "Get Distinct Values",
                "description": "Get distinct values for a field",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": { "type": "string", "description": "Collection name" },
                        "field": { "type": "string", "description": "Field name to get distinct values for" },
                        "query": { "type": "object", "description": "Optional filter to apply before getting distinct values" }
                    },
                    "required": ["collection", "field"]
                }
            },
            {
                "name": "aggregate",
                "title": "Aggregation Pipeline",
                "description": "Execute an aggregation pipeline. Include {\"$limit\": 10-20} stage to avoid context overflow!",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": { "type": "string", "description": "Collection name" },
                        "pipeline": { "type": "array", "description": "Aggregation pipeline stages: $match, $group, $project, $sort, $limit, $skip" }
                    },
                    "required": ["collection", "pipeline"]
                }
            },
            // Index Management
            {
                "name": "index_create",
                "title": "Create Index",
                "description": "Create an index on a collection field(s)",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": { "type": "string", "description": "Collection name" },
                        "field": { "type": "string", "description": "Field name to index (for single-field index)" },
                        "fields": { "type": "array", "items": { "type": "string" }, "description": "Field names for compound index" },
                        "unique": { "type": "boolean", "description": "Whether the index should enforce uniqueness", "default": false }
                    },
                    "required": ["collection"]
                }
            },
            {
                "name": "index_list",
                "title": "List Indexes",
                "description": "List all indexes on a collection",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": { "type": "string", "description": "Collection name" }
                    },
                    "required": ["collection"]
                }
            },
            {
                "name": "index_create_fuzzy",
                "title": "Create Fuzzy Index",
                "description": "Create a fuzzy text index for similarity-based search",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": { "type": "string", "description": "Collection name" },
                        "field": { "type": "string", "description": "Field name to index for fuzzy search" },
                        "algorithm": { "type": "string", "description": "Similarity algorithm: jaro_winkler, levenshtein, damerau_levenshtein", "enum": ["jaro_winkler", "levenshtein", "damerau_levenshtein"], "default": "jaro_winkler" },
                        "threshold": { "type": "number", "description": "Minimum similarity threshold 0.0-1.0", "default": 0.8 }
                    },
                    "required": ["collection", "field"]
                }
            },
            {
                "name": "index_create_fulltext",
                "title": "Create Full-Text Index",
                "description": "Create a full-text search index with language-aware stemming and TF-IDF scoring",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": { "type": "string", "description": "Collection name" },
                        "field": { "type": "string", "description": "Field name to index" },
                        "language": { "type": "string", "description": "Language for stemming: hungarian, english, german, none", "enum": ["hungarian", "english", "german", "none"], "default": "none" },
                        "min_word_length": { "type": "integer", "description": "Minimum word length to index", "default": 2 },
                        "accent_folding": { "type": "boolean", "description": "Apply accent folding", "default": true }
                    },
                    "required": ["collection", "field"]
                }
            },
            {
                "name": "fulltext_search",
                "title": "Full-Text Search",
                "description": "Search documents using full-text index with TF-IDF relevance scoring",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": { "type": "string", "description": "Collection name" },
                        "field": { "type": "string", "description": "Field name with full-text index" },
                        "query": { "type": "string", "description": "Search query text" },
                        "limit": { "type": "integer", "description": "Maximum number of results", "default": 10 },
                        "skip": { "type": "integer", "description": "Number of results to skip", "default": 0 },
                        "min_score": { "type": "number", "description": "Minimum TF-IDF score threshold" },
                        "projection": { "type": "object", "description": "Fields to include (1) or exclude (0)" }
                    },
                    "required": ["collection", "field", "query"]
                }
            },
            {
                "name": "fuzzy_search",
                "title": "Fuzzy Text Search",
                "description": "Find documents using fuzzy text index (REQUIRES index_create_fuzzy first!)",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": { "type": "string", "description": "Collection name" },
                        "field": { "type": "string", "description": "Field name to search in" },
                        "query": { "type": "string", "description": "Search term to match approximately" },
                        "algorithm": { "type": "string", "description": "Override fuzzy algorithm", "enum": ["jaro_winkler", "levenshtein", "damerau_levenshtein"] },
                        "threshold": { "type": "number", "description": "Override similarity threshold 0.0-1.0" },
                        "limit": { "type": "integer", "description": "Maximum number of documents to return" },
                        "projection": { "type": "object", "description": "Fields to include (1) or exclude (0)" }
                    },
                    "required": ["collection", "field", "query"]
                }
            },
            {
                "name": "index_list_fulltext",
                "title": "List Full-Text Indexes",
                "description": "List all full-text search indexes for a collection",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": { "type": "string", "description": "Collection name" }
                    },
                    "required": ["collection"]
                }
            },
            {
                "name": "index_drop",
                "title": "Drop Index",
                "description": "Drop (delete) an index from a collection",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": { "type": "string", "description": "Collection name" },
                        "index_name": { "type": "string", "description": "Name of the index to drop" }
                    },
                    "required": ["collection", "index_name"]
                }
            },
            {
                "name": "explain",
                "title": "Explain Query",
                "description": "Explain query execution plan",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": { "type": "string", "description": "Collection name" },
                        "query": { "type": "object", "description": "Query to analyze" }
                    },
                    "required": ["collection", "query"]
                }
            },
            {
                "name": "find_with_hint",
                "title": "Find with Index Hint",
                "description": "Find documents using a specific index",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": { "type": "string", "description": "Collection name" },
                        "query": { "type": "object", "description": "Query filter" },
                        "hint": { "type": "string", "description": "Index name to use" },
                        "projection": { "type": "object", "description": "Fields to include (1) or exclude (0)" },
                        "sort": { "type": "array", "description": "Sort order" },
                        "limit": { "type": "integer", "description": "Maximum number of documents" },
                        "skip": { "type": "integer", "description": "Number of documents to skip" }
                    },
                    "required": ["collection", "query", "hint"]
                }
            },
            // Schema Management
            {
                "name": "schema_set",
                "title": "Set JSON Schema",
                "description": "Set or clear a JSON schema for a collection",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": { "type": "string", "description": "Collection name" },
                        "schema": { "type": "object", "description": "JSON Schema object. Pass null to clear schema." }
                    },
                    "required": ["collection"]
                }
            },
            {
                "name": "schema_get",
                "title": "Get JSON Schema",
                "description": "Get the JSON schema for a collection",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": { "type": "string", "description": "Collection name" }
                    },
                    "required": ["collection"]
                }
            },
            // Script Management
            {
                "name": "script_save",
                "title": "Save Script",
                "description": "Save a script to the database",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "Script name" },
                        "code": { "type": "string", "description": "Rhai script code" },
                        "description": { "type": "string", "description": "Optional description" },
                        "tags": { "type": "array", "items": { "type": "string" }, "description": "Optional tags" },
                        "dependencies": { "type": "array", "items": { "type": "string" }, "description": "Script dependencies" }
                    },
                    "required": ["name", "code"]
                }
            },
            {
                "name": "script_list",
                "title": "List Scripts",
                "description": "List all saved scripts",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "tags": { "type": "array", "items": { "type": "string" }, "description": "Filter by tags" },
                        "match_all": { "type": "boolean", "description": "Match ALL tags (AND) or ANY tag (OR)" }
                    },
                    "required": []
                }
            },
            {
                "name": "script_get",
                "title": "Get Script",
                "description": "Get a script by name",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "Script name" }
                    },
                    "required": ["name"]
                }
            },
            {
                "name": "script_delete",
                "title": "Delete Script",
                "description": "Delete a script by name",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "Script name" }
                    },
                    "required": ["name"]
                }
            },
            {
                "name": "script_run",
                "title": "Run Script",
                "description": "Run a saved script by name",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "Script name" },
                        "params": { "type": "object", "description": "Optional parameters" },
                        "max_operations": { "type": "integer", "description": "Max operations limit" }
                    },
                    "required": ["name"]
                }
            },
            {
                "name": "script_exec",
                "title": "Execute Script",
                "description": "Execute inline Rhai code",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "code": { "type": "string", "description": "Rhai script code" },
                        "params": { "type": "object", "description": "Optional parameters" },
                        "max_operations": { "type": "integer", "description": "Max operations limit" }
                    },
                    "required": ["code"]
                }
            },
            {
                "name": "script_history",
                "title": "Script History",
                "description": "Get version history of a script",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "Script name" },
                        "limit": { "type": "integer", "description": "Max versions to return" }
                    },
                    "required": ["name"]
                }
            },
            {
                "name": "script_rollback",
                "title": "Rollback Script",
                "description": "Rollback a script to a previous version",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "Script name" },
                        "version": { "type": "integer", "description": "Version to rollback to" }
                    },
                    "required": ["name", "version"]
                }
            },
            {
                "name": "script_version_get",
                "title": "Get Script Version",
                "description": "Get a specific version of a script",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "Script name" },
                        "version": { "type": "integer", "description": "Version number" }
                    },
                    "required": ["name", "version"]
                }
            },
            {
                "name": "script_tags_add",
                "title": "Add Script Tags",
                "description": "Add tags to a script",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "Script name" },
                        "tags": { "type": "array", "items": { "type": "string" }, "description": "Tags to add" }
                    },
                    "required": ["name", "tags"]
                }
            },
            {
                "name": "script_tags_remove",
                "title": "Remove Script Tags",
                "description": "Remove tags from a script",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "Script name" },
                        "tags": { "type": "array", "items": { "type": "string" }, "description": "Tags to remove" }
                    },
                    "required": ["name", "tags"]
                }
            },
            {
                "name": "script_stats",
                "title": "Script Statistics",
                "description": "Get execution statistics for a script",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "Script name" }
                    },
                    "required": ["name"]
                }
            },
            // Transaction Management
            {
                "name": "begin_transaction",
                "title": "Begin Transaction",
                "description": "Start a new transaction",
                "inputSchema": { "type": "object", "properties": {}, "required": [] }
            },
            {
                "name": "commit_transaction",
                "title": "Commit Transaction",
                "description": "Commit an active transaction",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "transaction_id": { "type": "string", "description": "Transaction ID" }
                    },
                    "required": ["transaction_id"]
                }
            },
            {
                "name": "rollback_transaction",
                "title": "Rollback Transaction",
                "description": "Rollback an active transaction",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "transaction_id": { "type": "string", "description": "Transaction ID" }
                    },
                    "required": ["transaction_id"]
                }
            },
            {
                "name": "insert_one_tx",
                "title": "Insert Document (Transaction)",
                "description": "Insert a document within a transaction",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "transaction_id": { "type": "string", "description": "Transaction ID" },
                        "collection": { "type": "string", "description": "Collection name" },
                        "document": { "type": "object", "description": "Document to insert" }
                    },
                    "required": ["transaction_id", "collection", "document"]
                }
            },
            {
                "name": "update_one_tx",
                "title": "Update Document (Transaction)",
                "description": "Update a document within a transaction",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "transaction_id": { "type": "string", "description": "Transaction ID" },
                        "collection": { "type": "string", "description": "Collection name" },
                        "filter": { "type": "object", "description": "Query filter" },
                        "update": { "type": "object", "description": "Update operators" }
                    },
                    "required": ["transaction_id", "collection", "filter", "update"]
                }
            },
            {
                "name": "delete_one_tx",
                "title": "Delete Document (Transaction)",
                "description": "Delete a document within a transaction",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "transaction_id": { "type": "string", "description": "Transaction ID" },
                        "collection": { "type": "string", "description": "Collection name" },
                        "filter": { "type": "object", "description": "Query filter" }
                    },
                    "required": ["transaction_id", "collection", "filter"]
                }
            },
            {
                "name": "transaction_status",
                "title": "Transaction Status",
                "description": "Check if there's an active write transaction",
                "inputSchema": { "type": "object", "properties": {}, "required": [] }
            },
            // Admin Operations
            {
                "name": "admin_list_all_collections",
                "title": "Admin: List All Collections",
                "description": "List ALL collections including hidden/system collections",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "admin_key": { "type": "string", "description": "Admin key" }
                    },
                    "required": ["admin_key"]
                }
            },
            {
                "name": "admin_create_system_collection",
                "title": "Admin: Create System Collection",
                "description": "Create a system collection with protected/hidden flags",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "admin_key": { "type": "string", "description": "Admin key" },
                        "name": { "type": "string", "description": "Collection name" }
                    },
                    "required": ["admin_key", "name"]
                }
            },
            {
                "name": "admin_set_collection_flags",
                "title": "Admin: Set Collection Flags",
                "description": "Set collection flags (is_system, protected, hidden)",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "admin_key": { "type": "string", "description": "Admin key" },
                        "collection": { "type": "string", "description": "Collection name" },
                        "is_system": { "type": "boolean", "description": "Mark as system collection" },
                        "protected": { "type": "boolean", "description": "Prevent deletion" },
                        "hidden": { "type": "boolean", "description": "Hide from list_collections" }
                    },
                    "required": ["admin_key", "collection"]
                }
            },
            {
                "name": "admin_drop_protected",
                "title": "Admin: Drop Protected Collection",
                "description": "Force drop a protected collection",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "admin_key": { "type": "string", "description": "Admin key" },
                        "name": { "type": "string", "description": "Collection name" }
                    },
                    "required": ["admin_key", "name"]
                }
            },
            {
                "name": "admin_apikey_create",
                "title": "Admin: Create API Key",
                "description": "Create a new API key",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "admin_key": { "type": "string", "description": "Admin key" },
                        "name": { "type": "string", "description": "Name for this API key" }
                    },
                    "required": ["admin_key", "name"]
                }
            },
            {
                "name": "admin_apikey_list",
                "title": "Admin: List API Keys",
                "description": "List all API keys (masked)",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "admin_key": { "type": "string", "description": "Admin key" }
                    },
                    "required": ["admin_key"]
                }
            },
            {
                "name": "admin_apikey_revoke",
                "title": "Admin: Revoke API Key",
                "description": "Revoke (disable) an API key",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "admin_key": { "type": "string", "description": "Admin key" },
                        "id": { "type": "integer", "description": "API key ID" }
                    },
                    "required": ["admin_key", "id"]
                }
            },
            {
                "name": "admin_apikey_delete",
                "title": "Admin: Delete API Key",
                "description": "Permanently delete an API key",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "admin_key": { "type": "string", "description": "Admin key" },
                        "id": { "type": "integer", "description": "API key ID" }
                    },
                    "required": ["admin_key", "id"]
                }
            },
            // ACL Management
            {
                "name": "acl_list",
                "title": "List ACL Rules",
                "description": "List all ACL rules",
                "inputSchema": { "type": "object", "properties": {}, "required": [] }
            },
            {
                "name": "acl_get",
                "title": "Get Collection ACL",
                "description": "Get ACL rules for a specific collection",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": { "type": "string", "description": "Collection name" }
                    },
                    "required": ["collection"]
                }
            },
            {
                "name": "acl_set",
                "title": "Set Collection ACL",
                "description": "Set ACL rules for a collection",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": { "type": "string", "description": "Collection name" },
                        "rules": { "type": "array", "description": "Array of ACL rules" }
                    },
                    "required": ["collection", "rules"]
                }
            },
            {
                "name": "acl_delete",
                "title": "Delete Collection ACL",
                "description": "Delete custom ACL rules for a collection",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": { "type": "string", "description": "Collection name" }
                    },
                    "required": ["collection"]
                }
            },
            {
                "name": "acl_cleanup",
                "title": "Cleanup Orphan ACLs",
                "description": "Remove ACL rules for deleted collections",
                "inputSchema": { "type": "object", "properties": {}, "required": [] }
            },
            // Listener Management
            {
                "name": "listener_list",
                "title": "List Listeners",
                "description": "List all configured listeners",
                "inputSchema": { "type": "object", "properties": {}, "required": [] }
            },
            {
                "name": "listener_get",
                "title": "Get Listener",
                "description": "Get configuration for a specific listener",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "description": "Listener ID" }
                    },
                    "required": ["id"]
                }
            },
            {
                "name": "listener_add",
                "title": "Add Listener",
                "description": "Add a new HTTP/HTTPS listener",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "description": "Listener ID" },
                        "bind": { "type": "string", "description": "Bind address (e.g., '0.0.0.0:8080')" },
                        "tls": { "type": "boolean", "description": "Enable TLS", "default": false },
                        "cert_path": { "type": "string", "description": "TLS certificate path" },
                        "key_path": { "type": "string", "description": "TLS private key path" },
                        "description": { "type": "string", "description": "Description" }
                    },
                    "required": ["id", "bind"]
                }
            },
            {
                "name": "listener_delete",
                "title": "Delete Listener",
                "description": "Delete a listener configuration",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "description": "Listener ID" }
                    },
                    "required": ["id"]
                }
            },
            {
                "name": "listener_enable",
                "title": "Enable Listener",
                "description": "Enable a disabled listener",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "description": "Listener ID" }
                    },
                    "required": ["id"]
                }
            },
            {
                "name": "listener_disable",
                "title": "Disable Listener",
                "description": "Disable a listener without deleting it",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "description": "Listener ID" }
                    },
                    "required": ["id"]
                }
            }
        ]
    })
}
