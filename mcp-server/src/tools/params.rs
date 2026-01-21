//! Typed parameter structs for MCP tools
//!
//! These structs provide compile-time type safety and automatic validation
//! via serde deserialization, replacing manual Value parsing.

use serde::Deserialize;
use serde_json::{json, Value};

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
    /// Accepts both "query" and "filter" for API consistency with update/delete tools
    #[serde(default = "empty_object", alias = "filter")]
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
    /// Accepts both "query" and "filter" for API consistency
    #[serde(default = "empty_object", alias = "filter")]
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
    /// Accepts both "filter" and "query" for API consistency
    #[serde(alias = "query")]
    pub filter: Value,
    pub update: Value,
    #[serde(default)]
    pub upsert: bool,
}

/// Parameters for `delete_one` / `delete_many` tools
#[derive(Debug, Deserialize)]
pub struct DeleteParams {
    pub collection: String,
    /// Accepts both "filter" and "query" for API consistency
    #[serde(alias = "query")]
    pub filter: Value,
}

/// Parameters for `count_documents` tool
#[derive(Debug, Deserialize)]
pub struct CountParams {
    pub collection: String,
    /// Accepts both "query" and "filter" for API consistency
    #[serde(default = "empty_object", alias = "filter")]
    pub query: Value,
}

/// Parameters for `distinct` tool
#[derive(Debug, Deserialize)]
pub struct DistinctParams {
    pub collection: String,
    pub field: String,
    /// Accepts both "query" and "filter" for API consistency
    #[serde(default = "empty_object", alias = "filter")]
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
    pub skip: Option<usize>,
    pub projection: Option<Value>,
    /// MongoDB-style filter applied AFTER fuzzy matching (core-level filtering)
    pub filter: Option<Value>,
    /// Enable highlight of matched value (default: false)
    #[serde(default)]
    pub highlight: bool,
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
    /// MongoDB-style filter applied AFTER TF-IDF scoring (core-level filtering)
    /// Use this to combine fulltext search with other query operators (e.g., $regex, $eq)
    /// Example: {"from.email": {"$regex": "@scania.com$"}}
    pub filter: Option<Value>,
    /// Enable highlight/snippet generation (default: false)
    #[serde(default)]
    pub highlight: bool,
    /// Characters of context around each match (default: 100)
    pub highlight_context: Option<usize>,
    /// Maximum snippets per field (default: 3)
    pub highlight_max_snippets: Option<usize>,
}

/// Parameters for `fulltext_analyze` tool (token debug)
#[derive(Debug, Deserialize)]
pub struct FulltextAnalyzeParams {
    /// Text to analyze
    pub text: String,
    /// Language for stemming/stop words ("hungarian", "english", "german", "none")
    #[serde(default = "default_language")]
    pub language: String,
    /// Enable accent folding (default: true)
    #[serde(default = "default_true")]
    pub accent_folding: bool,
    /// Minimum word length (default: 2)
    pub min_word_length: Option<usize>,
}

