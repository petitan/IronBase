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
        let transport = HttpTransport::new(url);
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

    /// Find documents in a collection
    pub async fn find(
        &self,
        collection: &str,
        query: &Value,
        skip: Option<usize>,
        limit: Option<usize>,
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
}

impl Drop for McpClient {
    fn drop(&mut self) {
        // Best effort close - we can't do async in drop
        // The transport's Drop impl will handle cleanup
    }
}
