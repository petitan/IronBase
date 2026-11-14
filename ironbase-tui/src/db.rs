//! IronBase database wrapper for TUI - MCP client based
//!
//! This module provides async database operations via MCP protocol.

use crate::mcp::McpClient;
use crate::state::index::IndexEntry;
use anyhow::Result;
use ironbase_mcp_types::params::{CountParams, FindParams, IndexListParams, NoParams};
use ironbase_mcp_types::responses::{
    BTreeIndexStats, IndexListResponse, IndexStatsResponse, VectorIndexInfo,
};
use ironbase_mcp_types::tools::{CollectionList, Count, Find, IndexList, IndexStats};
use ironbase_mcp_types::Tool;
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
}

/// Wrapper around MCP client for TUI operations
pub struct DbWrapper {
    client: McpClient,
}

impl DbWrapper {
    /// Connect via HTTP transport with TLS options
    pub async fn connect_http_with_options(
        url: &str,
        api_key: Option<String>,
        insecure: bool,
    ) -> Result<Self> {
        let client = McpClient::connect_http_with_options(url, api_key, insecure).await?;
        Ok(Self { client })
    }

    /// Wrapper over a ready transport (tests: no connection needed)
    #[cfg(test)]
    pub(crate) fn with_transport(transport: std::sync::Arc<dyn crate::mcp::Transport>) -> Self {
        Self {
            client: McpClient::with_transport(transport),
        }
    }

    /// Connect via stdio transport (spawns MCP server)
    pub async fn connect_stdio(server_path: &Path, db_path: &Path) -> Result<Self> {
        let client = McpClient::connect_stdio(server_path, db_path).await?;
        Ok(Self { client })
    }

    /// Call the tool `T` with typed arguments (see [`McpClient::call`])
    pub async fn call<T: Tool>(&self, args: &T::Args) -> Result<T::Resp> {
        Ok(self.client.call::<T>(args).await?)
    }

    /// List all collections (fast - names only, no counts)
    pub async fn list_collections(&self) -> Result<Vec<CollectionInfo>> {
        let mut names = self
            .client
            .call::<CollectionList>(&NoParams {})
            .await?
            .collections;
        // The server lists collections in HashMap order: sort for a stable list
        names.sort();

        // Fast path: just names, counts will be loaded lazily on selection
        let infos = names.into_iter().map(CollectionInfo::unloaded).collect();

        Ok(infos)
    }

    /// Load collection details (count + index names) - parallel requests
    /// Returns (doc_count, index_names)
    pub async fn load_collection_details(&self, name: &str) -> Result<(usize, Vec<String>)> {
        let count_args = CountParams {
            collection: name.to_string(),
            query: serde_json::json!({}),
        };

        // Parallel requests for count and indexes
        let (count_result, indexes_result) = tokio::join!(
            self.client.call::<Count>(&count_args),
            self.index_entries(name)
        );

        let doc_count = count_result?.count as usize;
        let index_names = indexes_result?.into_iter().map(|e| e.name).collect();

        Ok((doc_count, index_names))
    }

    /// Every index of a collection with its type (`index_list` without `type`)
    pub async fn index_entries(&self, collection: &str) -> Result<Vec<IndexEntry>> {
        let args = IndexListParams {
            collection: collection.to_string(),
            index_type: None,
        };
        match self.client.call::<IndexList>(&args).await? {
            IndexListResponse::All(all) => Ok(IndexEntry::from_index_list(all)),
            other => Err(anyhow::anyhow!(
                "index_list: expected every index subtype, got {:?}",
                other
            )),
        }
    }

    /// Vector indexes of a collection (`index_list` with `type: vector`)
    pub async fn vector_indexes(&self, collection: &str) -> Result<Vec<VectorIndexInfo>> {
        let args = IndexListParams {
            collection: collection.to_string(),
            index_type: Some("vector".to_string()),
        };
        match self.client.call::<IndexList>(&args).await? {
            IndexListResponse::Vector(list) => Ok(list.indexes),
            // An empty typed list has no item to tell its subtype by
            IndexListResponse::BTree(list) if list.indexes.is_empty() => Ok(Vec::new()),
            other => Err(anyhow::anyhow!(
                "index_list: expected vector indexes, got {:?}",
                other
            )),
        }
    }

