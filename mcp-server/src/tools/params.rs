//! Typed parameter structs for MCP tools
//!
//! These structs provide compile-time type safety and automatic validation
//! via serde deserialization, replacing manual Value parsing.

use serde::Deserialize;
use serde_json::{json, Value};

use super::defaults::{
    default_chunk_mode, default_chunk_overlap, default_chunk_size, default_embedding_provider,
};

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

/// Parameters for `update_one` tool
#[derive(Debug, Deserialize)]
pub struct UpdateOneParams {
    pub collection: String,
    /// Accepts both "filter" and "query" for API consistency
    #[serde(alias = "query")]
    pub filter: Value,
    pub update: Value,
    /// Upsert: insert if no document matches (only for update_one)
    #[serde(default)]
    pub upsert: bool,
}

/// Parameters for `update_many` tool
/// Note: upsert is NOT supported for update_many (MongoDB behavior)
#[derive(Debug, Deserialize)]
pub struct UpdateManyParams {
    pub collection: String,
    /// Accepts both "filter" and "query" for API consistency
    #[serde(alias = "query")]
    pub filter: Value,
    pub update: Value,
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

// Gaploader-compatible default content field
fn default_content_field() -> String {
    "content".to_string()
}

/// Parameters for `fulltext_search` tool
///
/// Field default matches gaploader schema: "content"
#[derive(Debug, Deserialize)]
pub struct FulltextSearchParams {
    pub collection: String,
    /// Field with fulltext index (default: "content" - gaploader compatible)
    #[serde(default = "default_content_field")]
    pub field: String,
    /// Multiple fields to search across (overrides `field` if provided)
    pub fields: Option<Vec<String>>,
    pub query: String,
    pub limit: Option<usize>,
    pub skip: Option<usize>,
    pub min_score: Option<f64>,
    pub projection: Option<Value>,
    /// Search mode: "or" (default) = any word matches, "and" = ALL words must match
    pub mode: Option<String>,
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
// Hybrid Search Parameters
// ============================================================================

// Gaploader-compatible default field names (convention over configuration)
fn default_vector_field() -> String {
    "embedding".to_string()
}
fn default_text_field() -> String {
    "content".to_string()
}

/// Parameters for `hybrid_search` tool (RRF-based fusion with preprocessing, reranking, dedup)
///
/// Field defaults match gaploader schema for seamless integration:
/// - vector_field: "embedding" (gaploader stores embeddings here)
/// - text_field: "content" (gaploader stores chunk text here)
#[derive(Debug, Deserialize)]
pub struct HybridSearchParams {
    pub collection: String,
    /// Field with vector index (default: "embedding" - gaploader compatible)
    #[serde(default = "default_vector_field")]
    pub vector_field: String,
    /// Field with fulltext index (default: "content" - gaploader compatible)
    #[serde(default = "default_text_field")]
    pub text_field: String,
    /// Query embedding vector
    pub vector: Vec<f64>,
    /// Text query for fulltext search
    pub query: String,
    #[serde(default = "default_hybrid_limit")]
    pub limit: usize,
    #[serde(default = "default_vector_weight")]
    pub vector_weight: f64,
    #[serde(default = "default_fulltext_weight")]
    pub fulltext_weight: f64,
    /// MongoDB projection (optional)
    pub projection: Option<Value>,

    // ========== v2 parameters (reranking, deduplication) ==========
    /// DEPRECATED: Language parameter is ignored for NLP consistency.
    /// Vector search uses original query (client-embedded), fulltext uses Snowball stemmer.
    #[serde(default)]
    pub language: Option<String>,

    /// Enable reranking after RRF fusion (default: true)
    /// Applies exact phrase match (1.3x), keyword density (1.0-1.1x), length penalty (0.8x)
    #[serde(default = "default_rerank")]
    pub rerank: bool,

    /// Enable MMR diversity reranking (default: true)
    /// Uses embedding cosine similarity to select diverse results
    #[serde(default = "default_deduplicate")]
    pub deduplicate: bool,

