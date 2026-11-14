//! Typed argument structs for the MCP tools
//!
//! One definition of what every tool accepts, shared by the server (which
//! deserializes them) and by clients (which serialize them). The wire format is
//! the server's: field names, aliases and defaults are exactly what the tools
//! have always accepted, and every struct round-trips —
//! `from_value(to_value(x))` equals `x`, with `None` fields omitted.

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{json, Value};

use crate::defaults::{
    default_chunk_mode, default_chunk_overlap, default_chunk_size, default_embedding_field,
    default_vector_limit, DEFAULT_MAX_VECTORS, DEFAULT_RRF_K,
};

/// Default value for query fields: empty object {}
fn empty_object() -> Value {
    json!({})
}

/// Deserialize an optional filter, normalizing empty `{}` to `None`.
///
/// MongoDB semantics: `{}` = "match all" = no filter.
/// Sending an empty filter to vector_search_with_filter or fulltext post-filter
/// triggers a full collection scan (69K+ docs) for no benefit.
pub fn deserialize_nonempty_filter<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<Value>, D::Error>
where
    D: Deserializer<'de>,
{
    let v: Option<Value> = Option::deserialize(deserializer)?;
    Ok(v.filter(|val| !val.as_object().is_some_and(|m| m.is_empty())))
}

/// Deserialize an optional string, mapping any non-string value to `None`.
///
/// For arguments the handlers used to read with `params.get(..).and_then(as_str)`:
/// a wrong-typed value was ignored, not rejected, and must stay that way.
pub fn deserialize_lenient_string<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let v: Option<Value> = Option::deserialize(deserializer)?;
    Ok(v.and_then(|val| val.as_str().map(str::to_string)))
}

// ============================================================================
// CRUD Tool Parameters
// ============================================================================

/// Parameters for `find` tool
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FindParams {
    /// Collection name (required)
    pub collection: String,
    /// Query filter (optional, defaults to {})
    /// Accepts both "query" and "filter" for API consistency with update/delete tools
    #[serde(default = "empty_object", alias = "filter")]
    pub query: Value,
    /// Fields to include/exclude
    #[serde(skip_serializing_if = "Option::is_none")]
    pub projection: Option<Value>,
    /// Sort specification
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort: Option<Value>,
    /// Maximum documents to return
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    /// Documents to skip
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skip: Option<usize>,
    /// Include total count in response
    #[serde(default)]
    pub include_total: bool,
}

/// Parameters for `find_one` tool
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FindOneParams {
    pub collection: String,
    /// Accepts both "query" and "filter" for API consistency
    #[serde(default = "empty_object", alias = "filter")]
    pub query: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub projection: Option<Value>,
}

/// Parameters for `insert_one` tool
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InsertOneParams {
    pub collection: String,
    pub document: Value,
}

/// Parameters for `insert_many` tool
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InsertManyParams {
    pub collection: String,
    pub documents: Vec<Value>,
}

/// Parameters for `update_one` tool
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UpdateManyParams {
    pub collection: String,
    /// Accepts both "filter" and "query" for API consistency
    #[serde(alias = "query")]
    pub filter: Value,
    pub update: Value,
}

/// Parameters for `delete_one` / `delete_many` tools
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeleteParams {
    pub collection: String,
    /// Accepts both "filter" and "query" for API consistency
    #[serde(alias = "query")]
    pub filter: Value,
}

/// Parameters for `count` tool
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CountParams {
    pub collection: String,
    /// Accepts both "query" and "filter" for API consistency
    #[serde(default = "empty_object", alias = "filter")]
    pub query: Value,
}

/// Parameters for `distinct` tool
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DistinctParams {
    pub collection: String,
    pub field: String,
    /// Accepts both "query" and "filter" for API consistency
    #[serde(default = "empty_object", alias = "filter")]
    pub query: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

/// Parameters for `aggregate` tool
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AggregateParams {
    pub collection: String,
    pub pipeline: Vec<Value>,
}

// ============================================================================
// Index Tool Parameters
// ============================================================================

/// Parameters for `index_create` tool
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndexCreateParams {
    pub collection: String,
    /// Single field (for simple index)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    /// Multiple fields (for compound index)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fields: Option<Vec<String>>,
    #[serde(default)]
    pub unique: bool,
    #[serde(default)]
    pub sparse: bool,
}

/// Parameters for `index_list` tool
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndexListParams {
    pub collection: String,
    /// Index subtype: `btree`, `fulltext`, `fuzzy` or `vector`. `index_list` returns
    /// every subtype without it; `index_stats` defaults to `btree`. A value that is
    /// not a string is ignored, as the handlers always did.
    #[serde(
        rename = "type",
        default,
        deserialize_with = "deserialize_lenient_string",
        skip_serializing_if = "Option::is_none"
    )]
    pub index_type: Option<String>,
}

