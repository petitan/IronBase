//! MCP Client - high-level API for IronBase MCP operations

use crate::logging::{redact_arguments, sanitize_for_log};
use crate::mcp::error::{McpError, McpResult};
use crate::mcp::protocol::{JsonRpcRequest, ToolCallResult};
use crate::mcp::transport::{HttpTransport, StdioTransport, Transport};
use ironbase_mcp_types::Tool;
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

/// Log how a request to the server ended: what was called, how long it took
/// and, on failure, the (sanitised) error text
fn log_outcome<T>(what: &str, tool: &str, started: Instant, result: &McpResult<T>) {
    let duration_ms = started.elapsed().as_millis() as u64;
    match result {
        Ok(_) => tracing::info!(tool, duration_ms, outcome = "ok", "{}", what),
        Err(e) => tracing::warn!(
            tool,
            duration_ms,
            outcome = "error",
            error = %sanitize_for_log(&e.to_string()),
            "{}",
            what
        ),
    }
}

/// MCP Client for IronBase operations
pub struct McpClient {
    transport: Arc<dyn Transport>,
    initialized: bool,
}

impl McpClient {
    /// Connect via HTTP transport with TLS options
    pub async fn connect_http_with_options(
        url: &str,
        api_key: Option<String>,
        insecure: bool,
    ) -> McpResult<Self> {
        let transport = HttpTransport::with_options(url, api_key, insecure)?;
        let mut client = Self {
            transport: Arc::new(transport),
            initialized: false,
        };
        client.initialize().await?;
        Ok(client)
    }

    /// Client over a ready transport (tests: no connection needed)
    #[cfg(test)]
    pub(crate) fn with_transport(transport: Arc<dyn Transport>) -> Self {
        Self {
            transport,
            initialized: true,
        }
    }

    /// Connect via stdio transport (spawns MCP server)
    pub async fn connect_stdio(server_path: &Path, db_path: &Path) -> McpResult<Self> {
        let transport = StdioTransport::new(server_path, db_path).await?;
        let mut client = Self {
            transport: Arc::new(transport),
            initialized: false,
        };
        client.initialize().await?;
        Ok(client)
    }

    /// Initialize the MCP connection
    async fn initialize(&mut self) -> McpResult<()> {
        let started = Instant::now();
        let request = JsonRpcRequest::initialize();
        let result = match self.transport.send(&request).await {
            Ok(response) if response.error.is_some() => {
                Err(McpError::invalid_response("Initialize failed"))
            }
            Ok(_) => Ok(()),
            Err(e) => Err(e),
        };
        log_outcome("MCP initialize", "initialize", started, &result);
        result?;

        self.initialized = true;
        Ok(())
    }

    /// Call an MCP tool and get the result. Every call is logged: name,
    /// duration and outcome at info level, the (redacted) arguments only at
    /// debug level - the result is never logged.
    async fn call_tool(&self, name: &str, arguments: Value) -> McpResult<Value> {
        tracing::debug!(
            tool = name,
            arguments = %sanitize_for_log(&redact_arguments(&arguments).to_string()),
            "MCP tool call started"
        );
        let started = Instant::now();
        let result = self.call_tool_unlogged(name, arguments).await;
        log_outcome("MCP tool call", name, started, &result);
        result
    }

    async fn call_tool_unlogged(&self, name: &str, arguments: Value) -> McpResult<Value> {
        if !self.initialized {
            return Err(McpError::NotInitialized);
        }

        let request = JsonRpcRequest::tool_call(name, arguments);
        let response = self.transport.send(&request).await?;

        // Check for RPC error
        if let Some(error) = response.error {
            return Err(McpError::rpc(error.code, error.message));
        }

        // Parse tool result
        let result = response
            .result
            .ok_or_else(|| McpError::invalid_response("Missing result in response"))?;

        // Parse as ToolCallResult
        let tool_result: ToolCallResult = serde_json::from_value(result)?;

        if tool_result.is_error {
            let msg = tool_result.text().unwrap_or("Unknown error");
            return Err(McpError::tool(msg));
        }

        // Parse text content as JSON
        let text = tool_result.text().unwrap_or("null");
        let value: Value = serde_json::from_str(text)?;

        Ok(value)
    }

