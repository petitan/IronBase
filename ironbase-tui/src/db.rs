//! IronBase database wrapper for TUI - MCP client based
//!
//! This module provides async database operations via MCP protocol.

use crate::mcp::{McpClient, McpResult};
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

/// Document search match
#[derive(Debug, Clone)]
pub struct DocumentMatch {
    pub collection: String,
    pub doc_id: String,
    pub preview: String,
    pub matched_field: String,
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
    pub async fn connect_http(url: &str) -> Result<Self> {
        let client = McpClient::connect_http(url).await?;
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

    /// Search documents across collections
    /// Returns (collection_name, doc_id, preview, matched_field)
    pub async fn search_documents(
        &self,
        query: &str,
        collection_filter: Option<&str>,
        limit: usize,
    ) -> Result<Vec<DocumentMatch>> {
        let query_lower = query.to_lowercase();
        let mut results = Vec::new();

        // Get collections to search
        let collections: Vec<String> = if let Some(coll) = collection_filter {
            vec![coll.to_string()]
        } else {
            self.client.list_collections().await?
        };

        // Search each collection
        for coll_name in collections {
            if results.len() >= limit {
                break;
            }

            // Get documents (limit per collection to avoid slowness)
            let docs = self
                .client
                .find(&coll_name, &serde_json::json!({}), None, Some(500))
                .await?;

            for doc in docs {
                if results.len() >= limit {
                    break;
                }

                // Search in document JSON
                if let Some(matched) = search_in_value(&doc, &query_lower) {
                    let doc_id = doc
                        .get("_id")
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "?".to_string());

                    // Create preview (truncated JSON)
                    let preview = truncate_json(&doc, 100);

                    results.push(DocumentMatch {
                        collection: coll_name.clone(),
                        doc_id,
                        preview,
                        matched_field: matched,
                    });
                }
            }
        }

        Ok(results)
    }

    /// Close the connection
    pub async fn close(&self) -> Result<()> {
        self.client.close().await?;
        Ok(())
    }
}

/// Search for query string in a JSON value, returning the matched field path
fn search_in_value(value: &Value, query: &str) -> Option<String> {
    search_in_value_recursive(value, query, "")
}

fn search_in_value_recursive(value: &Value, query: &str, path: &str) -> Option<String> {
    match value {
        Value::String(s) => {
            if s.to_lowercase().contains(query) {
                Some(if path.is_empty() {
                    "(root)".to_string()
                } else {
                    path.to_string()
                })
            } else {
                None
            }
        }
        Value::Number(n) => {
            if n.to_string().contains(query) {
                Some(if path.is_empty() {
                    "(root)".to_string()
                } else {
                    path.to_string()
                })
            } else {
                None
            }
        }
        Value::Object(obj) => {
            for (key, val) in obj {
                let new_path = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{}.{}", path, key)
                };

                // Also search in key name
                if key.to_lowercase().contains(query) {
                    return Some(format!("key:{}", new_path));
                }

                if let Some(found) = search_in_value_recursive(val, query, &new_path) {
                    return Some(found);
                }
            }
            None
        }
        Value::Array(arr) => {
            for (idx, val) in arr.iter().enumerate() {
                let new_path = format!("{}[{}]", path, idx);
                if let Some(found) = search_in_value_recursive(val, query, &new_path) {
                    return Some(found);
                }
            }
            None
        }
        _ => None,
    }
}

/// Truncate JSON to a max length for preview (UTF-8 safe)
fn truncate_json(value: &Value, max_len: usize) -> String {
    let json_str = serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string());
    if json_str.chars().count() <= max_len {
        json_str
    } else {
        // Safe UTF-8 truncation at character boundary
        let truncated: String = json_str.chars().take(max_len).collect();
        format!("{}...", truncated)
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