/// Parameters for `index_drop` tool
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndexDropParams {
    pub collection: String,
    pub index_name: String,
}

/// Parameters for the fuzzy sub-route of `index_create` (type='fuzzy')
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

/// Parameters for the fulltext sub-route of `index_create` (type='fulltext')
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FulltextIndexParams {
    pub collection: String,
    pub field: String,
    #[serde(default = "default_language")]
    pub language: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_word_length: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accent_folding: Option<bool>,
}

fn default_language() -> String {
    "none".to_string()
}

/// Parameters for `fuzzy_search` tool
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FuzzySearchParams {
    pub collection: String,
    pub field: String,
    pub query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub threshold: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub algorithm: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skip: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub projection: Option<Value>,
    /// MongoDB-style filter applied AFTER fuzzy matching (core-level filtering).
    /// Empty `{}` is normalized to `None` to avoid unnecessary collection scans.
    #[serde(default, deserialize_with = "deserialize_nonempty_filter")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
/// Field default matches RAG schema: "content"
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FulltextSearchParams {
    pub collection: String,
    /// Field with fulltext index (default: "content")
    #[serde(default = "default_content_field")]
    pub field: String,
    /// Multiple fields to search across (overrides `field` if provided)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fields: Option<Vec<String>>,
    pub query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skip: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_score: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub projection: Option<Value>,
    /// Search mode: "or" (default) = any word matches, ranked by BM25 score
    /// (industry-standard disjunctive retrieval); "and" = ALL words must match
    /// (opt-in precision filter).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// Match scope for "and" mode only: "document" (default) = all words must appear
    /// across the document's chunks, "chunk" = all words in a single chunk. Only
    /// relevant for RAG collections with chunked documents. No effect in default "or" mode.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub match_scope: Option<String>,
    /// MongoDB-style filter applied AFTER TF-IDF scoring (core-level filtering).
    /// Use this to combine fulltext search with other query operators (e.g., $regex, $eq)
    /// Example: {"from.email": {"$regex": "@scania.com$"}}
    /// Empty `{}` is normalized to `None` to avoid unnecessary collection scans.
    #[serde(default, deserialize_with = "deserialize_nonempty_filter")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<Value>,
    /// Enable highlight/snippet generation (default: false)
    #[serde(default)]
    pub highlight: bool,
    /// Characters of context around each match (default: 100)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub highlight_context: Option<usize>,
    /// Maximum snippets per field (default: 3)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub highlight_max_snippets: Option<usize>,
}

/// Parameters for `fulltext_analyze` tool (token debug)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_word_length: Option<usize>,
    /// Optional: inherit the real tokenization config of an existing fulltext index.
    /// When both `collection` and `field` are given, the index's stored language /
    /// accent_folding / min_word_length override the explicit params above, so the
    /// debugger reflects exactly how that index tokenizes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collection: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
}

fn default_true() -> bool {
    true
}

/// Parameters for `explain` tool
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExplainParams {
    pub collection: String,
    #[serde(default = "empty_object")]
    pub query: Value,
}

/// Parameters for `find_with_hint` tool
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FindWithHintParams {
    pub collection: String,
    #[serde(default = "empty_object")]
    pub query: Value,
    pub hint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub projection: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skip: Option<usize>,
}

// ============================================================================
// Collection Tool Parameters
// ============================================================================

/// Parameters for `list_collections` tool (no params needed)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ListCollectionsParams {}

/// Parameters for `collection_create` tool
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CollectionCreateParams {
    pub collection: String,
}

/// Parameters for `collection_drop` tool
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CollectionDropParams {
    pub collection: String,
}

/// Parameters for `collection_rename` tool
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CollectionRenameParams {
    pub old_collection: String,
    pub new_collection: String,
}