    /// Call the tool `T` with typed arguments. The result must parse as
    /// `T::Resp`: a missing or wrong-typed field is an error, never a default.
    pub async fn call<T: Tool>(&self, args: &T::Args) -> McpResult<T::Resp> {
        let result = self.call_tool(T::NAME, serde_json::to_value(args)?).await?;
        serde_json::from_value(result)
            .map_err(|e| McpError::invalid_response(format!("{}: {}", T::NAME, e)))
    }

    /// Close the connection
    pub async fn close(&self) -> McpResult<()> {
        self.transport.close().await
    }

    // === High-level database operations ===

    /// Open or create a database file (switches the current database)
    pub async fn db_open(&self, path: &str, create: bool) -> McpResult<Value> {
        let args = serde_json::json!({
            "path": path,
            "create": create
        });
        let result = self.call_tool("db_open", args).await?;
        Ok(result)
    }

    /// Get server info (version, etc.)
    pub async fn server_info(&self) -> McpResult<Value> {
        // Try to get URL from transport and fetch /health endpoint
        if let Some(url) = self.transport.get_base_url() {
            let health_url = format!("{}/health", url.trim_end_matches("/mcp"));
            let client = reqwest::Client::new();
            if let Ok(response) = client.get(&health_url).send().await {
                if let Ok(json) = response.json::<Value>().await {
                    return Ok(json);
                }
            }
        }
        // Fallback: return empty object
        Ok(serde_json::json!({"version": "unknown"}))
    }

    /// Get list of available MCP tools
    pub async fn tools_list(&self) -> McpResult<Vec<Value>> {
        if !self.initialized {
            return Err(McpError::NotInitialized);
        }

        let request = JsonRpcRequest::tools_list();
        let response = self.transport.send(&request).await?;

        if let Some(error) = response.error {
            return Err(McpError::rpc(error.code, error.message));
        }

        let result = response
            .result
            .ok_or_else(|| McpError::invalid_response("Missing result in response"))?;

        // Extract tools array
        let tools: Vec<Value> = result
            .get("tools")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        Ok(tools)
    }

    /// Get list of available MCP prompts
    pub async fn prompts_list(&self) -> McpResult<Vec<Value>> {
        if !self.initialized {
            return Err(McpError::NotInitialized);
        }

        let request = JsonRpcRequest::new("prompts/list", None);
        let response = self.transport.send(&request).await?;

        if let Some(error) = response.error {
            return Err(McpError::rpc(error.code, error.message));
        }

        let result = response
            .result
            .ok_or_else(|| McpError::invalid_response("Missing result in response"))?;

        // Extract prompts array
        let prompts: Vec<Value> = result
            .get("prompts")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        Ok(prompts)
    }

    // === IronRhai Script operations ===

    /// List all saved scripts
    pub async fn script_list(&self) -> McpResult<Vec<Value>> {
        let result = self.call_tool("script_list", serde_json::json!({})).await?;
        // Result is {"scripts": [...]}
        let scripts: Vec<Value> = result
            .get("scripts")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        Ok(scripts)
    }

    /// Get a script by name
    pub async fn script_get(&self, name: &str) -> McpResult<Value> {
        let args = serde_json::json!({
            "name": name
        });
        let result = self.call_tool("script_get", args).await?;
        Ok(result)
    }

    /// Save a script (create or update)
    pub async fn script_save(
        &self,
        name: &str,
        code: &str,
        description: Option<&str>,
        tags: Option<&[String]>,
    ) -> McpResult<Value> {
        let mut args = serde_json::json!({
            "name": name,
            "code": code
        });
        if let Some(desc) = description {
            args["description"] = serde_json::json!(desc);
        }
        if let Some(t) = tags {
            args["tags"] = serde_json::json!(t);
        }
        let result = self.call_tool("script_save", args).await?;
        Ok(result)
    }

