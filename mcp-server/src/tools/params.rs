//! Typed parameter structs for MCP tools
//!
//! These structs provide compile-time type safety and automatic validation
//! via serde deserialization, replacing manual Value parsing.

use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;

/// Default value for query fields: empty object {}
fn empty_object() -> Value {
    json!({})
}

// ============================================================================
// CRUD Tool Parameters
// ============================================================================

/// Parameters for `find` tool
#[derive(Debug, Deserialize)]
pub struct FindParams {
    /// Collection name (required)
    pub collection: String,
    /// Query filter (optional, defaults to {})
    #[serde(default = "empty_object")]
    pub query: Value,
    /// Fields to include/exclude
    pub projection: Option<Value>,
    /// Sort specification
    pub sort: Option<Value>,
    /// Maximum documents to return
    pub limit: Option<usize>,
    /// Documents to skip
    pub skip: Option<usize>,
    /// Include total count in response
    #[serde(default)]
    pub include_total: bool,
}

/// Parameters for `find_one` tool
#[derive(Debug, Deserialize)]
pub struct FindOneParams {
    pub collection: String,
    #[serde(default = "empty_object")]
    pub query: Value,
    pub projection: Option<Value>,
}

/// Parameters for `insert_one` tool
#[derive(Debug, Deserialize)]
pub struct InsertOneParams {
    pub collection: String,
    pub document: Value,
}

/// Parameters for `insert_many` tool
#[derive(Debug, Deserialize)]
pub struct InsertManyParams {
    pub collection: String,
    pub documents: Vec<Value>,
}

/// Parameters for `update_one` / `update_many` tools
#[derive(Debug, Deserialize)]
pub struct UpdateParams {
    pub collection: String,
    pub filter: Value,
    pub update: Value,
    #[serde(default)]
    pub upsert: bool,
}

/// Parameters for `delete_one` / `delete_many` tools
#[derive(Debug, Deserialize)]
pub struct DeleteParams {
    pub collection: String,
    pub filter: Value,
}

/// Parameters for `count_documents` tool
#[derive(Debug, Deserialize)]
pub struct CountParams {
    pub collection: String,
    #[serde(default = "empty_object")]
    pub query: Value,
}

/// Parameters for `distinct` tool
#[derive(Debug, Deserialize)]
pub struct DistinctParams {
    pub collection: String,
    pub field: String,
    #[serde(default = "empty_object")]
    pub query: Value,
    pub limit: Option<usize>,
}

/// Parameters for `aggregate` tool
#[derive(Debug, Deserialize)]
pub struct AggregateParams {
    pub collection: String,
    pub pipeline: Vec<Value>,
}

// ============================================================================
// Index Tool Parameters
// ============================================================================

/// Parameters for `index_create` tool
#[derive(Debug, Deserialize)]
pub struct IndexCreateParams {
    pub collection: String,
    /// Single field (for simple index)
    pub field: Option<String>,
    /// Multiple fields (for compound index)
    pub fields: Option<Vec<String>>,
    #[serde(default)]
    pub unique: bool,
    #[serde(default)]
    pub sparse: bool,
}

/// Parameters for `index_list` tool
#[derive(Debug, Deserialize)]
pub struct IndexListParams {
    pub collection: String,
}

/// Parameters for `index_drop` tool
#[derive(Debug, Deserialize)]
pub struct IndexDropParams {
    pub collection: String,
    pub index_name: String,
}

/// Parameters for `index_create_fuzzy` tool
#[derive(Debug, Deserialize)]
pub struct FuzzyIndexParams {
    pub collection: String,
    pub field: String,
    #[serde(default = "default_fuzzy_algorithm")]
    pub algorithm: String,
    #[serde(default = "default_threshold")]
    pub threshold: f64,
}

fn default_fuzzy_algorithm() -> String {
    "jaro_winkler".to_string()
}

fn default_threshold() -> f64 {
    0.8
}

/// Parameters for `index_create_fulltext` tool
#[derive(Debug, Deserialize)]
pub struct FulltextIndexParams {
    pub collection: String,
    pub field: String,
    #[serde(default = "default_language")]
    pub language: String,
    pub min_word_length: Option<usize>,
    pub accent_folding: Option<bool>,
}