    /// Statistics of the B+ tree indexes (`index_stats`, default `type`)
    pub async fn btree_index_statistics(&self, collection: &str) -> Result<Vec<BTreeIndexStats>> {
        let args = IndexListParams {
            collection: collection.to_string(),
            index_type: None,
        };
        match self.client.call::<IndexStats>(&args).await? {
            IndexStatsResponse::BTree(list) => Ok(list.indexes),
            other => Err(anyhow::anyhow!(
                "index_stats: expected B+ tree statistics, got {:?}",
                other
            )),
        }
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
        self.client
            .tools_list()
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))
    }

    /// Get list of available MCP prompts
    pub async fn prompts_list(&self) -> Result<Vec<Value>> {
        self.client
            .prompts_list()
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))
    }

    /// Get server version from MCP server info
    pub async fn get_server_version(&self) -> Result<String> {
        let info = self
            .client
            .server_info()
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        let version = info
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        Ok(version)
    }

    /// Get collection schema (field names and types from sample)
    pub async fn infer_schema(
        &self,
        collection: &str,
        sample_size: usize,
    ) -> Result<Vec<FieldInfo>> {
        let args = FindParams {
            collection: collection.to_string(),
            query: serde_json::json!({}),
            projection: None,
            sort: None,
            limit: Some(sample_size),
            skip: None,
            include_total: false,
        };
        let docs = self.client.call::<Find>(&args).await?.documents;

        let mut fields: std::collections::HashMap<String, std::collections::HashSet<String>> =
            std::collections::HashMap::new();

        for doc in &docs {
            if let Value::Object(obj) = doc {
                collect_fields(obj, "", &mut fields);
            }
        }

        let mut result: Vec<FieldInfo> =
            fields.into_keys().map(|name| FieldInfo { name }).collect();

        result.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(result)
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
        let result = self
            .client
            .script_save(name, code, description, tags)
            .await?;
        Ok(result)
    }

    /// Delete a script
    pub async fn script_delete(&self, name: &str) -> Result<()> {
        self.client.script_delete(name).await?;
        Ok(())
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

    // === Full-text Search ===

    /// Execute fulltext search
    pub async fn fulltext_search(
        &self,
        collection: &str,
        field: &str,
        query: &str,
        limit: Option<usize>,
    ) -> Result<Vec<Value>> {
        let results = self
            .client
            .fulltext_search(collection, field, query, limit)
            .await?;
        Ok(results)
    }

    // === ACL Management ===

    /// List all ACL rules
    pub async fn acl_list(&self) -> Result<Vec<Value>> {
        let rules = self.client.acl_list().await?;
        Ok(rules)
    }

    /// Get ACL for a specific collection
    pub async fn acl_get(&self, collection: &str) -> Result<Option<Value>> {
        let acl = self.client.acl_get(collection).await?;
        Ok(acl)
    }

    /// Set ACL for a collection
    pub async fn acl_set(&self, collection: &str, rules: &[Value]) -> Result<bool> {
        let success = self.client.acl_set(collection, rules).await?;
        Ok(success)
    }

    /// Delete ACL for a collection
    pub async fn acl_delete(&self, collection: &str) -> Result<bool> {
        let success = self.client.acl_delete(collection).await?;
        Ok(success)
    }

    // === Listener Management ===

    /// List all listeners
    pub async fn listener_list(&self) -> Result<Vec<Value>> {
        let listeners = self.client.listener_list().await?;
        Ok(listeners)
    }

    /// Add a new listener
    pub async fn listener_add(
        &self,
        id: &str,
        bind: &str,
        tls: bool,
        cert_path: Option<&str>,
        key_path: Option<&str>,
    ) -> Result<bool> {
        let success = self
            .client
            .listener_add(id, bind, tls, cert_path, key_path)
            .await?;
        Ok(success)
    }

    /// Delete a listener
    pub async fn listener_delete(&self, id: &str) -> Result<bool> {
        let success = self.client.listener_delete(id).await?;
        Ok(success)
    }

    /// Enable a listener
    pub async fn listener_enable(&self, id: &str) -> Result<bool> {
        let success = self.client.listener_enable(id).await?;
        Ok(success)
    }

    /// Disable a listener
    pub async fn listener_disable(&self, id: &str) -> Result<bool> {
        let success = self.client.listener_disable(id).await?;
        Ok(success)
    }

    // === Vector Search ===

    /// Perform vector similarity search
    pub async fn vector_search(
        &self,
        collection: &str,
        field: &str,
        vector: &[f64],
        limit: usize,
    ) -> Result<Vec<Value>> {
        let results = self
            .client
            .vector_search(collection, field, vector, limit)
            .await?;
        Ok(results)
    }

    /// Perform vector similarity search with filter
    pub async fn vector_search_filter(
        &self,
        collection: &str,
        field: &str,
        vector: &[f64],
        filter: &Value,
        limit: usize,
    ) -> Result<Vec<Value>> {
        let results = self
            .client
            .vector_search_filter(collection, field, vector, filter, limit)
            .await?;
        Ok(results)
    }

    // === RAG operations ===

    pub async fn rag_collection_create(
        &self,
        collection: &str,
        embedding_field: &str,
        text_field: &str,
        provider: &str,
        language: &str,
    ) -> Result<Value> {
        let result = self
            .client
            .rag_collection_create(collection, embedding_field, text_field, provider, language)
            .await?;
        Ok(result)
    }

    pub async fn rag_document_import(
        &self,
        collection: &str,
        content: &str,
        title: Option<&str>,
        chunk_size: usize,
        overlap: usize,
        mode: &str,
    ) -> Result<Value> {
        let result = self
            .client
            .rag_document_import(collection, content, title, chunk_size, overlap, mode)
            .await?;
        Ok(result)
    }

    pub async fn hybrid_search_rag(
        &self,
        collection: &str,
        query: &str,
        limit: usize,
        search_mode: &str,
        rrf_k: f64,
    ) -> Result<Value> {
        let result = self
            .client
            .hybrid_search_rag(collection, query, limit, search_mode, rrf_k)
            .await?;
        Ok(result)
    }

    pub async fn rag_collection_stats(&self, collection: &str) -> Result<Value> {
        let result = self.client.rag_collection_stats(collection).await?;
        Ok(result)
    }

    // === Embedding operations ===

    pub async fn embed_text(&self, text: &str, provider: &str) -> Result<Value> {
        let result = self.client.embed_text(text, provider).await?;
        Ok(result)
    }

    pub async fn embed_list_models(&self) -> Result<Value> {
        let result = self.client.embed_list_models().await?;
        Ok(result)
    }

    pub async fn embed_cache_stats(&self) -> Result<Value> {
        let result = self.client.embed_cache_stats().await?;
        Ok(result)
    }

    pub async fn embed_cache_clear(&self) -> Result<Value> {
        let result = self.client.embed_cache_clear().await?;
        Ok(result)
    }

    // === Auto-embed operations ===

    pub async fn auto_embed_enable(
        &self,
        collection: &str,
        source_field: &str,
        target_field: &str,
        provider: &str,
    ) -> Result<Value> {
        let result = self
            .client
            .auto_embed_enable(collection, source_field, target_field, provider)
            .await?;
        Ok(result)
    }

    pub async fn auto_embed_disable(&self, collection: &str) -> Result<Value> {
        let result = self.client.auto_embed_disable(collection).await?;
        Ok(result)
    }

    pub async fn auto_embed_status(&self, collection: &str) -> Result<Value> {
        let result = self.client.auto_embed_status(collection).await?;
        Ok(result)
    }

    // === Job management ===

    pub async fn embed_job_list(&self, active_only: bool) -> Result<Value> {
        let result = self.client.embed_job_list(active_only).await?;
        Ok(result)
    }

    pub async fn embed_job_cancel(&self, job_id: &str) -> Result<Value> {
        let result = self.client.embed_job_cancel(job_id).await?;
        Ok(result)
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
