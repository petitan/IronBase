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
pub mod auto_embed;
pub mod collection;
pub mod crud;
mod definitions;
pub mod embedding;
pub mod helpers;
pub mod hybrid;
pub mod index;
pub mod jobs;
pub mod listener;
pub mod params;
pub mod preprocessing;
pub mod script;
pub mod transaction;
pub mod vector;

use definitions::get_all_tools_json;

use crate::adapter::IronBaseAdapter;
use crate::api_keys::ApiKeyCache;
use crate::embedding::EmbeddingManager;
use crate::error::{McpError, Result};
use crate::jobs::JobManager;
use crate::scripting::ScriptLimits;
use crate::ServerInfo;
use serde_json::{json, Value};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::AtomicBool;
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
///
/// # Arguments
/// * `name` - Tool name
/// * `params` - Tool parameters as JSON
/// * `adapter` - Database adapter
/// * `api_key_cache` - Optional API key cache for admin operations
/// * `server_info` - Optional server info for admin operations
/// * `limits` - Optional script limits for unified resource limiting
/// * `cancel_flag` - Optional cancellation flag for cooperative timeout
/// * `embedding_manager` - Optional embedding manager for embedding operations
/// * `job_manager` - Optional job manager for async operations
#[allow(clippy::too_many_arguments)]
pub fn dispatch_tool(
    name: &str,
    params: Value,
    adapter: &Arc<IronBaseAdapter>,
    api_key_cache: Option<&ApiKeyCache>,
    server_info: Option<&ServerInfo>,
    limits: Option<&ScriptLimits>,
    cancel_flag: Option<Arc<AtomicBool>>,
    embedding_manager: &Option<Arc<EmbeddingManager>>,
    job_manager: &Option<Arc<JobManager>>,
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
        dispatch_tool_inner(
            name,
            params,
            adapter,
            api_key_cache,
            server_info,
            limits,
            cancel_flag,
            embedding_manager,
            job_manager,
        )
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

            Err(McpError::panic(format!(
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
                    let count = adapter
                        .collection_count_cached(collection)
                        .or_else(|| adapter.count_documents(collection, json!({})).ok());
                    if let Some(count) = count {
                        if count as usize > MAX_UNINDEXED_SORT_DOCS {
                            // Extract sort field name from sort parameter
                            // Format: [["field", 1]] or [["field", -1]]
                            let sort_field = params
                                .get("sort")
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
                                let has_index = adapter
                                    .list_indexes(collection)
                                    .map(|indexes| {
                                        indexes.iter().any(|idx_name| {
                                            // Index names are like "field_1" or "compound_field1_field2_1"
                                            idx_name.starts_with(&format!("{}_", field))
                                                || idx_name == &format!("idx_{}", field)
                                        })
                                    })
                                    .unwrap_or(false);

                                if !has_index {
                                    return Err(McpError::operation_too_large(format!(
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
                        let count = adapter
                            .collection_count_cached(collection)
                            .or_else(|| adapter.count_documents(collection, json!({})).ok());
                        if let Some(count) = count {
                            if count as usize > MAX_UNINDEXED_SORT_DOCS {
                                return Err(McpError::operation_too_large(format!(
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
#[allow(clippy::too_many_arguments)]
fn dispatch_tool_inner(
    name: &str,
    params: Value,
    adapter: &Arc<IronBaseAdapter>,
    api_key_cache: Option<&ApiKeyCache>,
    server_info: Option<&ServerInfo>,
    limits: Option<&ScriptLimits>,
    cancel_flag: Option<Arc<AtomicBool>>,
    embedding_manager: &Option<Arc<EmbeddingManager>>,
    job_manager: &Option<Arc<JobManager>>,
) -> Result<Value> {
    match name {
        // CRUD operations (with auto-embedding support for insert)
        "insert_one" | "insert_many" | "find" | "find_one" | "update_one" | "update_many"
        | "delete_one" | "delete_many" | "count_documents" | "distinct" | "aggregate" => {
            crud::dispatch(name, params, adapter, limits, cancel_flag, embedding_manager)
        }

        // Index operations
        "index_create"
        | "index_list"
        | "index_create_fuzzy"
        | "index_create_fulltext"
        | "index_list_fulltext"
        | "index_drop"
        | "index_stats_refresh"
        | "index_stats"
        | "fuzzy_search"
        | "fulltext_search"
        | "fulltext_analyze"
        | "explain"
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
        "begin_transaction"
        | "commit_transaction"
        | "rollback_transaction"
        | "insert_one_tx"
        | "update_one_tx"
        | "delete_one_tx"
        | "transaction_status" => transaction::dispatch(name, params, adapter),

        // Admin operations (db_*, admin_*)
        "db_open"
        | "db_stats"
        | "db_compact"
        | "db_checkpoint"
        | "admin_list_all_collections"
        | "admin_create_system_collection"
        | "admin_set_collection_flags"
        | "admin_drop_protected"
        | "admin_apikey_create"
        | "admin_apikey_list"
        | "admin_apikey_revoke"
        | "admin_apikey_delete" => {
            admin::dispatch(name, params, adapter, api_key_cache, server_info)
        }

        // Vector operations (similarity search)
        "index_create_vector"
        | "index_list_vector"
        | "index_drop_vector"
        | "vector_search"
        | "vector_search_filter" => vector::dispatch(name, params, adapter),

        // Hybrid search (RRF fusion of vector + fulltext)
        "hybrid_search" => hybrid::dispatch(name, params, adapter),

        // Embedding operations
        "embed_text"
        | "embed_batch"
        | "embed_list_models"
        | "embed_document"
        | "embed_cache_stats"
        | "embed_cache_clear" => {
            embedding::dispatch(name, params, embedding_manager, Some(adapter))
        }

        // Auto-embedding configuration
        "auto_embed_enable" | "auto_embed_disable" | "auto_embed_status" | "auto_embed_backfill" => {
            auto_embed::dispatch(name, params, adapter, embedding_manager, job_manager)
        }

        // Job management
        "embed_job_status" | "embed_job_list" | "embed_job_cancel" => {
            jobs::dispatch(name, params, job_manager)
        }

        _ => Err(McpError::invalid_params(format!("Unknown tool: {}", name))),
    }
}