fn default_language() -> String {
    "none".to_string()
}

/// Parameters for `fuzzy_search` tool
#[derive(Debug, Deserialize)]
pub struct FuzzySearchParams {
    pub collection: String,
    pub field: String,
    pub query: String,
    pub threshold: Option<f64>,
    pub algorithm: Option<String>,
    pub limit: Option<usize>,
    pub projection: Option<Value>,
}

/// Parameters for `fulltext_search` tool
#[derive(Debug, Deserialize)]
pub struct FulltextSearchParams {
    pub collection: String,
    pub field: String,
    pub query: String,
    pub limit: Option<usize>,
    pub skip: Option<usize>,
    pub min_score: Option<f64>,
    pub projection: Option<Value>,
}

/// Parameters for `explain` tool
#[derive(Debug, Deserialize)]
pub struct ExplainParams {
    pub collection: String,
    #[serde(default = "empty_object")]
    pub query: Value,
}

/// Parameters for `find_with_hint` tool
#[derive(Debug, Deserialize)]
pub struct FindWithHintParams {
    pub collection: String,
    #[serde(default = "empty_object")]
    pub query: Value,
    pub hint: String,
    pub projection: Option<Value>,
    pub sort: Option<Value>,
    pub limit: Option<usize>,
    pub skip: Option<usize>,
}

// ============================================================================
// Collection Tool Parameters
// ============================================================================

/// Parameters for `list_collections` tool (no params needed)
#[derive(Debug, Deserialize, Default)]
pub struct ListCollectionsParams {}

/// Parameters for `create_collection` tool
#[derive(Debug, Deserialize)]
pub struct CreateCollectionParams {
    pub collection: String,
}

/// Parameters for `drop_collection` tool
#[derive(Debug, Deserialize)]
pub struct DropCollectionParams {
    pub collection: String,
}

/// Parameters for `collection_stats` tool
#[derive(Debug, Deserialize)]
pub struct CollectionStatsParams {
    pub collection: String,
}

// ============================================================================
// Transaction Tool Parameters
// ============================================================================

/// Parameters for transaction tools (collection-scoped)
#[derive(Debug, Deserialize)]
pub struct TransactionParams {
    pub collection: String,
}

// ============================================================================
// Script Tool Parameters
// ============================================================================

/// Parameters for `script_exec` tool
#[derive(Debug, Deserialize)]
pub struct ScriptExecParams {
    pub script: String,
    #[serde(default)]
    pub params: HashMap<String, Value>,
    pub timeout_ms: Option<u64>,
    pub max_operations: Option<u64>,
    pub api_key: Option<String>,
}

/// Parameters for `script_register` tool
#[derive(Debug, Deserialize)]
pub struct ScriptRegisterParams {
    pub name: String,
    pub script: String,
    pub description: Option<String>,
}

/// Parameters for `script_call` tool
#[derive(Debug, Deserialize)]
pub struct ScriptCallParams {
    pub name: String,
    #[serde(default)]
    pub params: HashMap<String, Value>,
    pub timeout_ms: Option<u64>,
    pub max_operations: Option<u64>,
    pub api_key: Option<String>,
}

/// Parameters for `script_delete` / `script_get` tools
#[derive(Debug, Deserialize)]
pub struct ScriptNameParams {
    pub name: String,
}

// ============================================================================
// ACL Tool Parameters
// ============================================================================

/// Parameters for `acl_set` tool
#[derive(Debug, Deserialize)]
pub struct AclSetParams {
    pub collection: String,
    pub permission: String,
    pub allow: Option<Vec<String>>,
    pub deny: Option<Vec<String>>,
}

/// Parameters for `acl_get` tool
#[derive(Debug, Deserialize)]
pub struct AclGetParams {
    pub collection: String,
}

/// Parameters for `acl_delete` tool
#[derive(Debug, Deserialize)]
pub struct AclDeleteParams {
    pub collection: String,
    pub permission: String,
}

// ============================================================================
// Listener Tool Parameters
// ============================================================================

/// Parameters for `listener_register` tool
#[derive(Debug, Deserialize)]
pub struct ListenerRegisterParams {
    pub collection: String,
    pub event: String,
    pub script: String,
    #[serde(default)]
    pub enabled: bool,
    pub priority: Option<i32>,
}