/// Parameters for `collection_stats` tool
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CollectionStatsParams {
    pub collection: String,
}

/// Parameters for `schema_set` tool
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SchemaSetParams {
    pub collection: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<Value>,
}

/// Parameters for `schema_get` tool
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SchemaGetParams {
    pub collection: String,
}

// ============================================================================
// Transaction Tool Parameters
// ============================================================================

/// Parameters for `transaction_commit` / `transaction_rollback` tools
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransactionIdParams {
    pub transaction_id: String,
}

/// Parameters for `transaction_insert_one` tool
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransactionInsertParams {
    pub transaction_id: String,
    pub collection: String,
    pub document: Value,
}

/// Parameters for `transaction_update_one` tool
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransactionUpdateParams {
    pub transaction_id: String,
    pub collection: String,
    /// Accepts both "filter" and "query" for API consistency
    #[serde(alias = "query")]
    pub filter: Value,
    pub update: Value,
}

/// Parameters for `transaction_delete_one` tool
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScriptSaveParams {
    pub name: String,
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dependencies: Option<Vec<String>>,
}

/// Parameters for `script_list` tool
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ScriptListParams {
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub match_all: bool,
}

/// Parameters for `script_get` / `script_delete` / `script_stats` tools
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScriptNameParams {
    pub name: String,
}

/// Parameters for `script_run` tool
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScriptRunParams {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_operations: Option<u64>,
}

/// Parameters for `script_exec` tool
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScriptExecParams {
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_operations: Option<u64>,
}