fn default_true() -> bool {
    true
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

/// Parameters for `collection_create` tool
#[derive(Debug, Deserialize)]
pub struct CollectionCreateParams {
    pub collection: String,
}

/// Parameters for `collection_drop` tool
#[derive(Debug, Deserialize)]
pub struct CollectionDropParams {
    pub collection: String,
}

/// Parameters for `collection_stats` tool
#[derive(Debug, Deserialize)]
pub struct CollectionStatsParams {
    pub collection: String,
}

/// Parameters for `schema_set` tool
#[derive(Debug, Deserialize)]
pub struct SchemaSetParams {
    pub collection: String,
    pub schema: Option<Value>,
}

/// Parameters for `schema_get` tool
#[derive(Debug, Deserialize)]
pub struct SchemaGetParams {
    pub collection: String,
}

// ============================================================================
// Transaction Tool Parameters
// ============================================================================

/// Parameters for `commit_transaction` / `rollback_transaction` tools
#[derive(Debug, Deserialize)]
pub struct TransactionIdParams {
    pub transaction_id: String,
}

/// Parameters for `insert_one_tx` tool
#[derive(Debug, Deserialize)]
pub struct TransactionInsertParams {
    pub transaction_id: String,
    pub collection: String,
    pub document: Value,
}

/// Parameters for `update_one_tx` tool
#[derive(Debug, Deserialize)]
pub struct TransactionUpdateParams {
    pub transaction_id: String,
    pub collection: String,
    /// Accepts both "filter" and "query" for API consistency
    #[serde(alias = "query")]
    pub filter: Value,
    pub update: Value,
}

/// Parameters for `delete_one_tx` tool
#[derive(Debug, Deserialize)]
pub struct TransactionDeleteParams {
    pub transaction_id: String,
    pub collection: String,
    /// Accepts both "filter" and "query" for API consistency
    #[serde(alias = "query")]
    pub filter: Value,
}

// ============================================================================
// Script Tool Parameters
// ============================================================================

/// Parameters for `script_save` tool
#[derive(Debug, Deserialize)]
pub struct ScriptSaveParams {
    pub name: String,
    pub code: String,
    pub description: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub dependencies: Option<Vec<String>>,
}

/// Parameters for `script_list` tool
#[derive(Debug, Deserialize, Default)]
pub struct ScriptListParams {
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub match_all: bool,
}

/// Parameters for `script_get` / `script_delete` / `script_stats` tools
#[derive(Debug, Deserialize)]
pub struct ScriptNameParams {
    pub name: String,
}

/// Parameters for `script_run` tool
#[derive(Debug, Deserialize)]
pub struct ScriptRunParams {
    pub name: String,
    pub params: Option<Value>,
    pub max_operations: Option<u64>,
}

/// Parameters for `script_exec` tool
#[derive(Debug, Deserialize)]
pub struct ScriptExecParams {
    pub code: String,
    pub params: Option<Value>,
    pub max_operations: Option<u64>,
}

/// Parameters for `script_history` tool
#[derive(Debug, Deserialize)]
pub struct ScriptHistoryParams {
    pub name: String,
    pub limit: Option<usize>,
}

/// Parameters for `script_rollback` / `script_version_get` tools
#[derive(Debug, Deserialize)]
pub struct ScriptVersionParams {
    pub name: String,
    pub version: u32,
}

/// Parameters for `script_tags_add` / `script_tags_remove` tools
#[derive(Debug, Deserialize)]
pub struct ScriptTagsParams {
    pub name: String,
    pub tags: Vec<String>,
}

// ============================================================================
// ACL Tool Parameters
// ============================================================================

/// Parameters for `acl_set` tool
#[derive(Debug, Deserialize)]
pub struct AclSetParams {
    pub collection: String,
    pub rules: Vec<Value>,
}

/// Parameters for `acl_get` / `acl_delete` tools
#[derive(Debug, Deserialize)]
pub struct AclCollectionParams {
    pub collection: String,
}

// ============================================================================
// Listener Tool Parameters (Network Listeners)
// ============================================================================

/// Parameters for `listener_get` / `listener_delete` / `listener_enable` / `listener_disable` tools
#[derive(Debug, Deserialize)]
pub struct ListenerIdParams {
    pub id: String,
}

/// Parameters for `listener_add` tool
#[derive(Debug, Deserialize)]
pub struct ListenerAddParams {
    pub id: String,
    pub bind: String,
    #[serde(default)]
    pub tls: bool,
    pub cert_path: Option<String>,
    pub key_path: Option<String>,
    pub description: Option<String>,
}

// ============================================================================
// Admin Tool Parameters
// ============================================================================

/// Parameters for `db_open` tool
#[derive(Debug, Deserialize)]
pub struct DbOpenParams {
    pub path: String,
    #[serde(default)]
    pub create: bool,
}

/// Parameters for `admin_compact` tool
#[derive(Debug, Deserialize, Default)]
pub struct AdminCompactParams {
    #[serde(default)]
    pub force: bool,
}

/// Parameters requiring only admin_key verification
#[derive(Debug, Deserialize)]
pub struct AdminKeyParams {
    pub admin_key: Option<String>,
}

/// Parameters for `admin_create_system_collection` and `admin_drop_protected` tools
#[derive(Debug, Deserialize)]
pub struct AdminCollectionParams {
    pub collection: String,
    pub admin_key: Option<String>,
}

/// Parameters for `admin_set_collection_flags` tool
#[derive(Debug, Deserialize)]
pub struct AdminFlagsParams {
    pub collection: String,
    pub is_system: Option<bool>,
    pub protected: Option<bool>,
    pub hidden: Option<bool>,
    pub admin_key: Option<String>,
}

/// Parameters for `admin_apikey_create` tool
#[derive(Debug, Deserialize)]
pub struct AdminApiKeyCreateParams {
    pub name: String,
    pub admin_key: Option<String>,
}

/// Parameters for `admin_apikey_revoke` / `admin_apikey_delete` tools
#[derive(Debug, Deserialize)]
pub struct AdminApiKeyIdParams {
    pub id: u64,
    pub admin_key: Option<String>,
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
    fn test_find_params_filter_alias() {
        // Issue #26: "filter" should work as alias for "query"
        let params = json!({
            "collection": "emails",
            "filter": {"from.email": "test@example.com"},
            "limit": 10
        });
        let p: FindParams = FindParams::parse(params).unwrap();
        assert_eq!(p.collection, "emails");
        assert_eq!(p.query["from.email"], "test@example.com");
        assert_eq!(p.limit, Some(10));
    }

    #[test]
    fn test_find_one_params_filter_alias() {
        let params = json!({
            "collection": "users",
            "filter": {"_id": 123}
        });
        let p: FindOneParams = FindOneParams::parse(params).unwrap();
        assert_eq!(p.query["_id"], 123);
    }

    #[test]
    fn test_count_params_filter_alias() {
        let params = json!({
            "collection": "orders",
            "filter": {"status": "pending"}
        });
        let p: CountParams = CountParams::parse(params).unwrap();
        assert_eq!(p.query["status"], "pending");
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
            "code": "return 42;",
            "params": {"x": 10},
            "max_operations": 5000
        });
        let p: ScriptExecParams = ScriptExecParams::parse(params).unwrap();
        assert_eq!(p.code, "return 42;");
        assert_eq!(p.params.as_ref().and_then(|v| v.get("x")), Some(&json!(10)));
        assert_eq!(p.max_operations, Some(5000));
    }
}