/// Parameters for `listener_list` tool
#[derive(Debug, Deserialize)]
pub struct ListenerListParams {
    pub collection: Option<String>,
    pub event: Option<String>,
}

/// Parameters for `listener_delete` tool
#[derive(Debug, Deserialize)]
pub struct ListenerDeleteParams {
    pub id: String,
}

/// Parameters for `listener_enable` / `listener_disable` tools
#[derive(Debug, Deserialize)]
pub struct ListenerToggleParams {
    pub id: String,
}

// ============================================================================
// Admin Tool Parameters
// ============================================================================

/// Parameters for `admin_compact` tool
#[derive(Debug, Deserialize, Default)]
pub struct AdminCompactParams {
    #[serde(default)]
    pub force: bool,
}

/// Parameters for `api_key_create` tool
#[derive(Debug, Deserialize)]
pub struct ApiKeyCreateParams {
    pub name: String,
    pub admin_key: String,
    pub expires_days: Option<i64>,
}

/// Parameters for `api_key_revoke` tool
#[derive(Debug, Deserialize)]
pub struct ApiKeyRevokeParams {
    pub key: String,
    pub admin_key: String,
}

/// Parameters for `api_key_list` tool
#[derive(Debug, Deserialize)]
pub struct ApiKeyListParams {
    pub admin_key: String,
}

// ============================================================================
// Helper trait for param parsing
// ============================================================================

use crate::error::{McpError, Result};

/// Helper trait for parsing params from Value
pub trait ParseParams: Sized {
    fn parse(params: Value) -> Result<Self>;
}

impl<T: for<'de> Deserialize<'de>> ParseParams for T {
    fn parse(params: Value) -> Result<Self> {
        serde_json::from_value(params).map_err(|e| McpError::invalid_params(e.to_string()))
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_find_params_full() {
        let params = json!({
            "collection": "users",
            "query": {"age": {"$gt": 18}},
            "projection": {"name": 1},
            "sort": {"age": -1},
            "limit": 10,
            "skip": 5,
            "include_total": true
        });
        let p: FindParams = FindParams::parse(params).unwrap();
        assert_eq!(p.collection, "users");
        assert_eq!(p.limit, Some(10));
        assert!(p.include_total);
    }

    #[test]
    fn test_find_params_minimal() {
        let params = json!({"collection": "users"});
        let p: FindParams = FindParams::parse(params).unwrap();
        assert_eq!(p.collection, "users");
        assert_eq!(p.query, json!({}));
        assert_eq!(p.limit, None);
        assert!(!p.include_total);
    }

    #[test]
    fn test_find_params_missing_collection() {
        let params = json!({"query": {}});
        let result = FindParams::parse(params);
        assert!(result.is_err());
    }

    #[test]
    fn test_insert_one_params() {
        let params = json!({
            "collection": "users",
            "document": {"name": "Alice", "age": 30}
        });
        let p: InsertOneParams = InsertOneParams::parse(params).unwrap();
        assert_eq!(p.collection, "users");
        assert_eq!(p.document["name"], "Alice");
    }

    #[test]
    fn test_update_params() {
        let params = json!({
            "collection": "users",
            "filter": {"name": "Alice"},
            "update": {"$set": {"age": 31}},
            "upsert": true
        });
        let p: UpdateParams = UpdateParams::parse(params).unwrap();
        assert!(p.upsert);
    }

    #[test]
    fn test_fuzzy_index_defaults() {
        let params = json!({
            "collection": "users",
            "field": "name"
        });
        let p: FuzzyIndexParams = FuzzyIndexParams::parse(params).unwrap();
        assert_eq!(p.algorithm, "jaro_winkler");
        assert_eq!(p.threshold, 0.8);
    }

    #[test]
    fn test_script_exec_params() {
        let params = json!({
            "script": "return 42;",
            "params": {"x": 10},
            "timeout_ms": 5000
        });
        let p: ScriptExecParams = ScriptExecParams::parse(params).unwrap();
        assert_eq!(p.script, "return 42;");
        assert_eq!(p.params.get("x"), Some(&json!(10)));
    }
}