    /// MMR lambda parameter for diversity vs relevance trade-off (default: 0.5)
    /// 1.0 = pure relevance, 0.0 = pure diversity
    #[serde(default = "default_mmr_lambda")]
    pub mmr_lambda: f64,
    /// MongoDB-style filter applied BEFORE both vector and fulltext search
    pub filter: Option<Value>,

    /// RRF K constant — lower = wider score spread (default: 60)
    #[serde(default = "default_rrf_k")]
    pub rrf_k: f64,

    /// Optional title field for title match boost in reranking (up to 1.5x)
    pub title_field: Option<String>,
}

fn default_hybrid_limit() -> usize {
    10
}
fn default_vector_weight() -> f64 {
    0.5
}
fn default_fulltext_weight() -> f64 {
    0.5
}
fn default_rerank() -> bool {
    true
}
fn default_deduplicate() -> bool {
    true
}
fn default_mmr_lambda() -> f64 {
    0.5
}
fn default_rrf_k() -> f64 {
    20.0
}

// ============================================================================
// RAG Tool Parameters
// ============================================================================

fn default_rag_language() -> String {
    "none".to_string()
}

/// Parameters for `rag_collection_create` tool
#[derive(Debug, Deserialize)]
pub struct RagCollectionCreateParams {
    pub collection: String,
    #[serde(default = "default_vector_field")]
    pub embedding_field: String,
    #[serde(default = "default_text_field")]
    pub text_field: String,
    #[serde(default = "default_embedding_provider")]
    pub provider: String,
    #[serde(default = "default_rag_language")]
    pub language: String,
}

/// Parameters for `rag_document_import` tool
#[derive(Debug, Deserialize)]
pub struct RagDocumentImportParams {
    pub collection: String,
    pub content: String,
    pub title: Option<String>,
    pub metadata: Option<Value>,
    pub doc_id: Option<String>,
    #[serde(default = "default_chunk_size")]
    pub chunk_size: usize,
    #[serde(default = "default_chunk_overlap")]
    pub overlap: usize,
    #[serde(default = "default_chunk_mode")]
    pub mode: String,
    pub provider: Option<String>,
}

/// Parameters for `rag_search` tool - THE KEY TOOL
/// Unlike hybrid_search, this takes a TEXT query and embeds it automatically
#[derive(Debug, Deserialize)]
pub struct RagSearchParams {
    pub collection: String,
    /// Natural language query (auto-embedded)
    pub query: String,
    #[serde(default = "default_hybrid_limit")]
    pub limit: usize,
    #[serde(default = "default_vector_weight")]
    pub vector_weight: f64,
    #[serde(default = "default_fulltext_weight")]
    pub fulltext_weight: f64,
    #[serde(default = "default_rerank")]
    pub rerank: bool,
    #[serde(default = "default_deduplicate")]
    pub deduplicate: bool,
    /// MMR lambda parameter for diversity vs relevance trade-off (default: 0.5)
    /// 1.0 = pure relevance, 0.0 = pure diversity
    #[serde(default = "default_mmr_lambda")]
    pub mmr_lambda: f64,
    pub projection: Option<Value>,
    pub provider: Option<String>,
    /// MongoDB-style filter applied BEFORE both vector and fulltext search
    pub filter: Option<Value>,

    /// RRF K constant — lower = wider score spread (default: 60)
    #[serde(default = "default_rrf_k")]
    pub rrf_k: f64,

    /// Optional title field for title match boost in reranking (up to 1.5x)
    pub title_field: Option<String>,
}

/// Parameters for `rag_collection_stats` tool
#[derive(Debug, Deserialize)]
pub struct RagCollectionStatsParams {
    pub collection: String,
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
    fn test_update_one_params() {
        let params = json!({
            "collection": "users",
            "filter": {"name": "Alice"},
            "update": {"$set": {"age": 31}},
            "upsert": true
        });
        let p: UpdateOneParams = UpdateOneParams::parse(params).unwrap();
        assert!(p.upsert);
    }