/// Parameters for `script_history` tool
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScriptHistoryParams {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

/// Parameters for `script_rollback` / `script_version_get` tools
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScriptVersionParams {
    pub name: String,
    pub version: u32,
}

/// Parameters for `script_tags_add` / `script_tags_remove` tools
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScriptTagsParams {
    pub name: String,
    pub tags: Vec<String>,
}

// ============================================================================
// ACL Tool Parameters
// ============================================================================

/// Parameters for `acl_set` tool
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AclSetParams {
    pub collection: String,
    pub rules: Vec<Value>,
}

/// Parameters for `acl_get` / `acl_delete` tools
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AclCollectionParams {
    pub collection: String,
}

// ============================================================================
// Listener Tool Parameters (Network Listeners)
// ============================================================================

/// Parameters for `listener_get` / `listener_delete` / `listener_enable` / `listener_disable` tools
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListenerIdParams {
    pub id: String,
}

/// Parameters for `listener_create` tool
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListenerAddParams {
    pub id: String,
    pub bind: String,
    #[serde(default)]
    pub tls: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cert_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

// ============================================================================
// Admin Tool Parameters
// ============================================================================

/// Parameters for `db_open` tool
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DbOpenParams {
    pub path: String,
    #[serde(default)]
    pub create: bool,
}

/// Parameters for `admin_compact` tool
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct AdminCompactParams {
    #[serde(default)]
    pub force: bool,
}

/// Parameters requiring only admin_key verification
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdminKeyParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub admin_key: Option<String>,
}

/// Parameters for `admin_collection_create_system` and `admin_collection_drop_protected` tools
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdminCollectionParams {
    pub collection: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub admin_key: Option<String>,
}

/// Parameters for `admin_collection_set_flags` tool
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdminFlagsParams {
    pub collection: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_system: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protected: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hidden: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub admin_key: Option<String>,
}

/// Parameters for `admin_apikey_create` tool
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdminApiKeyCreateParams {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub admin_key: Option<String>,
}

/// Parameters for `admin_apikey_revoke` / `admin_apikey_delete` tools
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdminApiKeyIdParams {
    pub id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub admin_key: Option<String>,
}

/// Parameters for `api_key_create` tool
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApiKeyCreateParams {
    pub name: String,
    pub admin_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_days: Option<i64>,
}

/// Parameters for `api_key_revoke` tool
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApiKeyRevokeParams {
    pub key: String,
    pub admin_key: String,
}

/// Parameters for `api_key_list` tool
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

/// Parameters for `hybrid_search` tool (RRF-based fusion with reranking, dedup, auto-embed)
///
/// Field defaults match RAG schema:
/// - vector_field: "embedding"
/// - text_field: "content"
///
/// If `vector` is omitted, the query text is automatically embedded using the
/// collection's RAG config or the specified `provider` (auto-embed mode).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HybridSearchParams {
    pub collection: String,
    /// Field with vector index (default: "embedding")
    #[serde(default = "default_vector_field")]
    pub vector_field: String,
    /// Field with fulltext index (default: "content")
    #[serde(default = "default_text_field")]
    pub text_field: String,
    /// Query embedding vector. If omitted, the query is auto-embedded using the
    /// collection's configured provider (or the `provider` parameter).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vector: Option<Vec<f64>>,
    /// Text query for fulltext search (and auto-embedding if vector is omitted)
    pub query: String,
    #[serde(default = "default_hybrid_limit")]
    pub limit: usize,
    /// Explicit vector weight override (takes priority over search_mode)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vector_weight: Option<f64>,
    /// Explicit fulltext weight override (takes priority over search_mode)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fulltext_weight: Option<f64>,
    /// MongoDB projection (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub projection: Option<Value>,

    /// Search mode preset: "balanced" (0.5/0.5), "semantic" (0.8/0.2), "keyword" (0.2/0.8)
    /// Overridden by explicit vector_weight/fulltext_weight if provided
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_mode: Option<String>,

    /// Embedding provider for auto-embed mode (uses collection RAG config if not specified)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,

    /// Enable reranking after RRF fusion (default: true)
    /// Applies exact phrase match (1.5x), keyword density (1.0-1.3x), length penalty (0.8x)
    #[serde(default = "default_rerank")]
    pub rerank: bool,

    /// Enable MMR diversity reranking (default: false)
    /// Uses embedding cosine similarity to select diverse results
    #[serde(default = "default_deduplicate")]
    pub deduplicate: bool,

    /// MMR lambda parameter for diversity vs relevance trade-off (default: 0.7)
    /// 1.0 = pure relevance, 0.0 = pure diversity
    #[serde(default = "default_mmr_lambda")]
    pub mmr_lambda: f64,
    /// MongoDB-style filter applied BEFORE both vector and fulltext search.
    /// Empty `{}` is normalized to `None` to avoid unnecessary collection scans.
    #[serde(default, deserialize_with = "deserialize_nonempty_filter")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<Value>,

    /// RRF K constant — lower = wider score spread (default: 20)
    #[serde(default = "default_rrf_k")]
    pub rrf_k: f64,

    /// Optional title field for title match boost in reranking (up to 1.5x)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title_field: Option<String>,

    /// Fulltext/fusion mode: "or" (default) = any word matches, ranked by BM25 +
    /// RRF (industry-standard disjunctive retrieval); "and" = ALL words must match
    /// (opt-in precision filter).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,

    /// Match scope for "and" mode only: "document" (default) = all words must appear
    /// across the document's chunks (not necessarily in one chunk), "chunk" = one chunk.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub match_scope: Option<String>,

    /// Multiple fulltext fields to search across (overrides text_field). Best-field strategy (max score).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_fields: Option<Vec<String>>,

    /// Merge adjacent chunks from same source document (default: true).
    /// Combines consecutive chunk_index values into single result with merged text,
    /// eliminating overlap-caused duplicates while preserving more context.
    #[serde(default = "default_merge_chunks")]
    pub merge_chunks: bool,

    /// Group results by document (default: false).
    /// When true, results are grouped by doc_id with all matching chunks per document.
    /// Limit applies to document count (not chunk count).
    #[serde(default = "default_group_by_document")]
    pub group_by_document: bool,

    /// Maximum chunks per source document.
    /// In flat mode: caps how many chunks from the same `doc_id` enter the global top-K
    /// (applied AFTER reranking, BEFORE merge_chunks/MMR).
    /// In grouped mode: caps the length of each group's `chunks[]` array.
    /// None = no cap (current behavior).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_chunks_per_doc: Option<usize>,
}

fn default_merge_chunks() -> bool {
    true
}

fn default_hybrid_limit() -> usize {
    10
}
fn default_rerank() -> bool {
    true
}
fn default_deduplicate() -> bool {
    false
}
fn default_mmr_lambda() -> f64 {
    0.7
}
fn default_group_by_document() -> bool {
    false
}
fn default_rrf_k() -> f64 {
    DEFAULT_RRF_K
}