    /// Delete a script
    pub async fn script_delete(&self, name: &str) -> McpResult<()> {
        let args = serde_json::json!({
            "name": name
        });
        self.call_tool("script_delete", args).await?;
        Ok(())
    }

    /// Execute inline script code (not saved)
    pub async fn script_exec(&self, code: &str, params: Option<&Value>) -> McpResult<Value> {
        let mut args = serde_json::json!({
            "code": code
        });
        if let Some(p) = params {
            args["params"] = p.clone();
        }
        let result = self.call_tool("script_exec", args).await?;
        Ok(result)
    }

    /// Get script version history
    pub async fn script_history(&self, name: &str, limit: Option<usize>) -> McpResult<Vec<Value>> {
        let mut args = serde_json::json!({
            "name": name
        });
        if let Some(lim) = limit {
            args["limit"] = serde_json::json!(lim);
        }
        let result = self.call_tool("script_history", args).await?;
        // Result is {"history": [...], "count": N}
        let history: Vec<Value> = result
            .get("history")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        Ok(history)
    }

    /// Rollback script to a specific version
    pub async fn script_rollback(&self, name: &str, version: u32) -> McpResult<u32> {
        let args = serde_json::json!({
            "name": name,
            "version": version
        });
        let result = self.call_tool("script_rollback", args).await?;
        // Result is {"success": true, "name": "...", "new_version": N}
        let new_version = result
            .get("new_version")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        Ok(new_version)
    }

    // === API Key Management (admin operations) ===

    /// List all API keys (requires admin key)
    pub async fn list_api_keys(&self, admin_key: &str) -> McpResult<Vec<Value>> {
        let args = serde_json::json!({
            "admin_key": admin_key
        });
        let result = self.call_tool("admin_apikey_list", args).await?;
        // Result is {"keys": [...]}
        let keys: Vec<Value> = result
            .get("keys")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        Ok(keys)
    }

    /// Create a new API key (requires admin key)
    /// Returns the full key (only shown once)
    pub async fn create_api_key(&self, admin_key: &str, name: &str) -> McpResult<Value> {
        let args = serde_json::json!({
            "admin_key": admin_key,
            "name": name
        });
        let result = self.call_tool("admin_apikey_create", args).await?;
        Ok(result)
    }

    /// Revoke (disable) an API key by ID (requires admin key)
    pub async fn revoke_api_key(&self, admin_key: &str, id: u64) -> McpResult<bool> {
        let args = serde_json::json!({
            "admin_key": admin_key,
            "id": id
        });
        let result = self.call_tool("admin_apikey_revoke", args).await?;
        let success = result
            .get("success")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        Ok(success)
    }

    /// Delete an API key by ID (requires admin key)
    pub async fn delete_api_key(&self, admin_key: &str, id: u64) -> McpResult<bool> {
        let args = serde_json::json!({
            "admin_key": admin_key,
            "id": id
        });
        let result = self.call_tool("admin_apikey_delete", args).await?;
        let success = result
            .get("success")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        Ok(success)
    }

    // === ACL Management ===

    /// List all ACL rules
    pub async fn acl_list(&self) -> McpResult<Vec<Value>> {
        let result = self.call_tool("acl_list", serde_json::json!({})).await?;
        // Result is {"rules": [...]} or {"builtin": [...], "custom": [...]}
        if let Some(rules) = result.get("rules") {
            Ok(serde_json::from_value(rules.clone()).unwrap_or_default())
        } else {
            // Combine builtin and custom rules
            let mut all_rules: Vec<Value> = Vec::new();
            if let Some(builtin) = result.get("builtin").and_then(|v| v.as_array()) {
                all_rules.extend(builtin.clone());
            }
            if let Some(custom) = result.get("custom").and_then(|v| v.as_array()) {
                all_rules.extend(custom.clone());
            }
            Ok(all_rules)
        }
    }

