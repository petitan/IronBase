//! Typed response structs for the MCP tools
//!
//! One definition of what a tool returns, shared by the server (which builds and
//! serializes them) and by clients (which deserialize them). The wire format is
//! the server's, byte for byte: field order is declaration order, optional
//! fields are omitted when absent, and nothing a client parses is defaulted — a
//! missing required field is an error. Documents and other open-ended JSON stay
//! `serde_json::Value`.
//!
//! Read-side tool group: `collection_list`, `find`, `count`, `index_list`,
//! `index_stats`, `explain`, `db_stats`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ============================================================================
// Collections and documents
// ============================================================================

/// `collection_list` response
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CollectionListResponse {
    /// Collection names, in the server's (unspecified) order
    pub collections: Vec<String>,
}

/// `find` response
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FindResponse {
    /// The matching documents (after skip / limit / projection)
    pub documents: Vec<Value>,
    /// Number of documents returned (`documents.len()`)
    pub count: usize,
    /// Total number of matching documents; only with `include_total: true`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<usize>,
}

/// `count` response
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CountResponse {
    /// Number of matching documents
    pub count: u64,
}

// ============================================================================
// Indexes
// ============================================================================

/// Distance metric of a vector index, as the server spells it
/// (`cosine`, `euclidean`, `dotproduct`)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VectorMetric {
    Cosine,
    Euclidean,
    DotProduct,
}

/// A fulltext index (`index_list` / `index_stats` with `type: fulltext`)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FulltextIndexInfo {
    pub name: String,
    pub field: String,
    /// Stemming language, lowercase (`hungarian`, `english`, `german`, `none`)
    pub language: String,
    pub min_word_length: usize,
    pub accent_folding: bool,
    pub num_documents: usize,
    pub num_tokens: usize,
}

/// A fuzzy index (`index_list` / `index_stats` with `type: fuzzy`)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FuzzyIndexInfo {
    pub name: String,
    pub field: String,
    /// Similarity algorithm, lowercase without separators (e.g. `jarowinkler`)
    pub algorithm: String,
    pub threshold: f64,
    pub num_entries: usize,
}

/// HNSW parameters of a vector index
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HnswParams {
    pub m: usize,
    pub ef_construction: usize,
    pub ef_search: usize,
}

/// A vector index (`index_list` / `index_stats` with `type: vector`)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VectorIndexInfo {
    pub name: String,
    pub field: String,
    /// Vector dimension
    pub dim: usize,
    pub metric: VectorMetric,
    pub max_vectors: usize,
    pub vector_count: usize,
    pub cache_file: String,
    pub created_at: String,
    pub hnsw_params: HnswParams,
}

/// Statistics of a B+ tree index (`index_stats` with `type: btree`)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BTreeIndexStats {
    pub name: String,
    /// Primary (first) field
    pub field: String,
    pub num_keys: u64,
    pub distinct_count: u64,
    pub has_histogram: bool,
    pub has_mcv: bool,
}

/// One index subtype's list: `{"indexes": [...], "count": n}`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TypedIndexList<T> {
    pub indexes: Vec<T>,
    pub count: usize,
}

impl<T> TypedIndexList<T> {
    /// The list with its `count`
    pub fn new(indexes: Vec<T>) -> Self {
        let count = indexes.len();
        Self { indexes, count }
    }
}

/// Per-subtype counts of an `index_list` without `type`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndexListSummary {
    pub btree_count: usize,
    pub fulltext_count: usize,
    pub fuzzy_count: usize,
    pub vector_count: usize,
    pub total: usize,
}

/// `index_list` without `type`: every subtype
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndexListAll {
    /// Names of every B+ tree-backed index. The engine registers fulltext, fuzzy
    /// and vector indexes here too (under their own names).
    pub btree_indexes: Vec<String>,
    pub fulltext_indexes: Vec<FulltextIndexInfo>,
    pub fuzzy_indexes: Vec<FuzzyIndexInfo>,
    pub vector_indexes: Vec<VectorIndexInfo>,
    pub summary: IndexListSummary,
}

impl IndexListAll {
    /// The lists with their summary
    pub fn new(
        btree_indexes: Vec<String>,
        fulltext_indexes: Vec<FulltextIndexInfo>,
        fuzzy_indexes: Vec<FuzzyIndexInfo>,
        vector_indexes: Vec<VectorIndexInfo>,
    ) -> Self {
        let summary = IndexListSummary {
            btree_count: btree_indexes.len(),
            fulltext_count: fulltext_indexes.len(),
            fuzzy_count: fuzzy_indexes.len(),
            vector_count: vector_indexes.len(),
            total: btree_indexes.len()
                + fulltext_indexes.len()
                + fuzzy_indexes.len()
                + vector_indexes.len(),
        };
        Self {
            btree_indexes,
            fulltext_indexes,
            fuzzy_indexes,
            vector_indexes,
            summary,
        }
    }
}

/// `index_list` response.
///
/// Two shapes on the wire, selected by the request's `type`, without a tag:
/// every subtype ([`IndexListAll`]) when `type` is absent, otherwise only that
/// subtype's `{"indexes", "count"}`. Untagged, so a client tells the variants
/// apart by their fields; an empty typed list has no item to tell its subtype
/// by and parses as [`IndexListResponse::BTree`] (the caller knows what it asked).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum IndexListResponse {
    All(IndexListAll),
    BTree(TypedIndexList<String>),
    Fulltext(TypedIndexList<FulltextIndexInfo>),
    Fuzzy(TypedIndexList<FuzzyIndexInfo>),
    Vector(TypedIndexList<VectorIndexInfo>),
}

