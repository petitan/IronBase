//! MCP Client - high-level API for IronBase MCP operations

use crate::mcp::error::{McpError, McpResult};
use crate::mcp::protocol::{JsonRpcRequest, JsonRpcResponse, ToolCallResult};
use crate::mcp::transport::{HttpTransport, StdioTransport, Transport};
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;

/// MCP Client for IronBase operations
pub struct McpClient {
    transport: Arc<dyn Transport>,
    initialized: bool,
}

impl McpClient {
    /// Connect via HTTP transport
    pub async fn connect_http(url: &str) -> McpResult<Self> {
        let transport = HttpTransport::new(url)?;
        let mut client = Self {
            transport: Arc::new(transport),
            initialized: false,
        };
        client.initialize().await?;
        Ok(client)
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
        let request = JsonRpcRequest::initialize();
        let response = self.transport.send(&request).await?;

        if response.error.is_some() {
            return Err(McpError::invalid_response("Initialize failed"));
        }

        self.initialized = true;
        Ok(())
    }

    /// Call an MCP tool and get the result
    async fn call_tool(&self, name: &str, arguments: Value) -> McpResult<Value> {
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

    /// Close the connection
    pub async fn close(&self) -> McpResult<()> {
        self.transport.close().await
    }

    // === High-level database operations ===

    /// List all collections
    pub async fn list_collections(&self) -> McpResult<Vec<String>> {
        let result = self
            .call_tool("collection_list", serde_json::json!({}))
            .await?;

        // Result is {"collections": ["name1", "name2", ...]}
        let names: Vec<String> = result
            .get("collections")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        Ok(names)
    }

    /// Create a new collection
    pub async fn create_collection(&self, name: &str) -> McpResult<()> {
        let args = serde_json::json!({
            "collection": name
        });
        self.call_tool("collection_create", args).await?;
        Ok(())
    }

    /// Drop a collection
    pub async fn drop_collection(&self, name: &str) -> McpResult<()> {
        let args = serde_json::json!({
            "collection": name
        });
        self.call_tool("collection_drop", args).await?;
        Ok(())
    }

    /// Find documents in a collection
    pub async fn find(
        &self,
        collection: &str,
        query: &Value,
        skip: Option<usize>,
        limit: Option<usize>,
    ) -> McpResult<Vec<Value>> {
        self.find_with_sort(collection, query, skip, limit, None)
            .await
    }

    /// Find documents with optional sort
    pub async fn find_with_sort(
        &self,
        collection: &str,
        query: &Value,
        skip: Option<usize>,
        limit: Option<usize>,
        sort: Option<&Value>,
    ) -> McpResult<Vec<Value>> {
        let mut args = serde_json::json!({
            "collection": collection,
            "query": query
        });

        if let Some(s) = skip {
            args["skip"] = serde_json::json!(s);
        }
        if let Some(l) = limit {
            args["limit"] = serde_json::json!(l);
        }
        if let Some(sort_obj) = sort {
            args["sort"] = sort_obj.clone();
        }

        let result = self.call_tool("find", args).await?;
        // Result is {"count": N, "documents": [...]}
        let docs: Vec<Value> = result
            .get("documents")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        Ok(docs)
    }

    /// Find one document
    pub async fn find_one(&self, collection: &str, query: &Value) -> McpResult<Option<Value>> {
        let args = serde_json::json!({
            "collection": collection,
            "query": query
        });

        let result = self.call_tool("find_one", args).await?;

        // Result is {"document": {...}} or {"document": null}
        match result.get("document") {
            Some(doc) if !doc.is_null() => Ok(Some(doc.clone())),
            _ => Ok(None),
        }
    }

    /// Count documents matching a query
    pub async fn count_documents(&self, collection: &str, query: &Value) -> McpResult<u64> {
        let args = serde_json::json!({
            "collection": collection,
            "query": query
        });

        let result = self.call_tool("count_documents", args).await?;
        // Result is {"count": 123}
        let count = result.get("count").and_then(|v| v.as_u64()).unwrap_or(0);
        Ok(count)
    }

    /// Insert one document
    pub async fn insert_one(&self, collection: &str, document: &Value) -> McpResult<Value> {
        let args = serde_json::json!({
            "collection": collection,
            "document": document
        });

        let result = self.call_tool("insert_one", args).await?;
        // Result contains inserted_id
        Ok(result.get("inserted_id").cloned().unwrap_or(Value::Null))
    }

    /// Update one document
    pub async fn update_one(
        &self,
        collection: &str,
        query: &Value,
        update: &Value,
    ) -> McpResult<(u64, u64)> {
        let args = serde_json::json!({
            "collection": collection,
            "filter": query,
            "update": update
        });

        let result = self.call_tool("update_one", args).await?;

        let matched = result
            .get("matched_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let modified = result
            .get("modified_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        Ok((matched, modified))
    }

    /// Delete one document
    pub async fn delete_one(&self, collection: &str, query: &Value) -> McpResult<u64> {
        let args = serde_json::json!({
            "collection": collection,
            "filter": query
        });

        let result = self.call_tool("delete_one", args).await?;

        let count = result
            .get("deleted_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        Ok(count)
    }

    /// Run aggregation pipeline
    pub async fn aggregate(&self, collection: &str, pipeline: &Value) -> McpResult<Vec<Value>> {
        let args = serde_json::json!({
            "collection": collection,
            "pipeline": pipeline
        });

        let result = self.call_tool("aggregate", args).await?;
        let docs: Vec<Value> = serde_json::from_value(result)?;
        Ok(docs)
    }

    /// List indexes for a collection
    pub async fn list_indexes(&self, collection: &str) -> McpResult<Vec<String>> {
        let args = serde_json::json!({
            "collection": collection
        });

        let result = self.call_tool("index_list", args).await?;
        // Result is {"indexes": ["name1", "name2", ...]}
        let indexes: Vec<String> = result
            .get("indexes")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        Ok(indexes)
    }

    /// Create an index
    pub async fn create_index(&self, collection: &str, field: &str, unique: bool) -> McpResult<()> {
        let args = serde_json::json!({
            "collection": collection,
            "field": field,
            "unique": unique
        });

        self.call_tool("index_create", args).await?;
        Ok(())
    }

    /// Drop an index
    pub async fn drop_index(&self, collection: &str, index_name: &str) -> McpResult<()> {
        let args = serde_json::json!({
            "collection": collection,
            "index_name": index_name
        });

        self.call_tool("index_drop", args).await?;
        Ok(())
    }

    /// Get database statistics
    pub async fn db_stats(&self) -> McpResult<Value> {
        let result = self.call_tool("db_stats", serde_json::json!({})).await?;
        Ok(result)
    }

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

    /// Run a saved script
    pub async fn script_run(&self, name: &str, params: Option<&Value>) -> McpResult<Value> {
        let mut args = serde_json::json!({
            "name": name
        });
        if let Some(p) = params {
            args["params"] = p.clone();
        }
        let result = self.call_tool("script_run", args).await?;
        Ok(result)
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
        let new_version = result.get("new_version").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        Ok(new_version)
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        // Best effort close - we can't do async in drop
        // The transport's Drop impl will handle cleanup
    }
}
