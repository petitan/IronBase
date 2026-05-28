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
    pub async fn connect_http(url: &str, api_key: Option<String>) -> McpResult<Self> {
        Self::connect_http_with_options(url, api_key, false).await
    }

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

    /// Delete multiple documents
    pub async fn delete_many(&self, collection: &str, filter: &Value) -> McpResult<u64> {
        let args = serde_json::json!({
            "collection": collection,
            "filter": filter
        });

        let result = self.call_tool("delete_many", args).await?;
        Ok(result
            .get("deleted_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0))
    }

    /// Insert multiple documents
    pub async fn insert_many(
        &self,
        collection: &str,
        documents: &[Value],
    ) -> McpResult<Vec<Value>> {
        let args = serde_json::json!({
            "collection": collection,
            "documents": documents
        });

        let result = self.call_tool("insert_many", args).await?;
        Ok(result
            .get("inserted_ids")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default())
    }

    /// Update multiple documents
    pub async fn update_many(
        &self,
        collection: &str,
        filter: &Value,
        update: &Value,
    ) -> McpResult<(u64, u64)> {
        let args = serde_json::json!({
            "collection": collection,
            "filter": filter,
            "update": update
        });

        let result = self.call_tool("update_many", args).await?;
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

    /// Get distinct values for a field
    pub async fn distinct(
        &self,
        collection: &str,
        field: &str,
        filter: Option<&Value>,
    ) -> McpResult<Vec<Value>> {
        let mut args = serde_json::json!({
            "collection": collection,
            "field": field
        });
        if let Some(f) = filter {
            args["query"] = f.clone();
        }

        let result = self.call_tool("distinct", args).await?;
        Ok(result
            .get("values")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default())
    }

    /// Fuzzy search on a field
    pub async fn fuzzy_search(
        &self,
        collection: &str,
        field: &str,
        query: &str,
        limit: Option<usize>,
    ) -> McpResult<Vec<Value>> {
        let mut args = serde_json::json!({
            "collection": collection,
            "field": field,
            "query": query
        });
        if let Some(l) = limit {
            args["limit"] = serde_json::json!(l);
        }

        let result = self.call_tool("fuzzy_search", args).await?;
        Ok(result
            .get("results")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default())
    }

    /// Explain query plan
    pub async fn explain(&self, collection: &str, query: &Value) -> McpResult<Value> {
        let args = serde_json::json!({
            "collection": collection,
            "query": query
        });

        let result = self.call_tool("explain", args).await?;
        Ok(result
            .get("plan")
            .cloned()
            .unwrap_or(serde_json::json!(null)))
    }

    /// Run aggregation pipeline
    pub async fn aggregate(&self, collection: &str, pipeline: &Value) -> McpResult<Vec<Value>> {
        let args = serde_json::json!({
            "collection": collection,
            "pipeline": pipeline
        });

        let result = self.call_tool("aggregate", args).await?;
        // Result is {"results": [...], "count": N}
        let docs: Vec<Value> = result
            .get("results")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        Ok(docs)
    }

    /// List indexes for a collection (names only, for backward compat)
    pub async fn list_indexes(&self, collection: &str) -> McpResult<Vec<String>> {
        let entries = self.list_indexes_typed(collection).await?;
        Ok(entries.into_iter().map(|e| e.name).collect())
    }

    /// List indexes with type information (btree/fulltext/vector)
    pub async fn list_indexes_typed(
        &self,
        collection: &str,
    ) -> McpResult<Vec<crate::state::index::IndexEntry>> {
        use crate::state::index::{IndexEntry, IndexKind};

        let args = serde_json::json!({
            "collection": collection
        });

        let result = self.call_tool("index_list", args).await?;
        let mut entries = Vec::new();

        // B+ tree indexes (simple string names)
        if let Some(btree) = result.get("btree_indexes").and_then(|v| v.as_array()) {
            for idx in btree {
                if let Some(name) = idx.as_str() {
                    entries.push(IndexEntry {
                        name: name.to_string(),
                        kind: IndexKind::BTree,
                        detail: String::new(),
                    });
                }
            }
        }

        // Fulltext indexes (objects with name, field, language, etc.)
        if let Some(ft) = result.get("fulltext_indexes").and_then(|v| v.as_array()) {
            for idx in ft {
                let name = idx.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let field = idx.get("field").and_then(|v| v.as_str()).unwrap_or("");
                let lang = idx.get("language").and_then(|v| v.as_str()).unwrap_or("");
                let docs = idx
                    .get("num_documents")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                // Remove from btree list if present (server includes fts in both)
                entries.retain(|e| e.name != name);
                entries.push(IndexEntry {
                    name: name.to_string(),
                    kind: IndexKind::Fulltext,
                    detail: format!("{} ({}, {} docs)", field, lang, docs),
                });
            }
        }

        // Vector indexes (objects with name, field, dim, metric, etc.)
        if let Some(vi) = result.get("vector_indexes").and_then(|v| v.as_array()) {
            for idx in vi {
                let name = idx.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let field = idx.get("field").and_then(|v| v.as_str()).unwrap_or("");
                let dim = idx.get("dim").and_then(|v| v.as_u64()).unwrap_or(0);
                let metric = idx.get("metric").and_then(|v| v.as_str()).unwrap_or("");
                let count = idx
                    .get("vector_count")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                // Remove from btree list if present
                entries.retain(|e| e.name != name);
                entries.push(IndexEntry {
                    name: name.to_string(),
                    kind: IndexKind::Vector,
                    detail: format!("{} ({}d, {}, {} vecs)", field, dim, metric, count),
                });
            }
        }

        Ok(entries)
    }

    /// Create a single-field index
    pub async fn create_index(
        &self,
        collection: &str,
        field: &str,
        unique: bool,
        sparse: bool,
    ) -> McpResult<()> {
        let args = serde_json::json!({
            "collection": collection,
            "field": field,
            "unique": unique,
            "sparse": sparse
        });

        self.call_tool("index_create", args).await?;
        Ok(())
    }

    /// Create a compound index (multiple fields)
    pub async fn create_compound_index(
        &self,
        collection: &str,
        fields: &[String],
        unique: bool,
        sparse: bool,
    ) -> McpResult<()> {
        let args = serde_json::json!({
            "collection": collection,
            "fields": fields,
            "unique": unique,
            "sparse": sparse
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

    /// Refresh index statistics for a collection
    ///
    /// Recomputes statistics (distinct_count, histograms, MCV) for all indexes.
    /// Run this after bulk inserts for optimal query plans.
    pub async fn refresh_index_stats(&self, collection: &str) -> McpResult<()> {
        let args = serde_json::json!({
            "collection": collection
        });

        self.call_tool("index_stats_refresh", args).await?;
        Ok(())
    }

    /// Get detailed index statistics for a collection
    ///
    /// Returns statistics for each index including:
    /// - name: Index name
    /// - field: Primary field
    /// - num_keys: Total keys in the index
    /// - distinct_count: Number of unique values
    /// - has_histogram: Whether histogram data is available
    /// - has_mcv: Whether MCV data is available
    pub async fn get_index_statistics(&self, collection: &str) -> McpResult<Vec<Value>> {
        let args = serde_json::json!({
            "collection": collection
        });

        let result = self.call_tool("index_stats", args).await?;
        // Result is {"indexes": [...], "count": N}
        let indexes: Vec<Value> = result
            .get("indexes")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        Ok(indexes)
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
        // Result is {"acl": {...}} or {"acl": null}
        match result.get("acl") {
            Some(acl) if !acl.is_null() => Ok(Some(acl.clone())),
            _ => Ok(None),
        }
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

    /// Get listener by ID
    pub async fn listener_get(&self, id: &str) -> McpResult<Option<Value>> {
        let args = serde_json::json!({
            "id": id
        });
        let result = self.call_tool("listener_get", args).await?;
        // Result is {"listener": {...}} or {"listener": null}
        match result.get("listener") {
            Some(listener) if !listener.is_null() => Ok(Some(listener.clone())),
            _ => Ok(None),
        }
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
        let result = self.call_tool("listener_add", args).await?;
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

    // === Full-text Search operations ===

    /// Create a fulltext index on a field
    pub async fn create_fulltext_index(
        &self,
        collection: &str,
        field: &str,
        language: &str,
    ) -> McpResult<()> {
        let args = serde_json::json!({
            "collection": collection,
            "field": field,
            "language": language
        });
        self.call_tool("index_create_fulltext", args).await?;
        Ok(())
    }

    /// Create a fuzzy index on a field
    pub async fn create_fuzzy_index(
        &self,
        collection: &str,
        field: &str,
        algorithm: Option<&str>,
        threshold: Option<f64>,
    ) -> McpResult<()> {
        let mut args = serde_json::json!({
            "collection": collection,
            "field": field
        });
        if let Some(alg) = algorithm {
            args["algorithm"] = serde_json::json!(alg);
        }
        if let Some(thresh) = threshold {
            args["threshold"] = serde_json::json!(thresh);
        }
        self.call_tool("index_create_fuzzy", args).await?;
        Ok(())
    }

    /// Execute fulltext search
    /// Returns Vec of (document, score, matched_tokens)
    pub async fn fulltext_search(
        &self,
        collection: &str,
        field: &str,
        query: &str,
        limit: Option<usize>,
    ) -> McpResult<Vec<Value>> {
        let mut args = serde_json::json!({
            "collection": collection,
            "field": field,
            "query": query
        });
        if let Some(l) = limit {
            args["limit"] = serde_json::json!(l);
        }
        let result = self.call_tool("fulltext_search", args).await?;
        // Result is {"results": [{<doc fields>, "_score": 0.5, "_matched_tokens": [...]}]}
        // (flat shape, v1.0.501+, #68)
        let results: Vec<Value> = result
            .get("results")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        Ok(results)
    }

    // === Vector Index and Search operations ===

    /// Create a vector index on a field for similarity search
    ///
    /// # Arguments
    /// * `collection` - Collection name
    /// * `field` - Field containing embedding vectors
    /// * `dimension` - Vector dimension (must match embeddings)
    /// * `metric` - Distance metric: "cosine", "euclidean", or "dot_product"
    pub async fn create_vector_index(
        &self,
        collection: &str,
        field: &str,
        dimension: usize,
        metric: &str,
    ) -> McpResult<String> {
        let args = serde_json::json!({
            "collection": collection,
            "field": field,
            "dim": dimension,
            "metric": metric
        });
        let result = self.call_tool("index_create_vector", args).await?;
        let name = result
            .get("index_name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        Ok(name)
    }

    /// List all vector indexes on a collection
    pub async fn list_vector_indexes(&self, collection: &str) -> McpResult<Vec<Value>> {
        let args = serde_json::json!({
            "collection": collection
        });
        let result = self.call_tool("index_list_vector", args).await?;
        let indexes: Vec<Value> = result
            .get("indexes")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        Ok(indexes)
    }

    /// Drop a vector index
    pub async fn drop_vector_index(&self, collection: &str, index_name: &str) -> McpResult<()> {
        let args = serde_json::json!({
            "collection": collection,
            "index_name": index_name
        });
        self.call_tool("index_drop_vector", args).await?;
        Ok(())
    }

    /// Perform vector similarity search
    ///
    /// # Arguments
    /// * `collection` - Collection with vector index
    /// * `field` - Field with vector index
    /// * `vector` - Query embedding vector
    /// * `limit` - Maximum number of results
    ///
    /// # Returns
    /// Vec of {document, distance} pairs sorted by similarity
    pub async fn vector_search(
        &self,
        collection: &str,
        field: &str,
        vector: &[f64],
        limit: usize,
    ) -> McpResult<Vec<Value>> {
        let args = serde_json::json!({
            "collection": collection,
            "field": field,
            "vector": vector,
            "limit": limit
        });
        let result = self.call_tool("vector_search", args).await?;
        let results: Vec<Value> = result
            .get("results")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        Ok(results)
    }

    /// Perform vector similarity search with filter
    ///
    /// Hybrid search: filter is applied first, then vector search on matching documents.
    pub async fn vector_search_filter(
        &self,
        collection: &str,
        field: &str,
        vector: &[f64],
        filter: &Value,
        limit: usize,
    ) -> McpResult<Vec<Value>> {
        let args = serde_json::json!({
            "collection": collection,
            "field": field,
            "vector": vector,
            "filter": filter,
            "limit": limit
        });
        let result = self.call_tool("vector_search_filter", args).await?;
        let results: Vec<Value> = result
            .get("results")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        Ok(results)
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

    /// Hybrid search with automatic query embedding (auto-embed mode)
    pub async fn hybrid_search_rag(
        &self,
        collection: &str,
        query: &str,
        limit: usize,
        search_mode: &str,
        rrf_k: f64,
    ) -> McpResult<Value> {
        let args = serde_json::json!({
            "collection": collection,
            "query": query,
            "limit": limit,
            "search_mode": search_mode,
            "rrf_k": rrf_k,
            "rerank": true,
            "deduplicate": true
        });
        self.call_tool("hybrid_search", args).await
    }

    /// Get RAG collection statistics
    pub async fn rag_collection_stats(&self, collection: &str) -> McpResult<Value> {
        let args = serde_json::json!({"collection": collection});
        self.call_tool("rag_collection_stats", args).await
    }

    // === Hybrid search ===

    /// Hybrid search combining vector similarity and fulltext search
    #[allow(clippy::too_many_arguments)]
    pub async fn hybrid_search(
        &self,
        collection: &str,
        vector_field: &str,
        text_field: &str,
        vector: &[f64],
        query: &str,
        limit: usize,
        search_mode: &str,
        rrf_k: f64,
    ) -> McpResult<Value> {
        let args = serde_json::json!({
            "collection": collection,
            "vector_field": vector_field,
            "text_field": text_field,
            "vector": vector,
            "query": query,
            "limit": limit,
            "search_mode": search_mode,
            "rrf_k": rrf_k,
            "rerank": true,
            "deduplicate": true
        });
        self.call_tool("hybrid_search", args).await
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

    /// Generate embeddings for multiple texts
    pub async fn embed_batch(&self, texts: &[String], provider: &str) -> McpResult<Value> {
        let args = serde_json::json!({
            "texts": texts,
            "provider": provider
        });
        self.call_tool("embed_batch", args).await
    }

    /// Embed a document with chunking
    #[allow(clippy::too_many_arguments)]
    pub async fn embed_document(
        &self,
        collection: &str,
        content: &str,
        title: Option<&str>,
        chunk_size: usize,
        overlap: usize,
        mode: &str,
        provider: &str,
    ) -> McpResult<Value> {
        let mut args = serde_json::json!({
            "collection": collection,
            "content": content,
            "chunk_size": chunk_size,
            "overlap": overlap,
            "mode": mode,
            "provider": provider
        });
        if let Some(t) = title {
            args["title"] = serde_json::json!(t);
        }
        self.call_tool("embed_document", args).await
    }

    /// List available embedding models
    pub async fn embed_list_models(&self) -> McpResult<Value> {
        self.call_tool("embed_list_models", serde_json::json!({}))
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

    /// Get job status
    pub async fn embed_job_status(&self, job_id: &str) -> McpResult<Value> {
        let args = serde_json::json!({"job_id": job_id});
        self.call_tool("embed_job_status", args).await
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
