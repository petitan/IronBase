//! High-level MCP client wrapping BridgeClient

use crate::bridge::BridgeClient;
use crate::error::Result;
use serde_json::Value;

/// High-level MCP client with tool wrappers
pub struct McpClient {
    bridge: BridgeClient,
    initialized: bool,
}

impl McpClient {
    /// Create new MCP client from bridge
    pub fn new(bridge: BridgeClient) -> Self {
        Self { bridge, initialized: false }
    }

    /// Initialize the MCP session (must be called before other methods)
    pub async fn initialize(&mut self) -> Result<()> {
        if self.initialized {
            return Ok(());
        }

        let params = serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {
                "name": "gaploader",
                "version": env!("CARGO_PKG_VERSION")
            }
        });

        self.bridge.call("initialize", params).await?;
        self.initialized = true;
        Ok(())
    }

    /// Ensure initialized before any operation
    async fn ensure_initialized(&mut self) -> Result<()> {
        if !self.initialized {
            self.initialize().await?;
        }
        Ok(())
    }

    /// List all collections
    pub async fn list_collections(&mut self) -> Result<Vec<String>> {
        self.ensure_initialized().await?;
        let result = self.bridge.call_tool("collection_list", Value::Null).await?;

        let collections = result
            .get("collections")
            .and_then(|c| c.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        Ok(collections)
    }

    /// Create a new collection
    pub async fn create_collection(&mut self, name: &str) -> Result<()> {
        self.ensure_initialized().await?;
        let args = serde_json::json!({ "collection": name });
        self.bridge.call_tool("collection_create", args).await?;
        Ok(())
    }

    /// Drop a collection
    pub async fn drop_collection(&mut self, name: &str) -> Result<()> {
        self.ensure_initialized().await?;
        let args = serde_json::json!({ "collection": name });
        self.bridge.call_tool("collection_drop", args).await?;
        Ok(())
    }

    /// Insert multiple documents
    pub async fn insert_many(&mut self, collection: &str, documents: &[Value]) -> Result<Vec<Value>> {
        self.ensure_initialized().await?;
        let args = serde_json::json!({
            "collection": collection,
            "documents": documents
        });

        let result = self.bridge.call_tool("insert_many", args).await?;

        let ids = result
            .get("inserted_ids")
            .and_then(|ids| ids.as_array())
            .cloned()
            .unwrap_or_default();

        Ok(ids)
    }

    /// Find documents matching a filter
    pub async fn find(&mut self, collection: &str, filter: &Value) -> Result<Vec<Value>> {
        self.ensure_initialized().await?;
        let args = serde_json::json!({
            "collection": collection,
            "filter": filter
        });

        let result = self.bridge.call_tool("find", args).await?;

        let docs = result
            .get("documents")
            .and_then(|d| d.as_array())
            .cloned()
            .unwrap_or_default();

        Ok(docs)
    }

    /// Count documents matching a filter
    pub async fn count(&mut self, collection: &str, filter: &Value) -> Result<u64> {
        self.ensure_initialized().await?;
        let args = serde_json::json!({
            "collection": collection,
            "filter": filter
        });

        let result = self.bridge.call_tool("count_documents", args).await?;

        let count = result
            .get("count")
            .and_then(|c| c.as_u64())
            .unwrap_or(0);

        Ok(count)
    }

    /// Delete documents matching a filter
    pub async fn delete_many(&mut self, collection: &str, filter: &Value) -> Result<u64> {
        self.ensure_initialized().await?;
        let args = serde_json::json!({
            "collection": collection,
            "filter": filter
        });

        let result = self.bridge.call_tool("delete_many", args).await?;

        let deleted = result
            .get("deleted_count")
            .and_then(|c| c.as_u64())
            .unwrap_or(0);

        Ok(deleted)
    }

    /// Update documents matching a filter
    pub async fn update_many(
        &mut self,
        collection: &str,
        filter: &Value,
        update: &Value,
    ) -> Result<u64> {
        self.ensure_initialized().await?;
        let args = serde_json::json!({
            "collection": collection,
            "filter": filter,
            "update": update
        });

        let result = self.bridge.call_tool("update_many", args).await?;

        let modified = result
            .get("modified_count")
            .and_then(|c| c.as_u64())
            .unwrap_or(0);

        Ok(modified)
    }

    /// Create an index on a field
    pub async fn create_index(&mut self, collection: &str, field: &str) -> Result<()> {
        self.ensure_initialized().await?;
        let args = serde_json::json!({
            "collection": collection,
            "field": field
        });

        self.bridge.call_tool("index_create", args).await?;
        Ok(())
    }

    /// Create a vector index on a field
    pub async fn create_vector_index(
        &mut self,
        collection: &str,
        field: &str,
        dimension: usize,
        metric: &str,
    ) -> Result<()> {
        self.ensure_initialized().await?;
        let args = serde_json::json!({
            "collection": collection,
            "field": field,
            "dim": dimension,
            "metric": metric
        });

        self.bridge.call_tool("index_create_vector", args).await?;
        Ok(())
    }

    /// Embed batch of texts
    ///
    /// OOM note: Uses references to avoid cloning text content.
    pub async fn embed_batch(
        &mut self,
        texts: &[&str],
        provider: Option<&str>,
    ) -> Result<Vec<Vec<f64>>> {
        self.ensure_initialized().await?;
        let mut args = serde_json::json!({ "texts": texts });

        if let Some(p) = provider {
            args["provider"] = serde_json::json!(p);
        }

        let result = self.bridge.call_tool("embed_batch", args).await?;

        let vectors = result
            .get("vectors")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .map(|v| {
                        v.as_array()
                            .map(|inner| {
                                inner
                                    .iter()
                                    .filter_map(|n| n.as_f64())
                                    .collect::<Vec<f64>>()
                            })
                            .unwrap_or_default()
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(vectors)
    }

    /// Check if collection exists
    pub async fn collection_exists(&mut self, name: &str) -> Result<bool> {
        let collections = self.list_collections().await?;
        Ok(collections.iter().any(|c| c == name))
    }

    /// Get distinct values for a field
    pub async fn distinct(&mut self, collection: &str, field: &str) -> Result<Vec<Value>> {
        self.ensure_initialized().await?;
        // Use aggregation to get distinct values
        let pipeline = serde_json::json!([
            { "$group": { "_id": format!("${}", field) } },
            { "$project": { "value": "$_id", "_id": 0 } }
        ]);

        let args = serde_json::json!({
            "collection": collection,
            "pipeline": pipeline
        });

        let result = self.bridge.call_tool("aggregate", args).await?;

        let values = result
            .get("results")
            .and_then(|r| r.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.get("value").cloned())
                    .collect()
            })
            .unwrap_or_default();

        Ok(values)
    }

    /// Close the bridge connection
    pub async fn close(self) -> Result<()> {
        self.bridge.close().await
    }
}