    /// Get ACL for a specific collection
    pub async fn acl_get(&self, collection: &str) -> McpResult<Option<Value>> {
        let args = serde_json::json!({
            "collection": collection
        });
        let result = self.call_tool("acl_get", args).await?;
        Ok(parse_acl_get_result(result))
    }

    /// Set ACL for a collection (localhost only)
    pub async fn acl_set(&self, collection: &str, rules: &[Value]) -> McpResult<bool> {
        let args = serde_json::json!({
            "collection": collection,
            "rules": rules
        });
        let result = self.call_tool("acl_set", args).await?;
        let success = result
            .get("success")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        Ok(success)
    }

    /// Delete ACL for a collection (localhost only, reverts to default)
    pub async fn acl_delete(&self, collection: &str) -> McpResult<bool> {
        let args = serde_json::json!({
            "collection": collection
        });
        let result = self.call_tool("acl_delete", args).await?;
        let success = result
            .get("success")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        Ok(success)
    }

    // === Listener Management ===

    /// List all listeners
    pub async fn listener_list(&self) -> McpResult<Vec<Value>> {
        let result = self
            .call_tool("listener_list", serde_json::json!({}))
            .await?;
        // Result is {"listeners": [...]}
        let listeners: Vec<Value> = result
            .get("listeners")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        Ok(listeners)
    }

    /// Add new listener (localhost only)
    pub async fn listener_add(
        &self,
        id: &str,
        bind: &str,
        tls: bool,
        cert_path: Option<&str>,
        key_path: Option<&str>,
    ) -> McpResult<bool> {
        let mut args = serde_json::json!({
            "id": id,
            "bind": bind,
            "tls": tls
        });
        if let Some(cert) = cert_path {
            args["cert_path"] = serde_json::json!(cert);
        }
        if let Some(key) = key_path {
            args["key_path"] = serde_json::json!(key);
        }
        let result = self.call_tool("listener_create", args).await?;
        let success = result
            .get("success")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        Ok(success)
    }

    /// Delete listener (localhost only)
    pub async fn listener_delete(&self, id: &str) -> McpResult<bool> {
        let args = serde_json::json!({
            "id": id
        });
        let result = self.call_tool("listener_delete", args).await?;
        let success = result
            .get("success")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        Ok(success)
    }

    /// Enable listener (localhost only)
    pub async fn listener_enable(&self, id: &str) -> McpResult<bool> {
        let args = serde_json::json!({
            "id": id
        });
        let result = self.call_tool("listener_enable", args).await?;
        let success = result
            .get("success")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        Ok(success)
    }

    /// Disable listener (localhost only)
    pub async fn listener_disable(&self, id: &str) -> McpResult<bool> {
        let args = serde_json::json!({
            "id": id
        });
        let result = self.call_tool("listener_disable", args).await?;
        let success = result
            .get("success")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        Ok(success)
    }

    // === RAG operations ===

    /// Create a RAG-optimized collection with vector + fulltext indexes
    pub async fn rag_collection_create(
        &self,
        collection: &str,
        embedding_field: &str,
        text_field: &str,
        provider: &str,
        language: &str,
    ) -> McpResult<Value> {
        let args = serde_json::json!({
            "collection": collection,
            "embedding_field": embedding_field,
            "text_field": text_field,
            "provider": provider,
            "language": language
        });
        self.call_tool("rag_collection_create", args).await
    }

    /// Import a document with automatic chunking and embedding
    pub async fn rag_document_import(
        &self,
        collection: &str,
        content: &str,
        title: Option<&str>,
        chunk_size: usize,
        overlap: usize,
        mode: &str,
    ) -> McpResult<Value> {
        let mut args = serde_json::json!({
            "collection": collection,
            "content": content,
            "chunk_size": chunk_size,
            "overlap": overlap,
            "mode": mode
        });
        if let Some(t) = title {
            args["title"] = serde_json::json!(t);
        }
        self.call_tool("rag_document_import", args).await
    }