    #[test]
    fn test_update_many_params() {
        let params = json!({
            "collection": "users",
            "filter": {"status": "inactive"},
            "update": {"$set": {"archived": true}}
        });
        let p: UpdateManyParams = UpdateManyParams::parse(params).unwrap();
        assert_eq!(p.collection, "users");
        assert_eq!(p.filter["status"], "inactive");
    }

    #[test]
    fn test_update_many_no_upsert_field() {
        // Verify UpdateManyParams doesn't have upsert field
        let params = json!({
            "collection": "users",
            "filter": {},
            "update": {"$set": {"x": 1}},
            "upsert": true  // This field should be ignored
        });
        let p: UpdateManyParams = UpdateManyParams::parse(params).unwrap();
        // If we could access upsert, the test would fail at compile time
        assert_eq!(p.collection, "users");
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

    #[test]
    fn test_fulltext_search_params_defaults() {
        let params = json!({
            "collection": "articles",
            "query": "rust programming"
        });
        let p: FulltextSearchParams = FulltextSearchParams::parse(params).unwrap();
        assert_eq!(p.collection, "articles");
        assert_eq!(p.field, "content"); // default
        assert!(p.fields.is_none());
        assert_eq!(p.query, "rust programming");
        assert!(p.limit.is_none());
        assert!(p.skip.is_none());
        assert!(p.min_score.is_none());
        assert!(p.mode.is_none());
        assert!(p.filter.is_none());
        assert!(!p.highlight);
    }

    #[test]
    fn test_fulltext_search_params_single_field() {
        let params = json!({
            "collection": "articles",
            "field": "title",
            "query": "rust"
        });
        let p: FulltextSearchParams = FulltextSearchParams::parse(params).unwrap();
        assert_eq!(p.field, "title");
        assert!(p.fields.is_none());
    }

    #[test]
    fn test_fulltext_search_params_multi_fields() {
        let params = json!({
            "collection": "articles",
            "fields": ["title", "body"],
            "query": "rust"
        });
        let p: FulltextSearchParams = FulltextSearchParams::parse(params).unwrap();
        let fields = p.fields.unwrap();
        assert_eq!(fields, vec!["title", "body"]);
    }

    #[test]
    fn test_fulltext_search_params_fields_overrides_field() {
        // When both field and fields are provided, fields should be present
        let params = json!({
            "collection": "articles",
            "field": "content",
            "fields": ["title", "body"],
            "query": "rust"
        });
        let p: FulltextSearchParams = FulltextSearchParams::parse(params).unwrap();
        assert!(p.fields.is_some());
        assert_eq!(p.fields.unwrap(), vec!["title", "body"]);
    }

    #[test]
    fn test_fulltext_search_params_with_all_options() {
        let params = json!({
            "collection": "articles",
            "fields": ["title", "content"],
            "query": "rust python",
            "mode": "and",
            "limit": 5,
            "skip": 2,
            "min_score": 0.5,
            "filter": {"status": "published"},
            "highlight": true,
            "highlight_context": 200,
            "highlight_max_snippets": 5
        });
        let p: FulltextSearchParams = FulltextSearchParams::parse(params).unwrap();
        assert_eq!(p.fields.unwrap(), vec!["title", "content"]);
        assert_eq!(p.mode.as_deref(), Some("and"));
        assert_eq!(p.limit, Some(5));
        assert_eq!(p.skip, Some(2));
        assert_eq!(p.min_score, Some(0.5));
        assert!(p.filter.is_some());
        assert!(p.highlight);
        assert_eq!(p.highlight_context, Some(200));
        assert_eq!(p.highlight_max_snippets, Some(5));
    }

    #[test]
    fn test_fulltext_search_params_empty_fields_array() {
        let params = json!({
            "collection": "articles",
            "fields": [],
            "query": "rust"
        });
        let p: FulltextSearchParams = FulltextSearchParams::parse(params).unwrap();
        let fields = p.fields.unwrap();
        assert!(fields.is_empty());
    }
}