// ============================================================================
// RAG Tool Parameters
// ============================================================================

fn default_rag_language() -> String {
    "none".to_string()
}

/// Parameters for `rag_collection_create` tool
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RagCollectionCreateParams {
    pub collection: String,
    #[serde(default = "default_vector_field")]
    pub embedding_field: String,
    #[serde(default = "default_text_field")]
    pub text_field: String,
    /// Extra text fields to fulltext-index (besides `text_field`), e.g.
    /// ["title", "customer"]. When set, hybrid_search defaults to searching all
    /// of them. Omitted → single `text_field` (backward compatible).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_fields: Option<Vec<String>>,
    /// Document-identity metadata fields prepended to each chunk's embedding text
    /// (not stored), e.g. ["customer", "title"]. Stored in the RAG config and used
    /// by rag_document_import so identical boilerplate gets distinct vectors.
    /// Omitted → verbatim embedding (backward compatible).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_fields: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default = "default_rag_language")]
    pub language: String,
}

/// Parameters for `rag_document_import` tool
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RagDocumentImportParams {
    pub collection: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc_id: Option<String>,
    #[serde(default = "default_chunk_size")]
    pub chunk_size: usize,
    #[serde(default = "default_chunk_overlap")]
    pub overlap: usize,
    #[serde(default = "default_chunk_mode")]
    pub mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default = "crate::defaults::default_if_exists")]
    pub if_exists: String,
    /// Fulltext language for the auto-created index (hungarian/english/german/none).
    /// Only applies when no fulltext index exists yet; default "none" (no stemming).
    #[serde(default = "default_rag_language")]
    pub language: String,
    /// Extra text fields to fulltext-index on the auto-created collection
    /// (besides `content`), e.g. ["title", "customer"]. Only applies when the
    /// collection has no RAG config yet. Omitted → single content index.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_fields: Option<Vec<String>>,
    /// Document-identity metadata fields prepended to each chunk's embedding text
    /// (not stored), e.g. ["customer", "title"]. Explicit value wins for THIS import;
    /// otherwise the collection's stored RAG config context_fields is used. Reserved
    /// (`doc_id`, `chunk_index`, …) and the embedding/text fields are never sourced
    /// into the prefix. NOTE: an explicit value on a collection that already has a RAG
    /// config is applied to this import only, NOT persisted (set it via
    /// rag_collection_create to persist; a warning is logged when it differs).
    /// Omitted on both → verbatim embedding (backward compatible).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_fields: Option<Vec<String>>,
}

/// Parameters for `rag_collection_stats` tool
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RagCollectionStatsParams {
    pub collection: String,
}

/// Parameters for `rag_chunks_load` tool (#73).
///
/// Loads every chunk for the given `doc_ids` from a RAG collection. Two modes:
/// - `query=None`: pure load, results sorted by `(doc_id, chunk_index)` ASC, no scoring
/// - `query=Some(...)`: scored load via fulltext OR-search filtered to `doc_ids`,
///   results sorted by `_score` DESC
///
/// In both modes adjacent chunks from the same document are merged by default
/// (`merge_chunks=true`), with table-header dedup and overlap removal — the same
/// pipeline `hybrid_search` uses.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RagLoadAllChunksParams {
    pub collection: String,
    /// Document IDs to load chunks for. Empty list returns an empty result set
    /// (NOT an error). Missing doc_ids in the collection are silently skipped.
    pub doc_ids: Vec<String>,
    /// Optional query text. When provided, chunks are scored via fulltext OR-
    /// search and the `_score` field is added to every hit. When omitted, this
    /// is a pure load with no scoring.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    /// Text field used for the fulltext search (when `query` is set) and for
    /// merge-adjacent-chunks overlap removal. Default: "content".
    #[serde(default = "default_text_field")]
    pub text_field: String,
    /// MongoDB projection (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub projection: Option<Value>,
    /// Merge adjacent chunks from the same source document (default: true).
    /// Same algorithm as `hybrid_search`: removes overlap-induced duplication
    /// and dedups table headers (#63).
    #[serde(default = "default_merge_chunks")]
    pub merge_chunks: bool,
    /// Cap chunks returned per source document. Applied AFTER merge so that
    /// a merged run counts as one chunk. Default null = no cap.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_chunks_per_doc: Option<usize>,
}

