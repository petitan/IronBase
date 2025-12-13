//! IronBase database wrapper for TUI - MCP client based
//!
//! This module provides async database operations via MCP protocol.

use crate::mcp::McpClient;
use anyhow::Result;
use serde_json::Value;
use std::path::Path;

/// Collection information for display
#[derive(Debug, Clone)]
pub struct CollectionInfo {
    pub name: String,
    /// Document count (None = not yet loaded)
    pub doc_count: Option<usize>,
    /// Index count (None = not yet loaded)
    pub index_count: Option<usize>,
}

impl CollectionInfo {
    /// Create unloaded collection info (fast, no MCP calls)
    pub fn unloaded(name: String) -> Self {
        Self {
            name,
            doc_count: None,
            index_count: None,
        }
    }

    /// Check if details have been loaded
    pub fn is_loaded(&self) -> bool {
        self.doc_count.is_some()
    }

    /// Format doc_count for display ("?" if not loaded)
    pub fn doc_count_display(&self) -> String {
        match self.doc_count {
            Some(n) => n.to_string(),
            None => "?".to_string(),
        }
    }
}

/// Field information from schema inference
#[derive(Debug, Clone)]
pub struct FieldInfo {
    pub name: String,
    pub types: Vec<String>,
}

/// Wrapper around MCP client for TUI operations
pub struct DbWrapper {
    client: McpClient,
}

impl DbWrapper {
    /// Connect via HTTP transport
    pub async fn connect_http(url: &str, api_key: Option<String>) -> Result<Self> {
        let client = McpClient::connect_http(url, api_key).await?;
        Ok(Self { client })
    }

    /// Connect via stdio transport (spawns MCP server)
    pub async fn connect_stdio(server_path: &Path, db_path: &Path) -> Result<Self> {
        let client = McpClient::connect_stdio(server_path, db_path).await?;
        Ok(Self { client })
    }

    /// List all collections (fast - names only, no counts)
    pub async fn list_collections(&self) -> Result<Vec<CollectionInfo>> {
        let names = self.client.list_collections().await?;

        // Fast path: just names, counts will be loaded lazily on selection
        let infos = names.into_iter().map(CollectionInfo::unloaded).collect();

        Ok(infos)
    }

    /// Load collection details (count + index names) - parallel requests
    /// Returns (doc_count, index_names)
    pub async fn load_collection_details(&self, name: &str) -> Result<(usize, Vec<String>)> {
        // Empty query for count
        let empty_query = serde_json::json!({});

        // Parallel requests for count and indexes
        let (count_result, indexes_result) = tokio::join!(
            self.client.count_documents(name, &empty_query),
            self.client.list_indexes(name)
        );

        let doc_count = count_result? as usize;
        let index_names = indexes_result?;

        Ok((doc_count, index_names))
    }

    /// Get documents from a collection with pagination
    pub async fn get_documents(
        &self,
        collection: &str,
        skip: usize,
        limit: usize,
    ) -> Result<Vec<Value>> {
        let docs = self
            .client
            .find(collection, &serde_json::json!({}), Some(skip), Some(limit))
            .await?;
        Ok(docs)
    }

    /// Count documents in a collection
    pub async fn count_documents(&self, collection: &str) -> Result<usize> {
        let count = self
            .client
            .count_documents(collection, &serde_json::json!({}))
            .await?;
        Ok(count as usize)
    }

    /// Get database statistics
    pub async fn db_stats(&self) -> Result<Value> {
        self.client.db_stats().await.map_err(|e| anyhow::anyhow!("{}", e))
    }

    /// Open or create a database file (switches the current database)
    pub async fn db_open(&self, path: &str, create: bool) -> Result<()> {
        self.client
            .db_open(path, create)
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        Ok(())
    }

    /// Get list of available MCP tools
    pub async fn tools_list(&self) -> Result<Vec<Value>> {
        self.client.tools_list().await.map_err(|e| anyhow::anyhow!("{}", e))
    }

    /// Get list of available MCP prompts
    pub async fn prompts_list(&self) -> Result<Vec<Value>> {
        self.client.prompts_list().await.map_err(|e| anyhow::anyhow!("{}", e))
    }

    /// Get server version from MCP server info
    pub async fn get_server_version(&self) -> Result<String> {
        let info = self.client.server_info().await.map_err(|e| anyhow::anyhow!("{}", e))?;
        let version = info.get("version")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        Ok(version)
    }

