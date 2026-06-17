//! Typed parameter structs for MCP tools
//!
//! These structs provide compile-time type safety and automatic validation
//! via serde deserialization, replacing manual Value parsing.

use serde::{Deserialize, Deserializer};
use serde_json::{json, Value};

use super::defaults::{
    default_chunk_mode, default_chunk_overlap, default_chunk_size, DEFAULT_RRF_K,
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
pub(crate) fn deserialize_nonempty_filter<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<Value>, D::Error>
where
    D: Deserializer<'de>,
{
    let v: Option<Value> = Option::deserialize(deserializer)?;
    Ok(v.filter(|val| !val.as_object().is_some_and(|m| m.is_empty())))
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

/// Parameters for `count` tool
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

/// Parameters for the fuzzy sub-route of `index_create` (type='fuzzy')
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

/// Parameters for the fulltext sub-route of `index_create` (type='fulltext')
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
    /// MongoDB-style filter applied AFTER fuzzy matching (core-level filtering).
    /// Empty `{}` is normalized to `None` to avoid unnecessary collection scans.
    #[serde(default, deserialize_with = "deserialize_nonempty_filter")]
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
#[derive(Debug, Deserialize)]
pub struct FulltextSearchParams {
    pub collection: String,
    /// Field with fulltext index (default: "content")
    #[serde(default = "default_content_field")]
    pub field: String,
    /// Multiple fields to search across (overrides `field` if provided)
    pub fields: Option<Vec<String>>,
    pub query: String,
    pub limit: Option<usize>,
    pub skip: Option<usize>,
    pub min_score: Option<f64>,
    pub projection: Option<Value>,
    /// Search mode: "or" (default) = any word matches, ranked by BM25 score
    /// (industry-standard disjunctive retrieval); "and" = ALL words must match
    /// (opt-in precision filter).
    pub mode: Option<String>,
    /// Match scope for "and" mode only: "document" (default) = all words must appear
    /// across the document's chunks, "chunk" = all words in a single chunk. Only
    /// relevant for RAG collections with chunked documents. No effect in default "or" mode.
    pub match_scope: Option<String>,
    /// MongoDB-style filter applied AFTER TF-IDF scoring (core-level filtering).
    /// Use this to combine fulltext search with other query operators (e.g., $regex, $eq)
    /// Example: {"from.email": {"$regex": "@scania.com$"}}
    /// Empty `{}` is normalized to `None` to avoid unnecessary collection scans.
    #[serde(default, deserialize_with = "deserialize_nonempty_filter")]
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
    /// Optional: inherit the real tokenization config of an existing fulltext index.
    /// When both `collection` and `field` are given, the index's stored language /
    /// accent_folding / min_word_length override the explicit params above, so the
    /// debugger reflects exactly how that index tokenizes.
    pub collection: Option<String>,
    pub field: Option<String>,
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

/// Parameters for `collection_rename` tool
#[derive(Debug, Deserialize)]
pub struct CollectionRenameParams {
    pub old_collection: String,
    pub new_collection: String,
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

/// Parameters for `transaction_commit` / `transaction_rollback` tools
#[derive(Debug, Deserialize)]
pub struct TransactionIdParams {
    pub transaction_id: String,
}

/// Parameters for `transaction_insert_one` tool
#[derive(Debug, Deserialize)]
pub struct TransactionInsertParams {
    pub transaction_id: String,
    pub collection: String,
    pub document: Value,
}

/// Parameters for `transaction_update_one` tool
#[derive(Debug, Deserialize)]
pub struct TransactionUpdateParams {
    pub transaction_id: String,
    pub collection: String,
    /// Accepts both "filter" and "query" for API consistency
    #[serde(alias = "query")]
    pub filter: Value,
    pub update: Value,
}

/// Parameters for `transaction_delete_one` tool
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

/// Parameters for `listener_create` tool
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

/// Parameters for `admin_collection_create_system` and `admin_collection_drop_protected` tools
#[derive(Debug, Deserialize)]
pub struct AdminCollectionParams {
    pub collection: String,
    pub admin_key: Option<String>,
}

/// Parameters for `admin_collection_set_flags` tool
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

/// Parameters for `hybrid_search` tool (RRF-based fusion with reranking, dedup, auto-embed)
///
/// Field defaults match RAG schema:
/// - vector_field: "embedding"
/// - text_field: "content"
///
/// If `vector` is omitted, the query text is automatically embedded using the
/// collection's RAG config or the specified `provider` (auto-embed mode).
#[derive(Debug, Deserialize)]
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
    pub vector: Option<Vec<f64>>,
    /// Text query for fulltext search (and auto-embedding if vector is omitted)
    pub query: String,
    #[serde(default = "default_hybrid_limit")]
    pub limit: usize,
    /// Explicit vector weight override (takes priority over search_mode)
    pub vector_weight: Option<f64>,
    /// Explicit fulltext weight override (takes priority over search_mode)
    pub fulltext_weight: Option<f64>,
    /// MongoDB projection (optional)
    pub projection: Option<Value>,

    /// Search mode preset: "balanced" (0.5/0.5), "semantic" (0.8/0.2), "keyword" (0.2/0.8)
    /// Overridden by explicit vector_weight/fulltext_weight if provided
    pub search_mode: Option<String>,

    /// Embedding provider for auto-embed mode (uses collection RAG config if not specified)
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
    pub filter: Option<Value>,

    /// RRF K constant — lower = wider score spread (default: 20)
    #[serde(default = "default_rrf_k")]
    pub rrf_k: f64,

    /// Optional title field for title match boost in reranking (up to 1.5x)
    pub title_field: Option<String>,

    /// Fulltext/fusion mode: "or" (default) = any word matches, ranked by BM25 +
    /// RRF (industry-standard disjunctive retrieval); "and" = ALL words must match
    /// (opt-in precision filter).
    pub mode: Option<String>,

    /// Match scope for "and" mode only: "document" (default) = all words must appear
    /// across the document's chunks (not necessarily in one chunk), "chunk" = one chunk.
    pub match_scope: Option<String>,

    /// Multiple fulltext fields to search across (overrides text_field). Best-field strategy (max score).
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
// Search Mode Presets
// ============================================================================

/// Named search mode presets for LLM-friendly weight configuration
///
/// Instead of tuning numerical weights (which LLMs are bad at),
/// the LLM picks a semantic mode name and the server resolves it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchMode {
    /// Equal weight for vector and fulltext (0.5 / 0.5)
    Balanced,
    /// Favor vector similarity (0.8 / 0.2) — conceptual queries
    Semantic,
    /// Favor fulltext matching (0.2 / 0.8) — specific term queries
    Keyword,
}

impl SearchMode {
    /// Parse search mode from string
    pub fn parse_mode(s: &str) -> Option<Self> {
        match s {
            "balanced" => Some(Self::Balanced),
            "semantic" => Some(Self::Semantic),
            "keyword" => Some(Self::Keyword),
            _ => None,
        }
    }

    /// Get (vector_weight, fulltext_weight) for this mode
    pub fn weights(&self) -> (f64, f64) {
        match self {
            Self::Balanced => (0.5, 0.5),
            Self::Semantic => (0.8, 0.2),
            Self::Keyword => (0.2, 0.8),
        }
    }
}

/// Resolve actual weights from search_mode + explicit overrides
///
/// Priority: explicit weights > search_mode preset > balanced default
pub fn resolve_weights(
    search_mode: Option<&str>,
    explicit_vector_weight: Option<f64>,
    explicit_fulltext_weight: Option<f64>,
) -> Result<(f64, f64)> {
    // Explicit weights take priority (either one triggers explicit mode)
    if explicit_vector_weight.is_some() || explicit_fulltext_weight.is_some() {
        let vw = explicit_vector_weight.unwrap_or(0.5);
        let fw = explicit_fulltext_weight.unwrap_or(0.5);
        if vw.abs() < f64::EPSILON && fw.abs() < f64::EPSILON {
            return Err(McpError::invalid_params(
                "At least one of vector_weight or fulltext_weight must be positive.",
            ));
        }
        // Normalize so weights sum to 1.0
        let total = vw + fw;
        return Ok((vw / total, fw / total));
    }

    // Search mode presets
    if let Some(mode_str) = search_mode {
        let mode = SearchMode::parse_mode(mode_str).ok_or_else(|| {
            McpError::invalid_params(format!(
                "Invalid search_mode '{}'. Use: 'balanced', 'semantic', or 'keyword'",
                mode_str
            ))
        })?;
        return Ok(mode.weights());
    }

    // Default: balanced
    Ok((0.5, 0.5))
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
    /// Extra text fields to fulltext-index (besides `text_field`), e.g.
    /// ["title", "customer"]. When set, hybrid_search defaults to searching all
    /// of them. Omitted → single `text_field` (backward compatible).
    pub text_fields: Option<Vec<String>>,
    /// Document-identity metadata fields prepended to each chunk's embedding text
    /// (not stored), e.g. ["customer", "title"]. Stored in the RAG config and used
    /// by rag_document_import so identical boilerplate gets distinct vectors.
    /// Omitted → verbatim embedding (backward compatible).
    pub context_fields: Option<Vec<String>>,
    pub provider: Option<String>,
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
    #[serde(default = "crate::tools::helpers::default_if_exists")]
    pub if_exists: String,
    /// Fulltext language for the auto-created index (hungarian/english/german/none).
    /// Only applies when no fulltext index exists yet; default "none" (no stemming).
    #[serde(default = "default_rag_language")]
    pub language: String,
    /// Extra text fields to fulltext-index on the auto-created collection
    /// (besides `content`), e.g. ["title", "customer"]. Only applies when the
    /// collection has no RAG config yet. Omitted → single content index.
    pub text_fields: Option<Vec<String>>,
    /// Document-identity metadata fields prepended to each chunk's embedding text
    /// (not stored), e.g. ["customer", "title"]. Explicit value wins; otherwise the
    /// collection's stored RAG config context_fields is used. Omitted on both →
    /// verbatim embedding (backward compatible).
    pub context_fields: Option<Vec<String>>,
}

/// Parameters for `rag_collection_stats` tool
#[derive(Debug, Deserialize)]
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
#[derive(Debug, Deserialize)]
pub struct RagLoadAllChunksParams {
    pub collection: String,
    /// Document IDs to load chunks for. Empty list returns an empty result set
    /// (NOT an error). Missing doc_ids in the collection are silently skipped.
    pub doc_ids: Vec<String>,
    /// Optional query text. When provided, chunks are scored via fulltext OR-
    /// search and the `_score` field is added to every hit. When omitted, this
    /// is a pure load with no scoring.
    pub query: Option<String>,
    /// Text field used for the fulltext search (when `query` is set) and for
    /// merge-adjacent-chunks overlap removal. Default: "content".
    #[serde(default = "default_text_field")]
    pub text_field: String,
    /// MongoDB projection (optional)
    pub projection: Option<Value>,
    /// Merge adjacent chunks from the same source document (default: true).
    /// Same algorithm as `hybrid_search`: removes overlap-induced duplication
    /// and dedups table headers (#63).
    #[serde(default = "default_merge_chunks")]
    pub merge_chunks: bool,
    /// Cap chunks returned per source document. Applied AFTER merge so that
    /// a merged run counts as one chunk. Default null = no cap.
    pub max_chunks_per_doc: Option<usize>,
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

    // =========================================================================
    // Search Mode Preset Tests
    // =========================================================================

    #[test]
    fn test_search_mode_from_str() {
        assert_eq!(
            SearchMode::parse_mode("balanced"),
            Some(SearchMode::Balanced)
        );
        assert_eq!(
            SearchMode::parse_mode("semantic"),
            Some(SearchMode::Semantic)
        );
        assert_eq!(SearchMode::parse_mode("keyword"), Some(SearchMode::Keyword));
        assert_eq!(SearchMode::parse_mode("invalid"), None);
        assert_eq!(SearchMode::parse_mode(""), None);
    }

    #[test]
    fn test_search_mode_weights() {
        assert_eq!(SearchMode::Balanced.weights(), (0.5, 0.5));
        assert_eq!(SearchMode::Semantic.weights(), (0.8, 0.2));
        assert_eq!(SearchMode::Keyword.weights(), (0.2, 0.8));
    }

    #[test]
    fn test_resolve_weights_default() {
        // No mode, no explicit weights → balanced
        let (vw, fw) = resolve_weights(None, None, None).unwrap();
        assert_eq!(vw, 0.5);
        assert_eq!(fw, 0.5);
    }

    #[test]
    fn test_resolve_weights_search_mode_semantic() {
        let (vw, fw) = resolve_weights(Some("semantic"), None, None).unwrap();
        assert_eq!(vw, 0.8);
        assert_eq!(fw, 0.2);
    }

    #[test]
    fn test_resolve_weights_search_mode_keyword() {
        let (vw, fw) = resolve_weights(Some("keyword"), None, None).unwrap();
        assert_eq!(vw, 0.2);
        assert_eq!(fw, 0.8);
    }

    #[test]
    fn test_resolve_weights_explicit_overrides_mode() {
        // Explicit weights take priority over search_mode
        let (vw, fw) = resolve_weights(Some("semantic"), Some(0.3), Some(0.7)).unwrap();
        assert_eq!(vw, 0.3);
        assert_eq!(fw, 0.7);
    }

    #[test]
    fn test_resolve_weights_partial_explicit() {
        // Only one explicit weight → other defaults to 0.5, then normalized to sum=1.0
        let (vw, fw) = resolve_weights(None, Some(0.9), None).unwrap();
        let total = 0.9 + 0.5;
        assert!((vw - 0.9 / total).abs() < 1e-10);
        assert!((fw - 0.5 / total).abs() < 1e-10);
    }

    #[test]
    fn test_resolve_weights_partial_explicit_overrides_mode() {
        // One explicit weight triggers explicit mode (ignores search_mode), normalized
        let (vw, fw) = resolve_weights(Some("keyword"), None, Some(0.9)).unwrap();
        let total = 0.5 + 0.9;
        assert!((vw - 0.5 / total).abs() < 1e-10);
        assert!((fw - 0.9 / total).abs() < 1e-10);
    }

    #[test]
    fn test_resolve_weights_invalid_mode() {
        let result = resolve_weights(Some("turbo"), None, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_hybrid_params_with_search_mode() {
        let params = json!({
            "collection": "test",
            "vector": [0.1, 0.2],
            "query": "test",
            "search_mode": "semantic"
        });
        let p: HybridSearchParams = HybridSearchParams::parse(params).unwrap();
        assert_eq!(p.search_mode.as_deref(), Some("semantic"));
        assert!(p.vector_weight.is_none());
        assert!(p.fulltext_weight.is_none());
    }

    #[test]
    fn test_hybrid_params_auto_embed_mode() {
        // No vector → auto-embed mode
        let params = json!({
            "collection": "docs",
            "query": "semantic search test"
        });
        let p: HybridSearchParams = HybridSearchParams::parse(params).unwrap();
        assert_eq!(p.collection, "docs");
        assert_eq!(p.query, "semantic search test");
        assert!(p.vector.is_none()); // auto-embed mode
        assert!(p.provider.is_none()); // use collection config
    }

    #[test]
    fn test_hybrid_params_auto_embed_with_provider() {
        let params = json!({
            "collection": "docs",
            "query": "keresés",
            "provider": "ollama"
        });
        let p: HybridSearchParams = HybridSearchParams::parse(params).unwrap();
        assert!(p.vector.is_none());
        assert_eq!(p.provider.as_deref(), Some("ollama"));
    }

    #[test]
    fn test_hybrid_params_explicit_weights_with_mode() {
        // Explicit weights should be parsed even alongside search_mode
        let params = json!({
            "collection": "test",
            "vector": [0.1, 0.2],
            "query": "test",
            "search_mode": "semantic",
            "vector_weight": 0.6,
            "fulltext_weight": 0.4
        });
        let p: HybridSearchParams = HybridSearchParams::parse(params).unwrap();
        assert_eq!(p.search_mode.as_deref(), Some("semantic"));
        assert_eq!(p.vector_weight, Some(0.6));
        assert_eq!(p.fulltext_weight, Some(0.4));
    }
}