// ============================================================================
// Vector Tool Parameters
// ============================================================================

/// Parameters for the vector sub-route of `index_create` (type='vector')
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VectorIndexCreateParams {
    pub collection: String,
    pub field: String,
    pub dim: usize,
    #[serde(default = "default_metric")]
    pub metric: String,
    #[serde(default = "default_max_vectors")]
    pub max_vectors: usize,
    #[serde(default = "default_m")]
    pub m: usize,
    #[serde(default = "default_ef_construction")]
    pub ef_construction: usize,
    #[serde(default = "default_ef_search")]
    pub ef_search: usize,
}

fn default_metric() -> String {
    "cosine".to_string()
}
fn default_max_vectors() -> usize {
    DEFAULT_MAX_VECTORS
}
fn default_m() -> usize {
    16
}
fn default_ef_construction() -> usize {
    200
}
fn default_ef_search() -> usize {
    50
}

/// Parameters for `vector_search` tool
///
/// Field default matches RAG schema: "embedding"
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VectorSearchParams {
    pub collection: String,
    /// Field with vector index (default: "embedding")
    #[serde(default = "default_embedding_field")]
    pub field: String,
    pub vector: Vec<f64>,
    #[serde(default = "default_vector_limit")]
    pub limit: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub projection: Option<Value>,
}

// ============================================================================
// Retrieval Tool Parameters
// ============================================================================

/// Parameters for `search` tool (intent-only hybrid retrieval)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchParams {
    pub collection: String,
    pub query: String,
    /// Structured metadata filter (e.g. {"year": 2026}). Empty `{}` → None.
    #[serde(default, deserialize_with = "deserialize_nonempty_filter")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<Value>,
    /// Max documents (coarse intent). Default from `default_search_limit`.
    #[serde(default = "default_search_limit")]
    pub limit: usize,
    /// "structured" (default) | "context_block".
    #[serde(default = "default_format")]
    pub format: String,
    /// Include the diagnostics block (default false).
    #[serde(default)]
    pub debug: bool,
}

fn default_search_limit() -> usize {
    5
}
fn default_format() -> String {
    "structured".to_string()
}

// ============================================================================
// Embedding Tool Parameters
// ============================================================================

/// Parameters for `embed_text` tool
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmbedTextParams {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
}

/// Parameters for `embed_batch` tool
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmbedBatchParams {
    pub texts: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
}

/// Parameters for `embed_models_list` tool (no params needed)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmbedListModelsParams {}

/// Parameters for `embed_document` tool
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmbedDocumentParams {
    pub collection: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
    #[serde(default = "default_chunk_mode")]
    pub mode: String,
    #[serde(default = "default_chunk_size")]
    pub chunk_size: usize,
    #[serde(default = "default_chunk_overlap")]
    pub overlap: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default = "default_create_vector_index")]
    pub create_vector_index: bool,
    #[serde(default = "crate::defaults::default_if_exists")]
    pub if_exists: String,
}

fn default_create_vector_index() -> bool {
    true
}

// ============================================================================
// Auto-Embedding Tool Parameters
// ============================================================================

/// Parameters for `auto_embed_enable` tool
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutoEmbedEnableParams {
    pub collection: String,
    pub source_field: String,
    pub target_field: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dimension: Option<usize>,
    #[serde(default)]
    pub skip_if_exists: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunking: Option<ChunkingParams>,
}

/// Chunking parameters (subset of ChunkingConfig for MCP)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChunkingParams {
    #[serde(default = "default_chunk_mode")]
    pub mode: String,
    #[serde(default = "default_chunk_size")]
    pub chunk_size: usize,
    #[serde(default = "default_chunk_overlap")]
    pub overlap: usize,
}

/// Parameters for `auto_embed_disable` tool
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutoEmbedDisableParams {
    pub collection: String,
}

/// Parameters for `auto_embed_status` tool
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutoEmbedStatusParams {
    pub collection: String,
}

// ============================================================================
// Job Tool Parameters
// ============================================================================

/// Parameters for `embed_job_status` tool
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobStatusParams {
    pub job_id: String,
}

/// Parameters for `embed_job_list` tool
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobListParams {
    #[serde(default)]
    pub active_only: bool,
}

/// Parameters for `embed_job_cancel` tool
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobCancelParams {
    pub job_id: String,
}