/// `index_stats` response: the subtype in `type` (first on the wire), then its
/// `indexes` and `count`. B+ tree indexes report statistics; the other subtypes
/// report the same objects as `index_list`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum IndexStatsResponse {
    #[serde(rename = "btree")]
    BTree(TypedIndexList<BTreeIndexStats>),
    #[serde(rename = "fulltext")]
    Fulltext(TypedIndexList<FulltextIndexInfo>),
    #[serde(rename = "fuzzy")]
    Fuzzy(TypedIndexList<FuzzyIndexInfo>),
    #[serde(rename = "vector")]
    Vector(TypedIndexList<VectorIndexInfo>),
}

// ============================================================================
// Query plan
// ============================================================================

/// `explain` response.
///
/// `plan` is the engine's query-plan object passed through unchanged; its shape
/// depends on the plan (single index, compound, logical `$and`/`$or`, collection
/// scan) and is owned by the engine, so it stays a `Value`. [`QueryPlanSummary`]
/// reads the fields every plan has.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExplainResponse {
    pub plan: Value,
}

impl ExplainResponse {
    /// The chosen plan's summary (fails if the plan has no `chosenPlan.queryPlan`)
    pub fn summary(&self) -> Result<QueryPlanSummary, serde_json::Error> {
        QueryPlanSummary::deserialize(&self.plan)
    }
}

/// The chosen access path of a query plan (read-only view of `plan.chosenPlan`)
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ChosenPlan {
    /// `IndexScan`, `CompoundIndexScan`, `IndexRangeScan`, `CollectionScan`, ...
    #[serde(rename = "queryPlan")]
    pub query_plan: String,
    /// The index used (`null` / absent for a collection or logical scan)
    #[serde(rename = "indexUsed", default)]
    pub index_used: Option<String>,
    /// Execution stage (`FETCH_WITH_INDEX`, `FULL_SCAN`, ...)
    #[serde(default)]
    pub stage: Option<String>,
    /// Asymptotic cost (`O(log n)`, `O(n)`, ...)
    #[serde(rename = "estimatedCost", default)]
    pub estimated_cost: Option<String>,
}

/// Read-only view of an `explain` plan: the fields shared by every plan shape.
/// Only `chosenPlan.queryPlan` is required; the rest depends on the plan.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct QueryPlanSummary {
    #[serde(rename = "chosenPlan")]
    pub chosen_plan: ChosenPlan,
    /// Why this plan was chosen
    #[serde(rename = "selectionReason", default)]
    pub selection_reason: Option<String>,
    /// Estimated rows of the chosen index plan (absent for collection scans
    /// and logical plans)
    #[serde(rename = "estimatedRows", default)]
    pub estimated_rows: Option<f64>,
}

// ============================================================================
// Database statistics
// ============================================================================

/// The database file
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DbStatsDatabase {
    pub path: String,
    pub file_size_bytes: u64,
    /// Human-readable file size (e.g. `15.50 MB`)
    pub file_size: String,
}

/// A vector index in `db_stats`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DbStatsVectorIndex {
    pub name: String,
    pub field: String,
    pub dim: usize,
    pub metric: VectorMetric,
    pub vector_count: usize,
}

/// One (non-system) collection in `db_stats`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DbStatsCollection {
    pub name: String,
    pub document_count: u64,
    pub btree_index_count: usize,
    pub btree_indexes: Vec<String>,
    pub vector_index_count: usize,
    pub vector_indexes: Vec<DbStatsVectorIndex>,
    pub total_vectors: usize,
}

/// Totals over the collections
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DbStatsSummary {
    pub collection_count: usize,
    pub total_documents: u64,
    pub total_btree_indexes: usize,
    pub total_vector_indexes: usize,
    pub total_vectors: usize,
}

/// Database statistics (also the `ironbase://stats` resource)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DatabaseStats {
    pub database: DbStatsDatabase,
    pub collections: Vec<DbStatsCollection>,
    pub summary: DbStatsSummary,
}

/// The server answering `db_stats`: an HTTP server or a stdio process.
/// Untagged: the HTTP shape has `protocol`/`host`/`port`, the stdio one `mode`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DbStatsServer {
    Http {
        version: String,
        protocol: String,
        host: String,
        port: u16,
        require_api_key: bool,
    },
    Stdio {
        version: String,
        /// Always `stdio`
        mode: String,
    },
}

/// Storage wastage of the database file
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StorageWastage {
    pub file_size_bytes: u64,
    pub file_size_mb: u64,
    pub estimated_live_bytes: u64,
    pub estimated_live_mb: u64,
    /// `file_size / estimated_live` with two decimals, or
    /// `infinity (never compacted)`
    pub bloat_ratio: String,
    pub total_writes: u64,
    pub total_live: u64,
    pub dead_writes: u64,
}

/// `db_stats` response: the database statistics, then the server and the
/// storage wastage
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DbStatsResponse {
    #[serde(flatten)]
    pub stats: DatabaseStats,
    pub server: DbStatsServer,
    pub storage_wastage: StorageWastage,
}