    /// Count documents matching a query
    pub async fn count_with_query(&self, collection: &str, query: &Value) -> Result<usize> {
        let count = self.client.count_documents(collection, query).await?;
        Ok(count as usize)
    }

    /// Execute a query on a collection
    pub async fn find(&self, collection: &str, query: &Value) -> Result<Vec<Value>> {
        let docs = self.client.find(collection, query, None, None).await?;
        Ok(docs)
    }

    /// Execute a query with options (skip, limit)
    pub async fn find_with_options(
        &self,
        collection: &str,
        query: &Value,
        skip: usize,
        limit: usize,
    ) -> Result<Vec<Value>> {
        let docs = self
            .client
            .find(collection, query, Some(skip), Some(limit))
            .await?;
        Ok(docs)
    }

    /// Execute a query with sort
    pub async fn find_with_sort(
        &self,
        collection: &str,
        query: &Value,
        skip: usize,
        limit: usize,
        sort: Option<&Value>,
    ) -> Result<Vec<Value>> {
        let docs = self
            .client
            .find_with_sort(collection, query, Some(skip), Some(limit), sort)
            .await?;
        Ok(docs)
    }

    /// Find one document
    pub async fn find_one(&self, collection: &str, query: &Value) -> Result<Option<Value>> {
        let doc = self.client.find_one(collection, query).await?;
        Ok(doc)
    }

    /// Get collection schema (field names and types from sample)
    pub async fn infer_schema(
        &self,
        collection: &str,
        sample_size: usize,
    ) -> Result<Vec<FieldInfo>> {
        let docs = self
            .client
            .find(collection, &serde_json::json!({}), None, Some(sample_size))
            .await?;

        let mut fields: std::collections::HashMap<String, std::collections::HashSet<String>> =
            std::collections::HashMap::new();

        for doc in &docs {
            if let Value::Object(obj) = doc {
                collect_fields(obj, "", &mut fields);
            }
        }

        let mut result: Vec<FieldInfo> = fields
            .into_iter()
            .map(|(name, types)| {
                let types_vec: Vec<String> = types.into_iter().collect();
                FieldInfo {
                    name,
                    types: types_vec,
                }
            })
            .collect();

        result.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(result)
    }

    /// Get indexes for a collection
    pub async fn list_indexes(&self, collection: &str) -> Result<Vec<String>> {
        let indexes = self.client.list_indexes(collection).await?;
        Ok(indexes)
    }

    /// Get total document count across all loaded collections
    pub async fn total_documents(&self) -> Result<usize> {
        let collections = self.list_collections().await?;
        Ok(collections.iter().filter_map(|c| c.doc_count).sum())
    }

    /// Create a new index
    pub async fn create_index(&self, collection: &str, field: &str, unique: bool) -> Result<()> {
        self.client.create_index(collection, field, unique).await?;
        Ok(())
    }

    /// Drop an index
    pub async fn drop_index(&self, collection: &str, name: &str) -> Result<()> {
        self.client.drop_index(collection, name).await?;
        Ok(())
    }

    /// Run aggregation pipeline
    pub async fn aggregate(&self, collection: &str, pipeline: &Value) -> Result<Vec<Value>> {
        let docs = self.client.aggregate(collection, pipeline).await?;
        Ok(docs)
    }

    /// Delete a document by _id
    pub async fn delete_document(&self, collection: &str, doc_id: &Value) -> Result<usize> {
        let query = serde_json::json!({"_id": doc_id});
        let deleted = self.client.delete_one(collection, &query).await?;
        Ok(deleted as usize)
    }

    /// Insert a new document
    pub async fn insert_document(&self, collection: &str, doc: &Value) -> Result<Value> {
        let inserted_id = self.client.insert_one(collection, doc).await?;
        Ok(inserted_id)
    }

    /// Update a document
    pub async fn update_document(
        &self,
        collection: &str,
        doc_id: &Value,
        update: &Value,
    ) -> Result<usize> {
        let query = serde_json::json!({"_id": doc_id});
        let (matched, _modified) = self.client.update_one(collection, &query, update).await?;
        Ok(matched as usize)
    }