    /// Get RAG collection statistics
    pub async fn rag_collection_stats(&self, collection: &str) -> McpResult<Value> {
        let args = serde_json::json!({"collection": collection});
        self.call_tool("rag_collection_stats", args).await
    }

    // === Embedding operations ===

    /// Generate embedding for a single text
    pub async fn embed_text(&self, text: &str, provider: &str) -> McpResult<Value> {
        let args = serde_json::json!({
            "text": text,
            "provider": provider
        });
        self.call_tool("embed_text", args).await
    }

    /// List available embedding models
    pub async fn embed_list_models(&self) -> McpResult<Value> {
        self.call_tool("embed_models_list", serde_json::json!({}))
            .await
    }

    /// Get embedding cache statistics
    pub async fn embed_cache_stats(&self) -> McpResult<Value> {
        self.call_tool("embed_cache_stats", serde_json::json!({}))
            .await
    }

    /// Clear embedding cache
    pub async fn embed_cache_clear(&self) -> McpResult<Value> {
        self.call_tool("embed_cache_clear", serde_json::json!({}))
            .await
    }

    // === Auto-embed operations ===

    /// Enable auto-embedding for a collection
    pub async fn auto_embed_enable(
        &self,
        collection: &str,
        source_field: &str,
        target_field: &str,
        provider: &str,
    ) -> McpResult<Value> {
        let args = serde_json::json!({
            "collection": collection,
            "source_field": source_field,
            "target_field": target_field,
            "provider": provider
        });
        self.call_tool("auto_embed_enable", args).await
    }

    /// Disable auto-embedding for a collection
    pub async fn auto_embed_disable(&self, collection: &str) -> McpResult<Value> {
        let args = serde_json::json!({"collection": collection});
        self.call_tool("auto_embed_disable", args).await
    }

    /// Get auto-embedding status for a collection
    pub async fn auto_embed_status(&self, collection: &str) -> McpResult<Value> {
        let args = serde_json::json!({"collection": collection});
        self.call_tool("auto_embed_status", args).await
    }

    // === Job management ===

    /// List embedding jobs
    pub async fn embed_job_list(&self, active_only: bool) -> McpResult<Value> {
        let args = serde_json::json!({"active_only": active_only});
        self.call_tool("embed_job_list", args).await
    }

    /// Cancel a running job
    pub async fn embed_job_cancel(&self, job_id: &str) -> McpResult<Value> {
        let args = serde_json::json!({"job_id": job_id});
        self.call_tool("embed_job_cancel", args).await
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        // Best effort close - we can't do async in drop
        // The transport's Drop impl will handle cleanup
    }
}

/// Interpret the `acl_get` tool result.
///
/// The server returns the stored ACL document itself
/// (`{"collection": ..., "rules": [...]}`) or, when the collection has no
/// custom ACL, `{"collection": ..., "rules": null, "note": ...}`.
fn parse_acl_get_result(result: Value) -> Option<Value> {
    match result.get("rules") {
        Some(rules) if !rules.is_null() => Some(result),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_parse_acl_get_result_returns_stored_document() {
        let doc = json!({
            "_id": 1,
            "collection": "users",
            "rules": [{
                "principal": {"type": "apikey", "value": "abc"},
                "permissions": {"read": true, "write": false, "admin": false}
            }]
        });
        assert_eq!(parse_acl_get_result(doc.clone()), Some(doc));
    }

    #[test]
    fn test_parse_acl_get_result_no_custom_acl() {
        let result = json!({
            "collection": "users",
            "rules": null,
            "note": "No custom ACL for this collection. Default rules apply."
        });
        assert_eq!(parse_acl_get_result(result), None);
    }
}