    /// Search collections by name (case-insensitive substring match)
    pub async fn search_collections(&self, query: &str) -> Result<Vec<CollectionInfo>> {
        let query_lower = query.to_lowercase();
        let all_collections = self.list_collections().await?;

        let filtered: Vec<CollectionInfo> = all_collections
            .into_iter()
            .filter(|c| c.name.to_lowercase().contains(&query_lower))
            .collect();

        Ok(filtered)
    }

    /// Create a new empty collection
    pub async fn create_collection(&self, name: &str) -> Result<()> {
        self.client.create_collection(name).await?;
        Ok(())
    }

    /// Drop a collection
    pub async fn drop_collection(&self, name: &str) -> Result<()> {
        self.client.drop_collection(name).await?;
        Ok(())
    }

    /// Close the connection
    pub async fn close(&self) -> Result<()> {
        self.client.close().await?;
        Ok(())
    }

    // === IronRhai Script operations ===

    /// List all saved scripts
    pub async fn script_list(&self) -> Result<Vec<Value>> {
        let scripts = self.client.script_list().await?;
        Ok(scripts)
    }

    /// Get a script by name
    pub async fn script_get(&self, name: &str) -> Result<Value> {
        let script = self.client.script_get(name).await?;
        Ok(script)
    }

    /// Save a script (create or update)
    pub async fn script_save(
        &self,
        name: &str,
        code: &str,
        description: Option<&str>,
        tags: Option<&[String]>,
    ) -> Result<Value> {
        let result = self.client.script_save(name, code, description, tags).await?;
        Ok(result)
    }

    /// Delete a script
    pub async fn script_delete(&self, name: &str) -> Result<()> {
        self.client.script_delete(name).await?;
        Ok(())
    }

    /// Run a saved script
    pub async fn script_run(&self, name: &str, params: Option<&Value>) -> Result<Value> {
        let result = self.client.script_run(name, params).await?;
        Ok(result)
    }

    /// Execute inline script code (not saved)
    pub async fn script_exec(&self, code: &str, params: Option<&Value>) -> Result<Value> {
        let result = self.client.script_exec(code, params).await?;
        Ok(result)
    }

    /// Get script version history
    pub async fn script_history(&self, name: &str, limit: Option<usize>) -> Result<Vec<Value>> {
        let history = self.client.script_history(name, limit).await?;
        Ok(history)
    }

    /// Rollback script to a specific version
    pub async fn script_rollback(&self, name: &str, version: u32) -> Result<u32> {
        let new_version = self.client.script_rollback(name, version).await?;
        Ok(new_version)
    }

    // === API Key Management ===

    /// List all API keys (requires admin key)
    pub async fn list_api_keys(&self, admin_key: &str) -> Result<Vec<Value>> {
        let keys = self.client.list_api_keys(admin_key).await?;
        Ok(keys)
    }

    /// Create a new API key (requires admin key)
    pub async fn create_api_key(&self, admin_key: &str, name: &str) -> Result<Value> {
        let result = self.client.create_api_key(admin_key, name).await?;
        Ok(result)
    }

    /// Revoke (disable) an API key by ID (requires admin key)
    pub async fn revoke_api_key(&self, admin_key: &str, id: u64) -> Result<bool> {
        let success = self.client.revoke_api_key(admin_key, id).await?;
        Ok(success)
    }

    /// Delete an API key by ID (requires admin key)
    pub async fn delete_api_key(&self, admin_key: &str, id: u64) -> Result<bool> {
        let success = self.client.delete_api_key(admin_key, id).await?;
        Ok(success)
    }
}

/// Recursively collect field names and types
fn collect_fields(
    obj: &serde_json::Map<String, Value>,
    prefix: &str,
    fields: &mut std::collections::HashMap<String, std::collections::HashSet<String>>,
) {
    for (key, value) in obj {
        let full_name = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{}.{}", prefix, key)
        };

        let type_name = match value {
            Value::Null => "null",
            Value::Bool(_) => "bool",
            Value::Number(_) => "number",
            Value::String(_) => "string",
            Value::Array(_) => "array",
            Value::Object(_) => "object",
        };

        fields
            .entry(full_name.clone())
            .or_default()
            .insert(type_name.to_string());

        // Recurse into nested objects
        if let Value::Object(nested) = value {
            collect_fields(nested, &full_name, fields);
        }
    }
}
