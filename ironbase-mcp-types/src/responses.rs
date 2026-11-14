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
//!
//! Write/admin tool group: `insert_one`, `insert_many`, `update_one`,
//! `update_many`, `delete_one`, `delete_many`, `collection_create`,
//! `collection_drop`, `index_create`, `index_drop`, `index_stats_refresh`.
//!
//! Search tool group: `fulltext_search`, `fulltext_analyze`, `fuzzy_search`,
//! `vector_search`, `search`.

use serde::de::Error as _;
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};

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

// ============================================================================
// Document writes
// ============================================================================

/// `insert_one` response
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InsertOneResponse {
    /// `_id` of the inserted document as a string, whatever its type (an
    /// integer `_id` too, e.g. `"10"`); a generated `_id` is the next integer
    pub inserted_id: String,
    /// `true` when auto-embedding wrote the document's vector; absent otherwise
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_embedded: Option<bool>,
}

/// `insert_many` response
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InsertManyResponse {
    /// `_id`s of the inserted documents as strings, in input order
    pub inserted_ids: Vec<String>,
    /// `inserted_ids.len()`
    pub inserted_count: usize,
    /// Number of documents auto-embedding wrote a vector into; absent when none
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_embedded_count: Option<usize>,
}

/// `update_one` response
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UpdateOneResponse {
    /// Number of documents the filter matched (0 or 1)
    pub matched_count: u64,
    /// Number of documents changed (0 when the update left the match as it was)
    pub modified_count: u64,
    /// `_id` (as a string) of the document an upsert inserted; absent otherwise
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upserted_id: Option<String>,
    /// `true` when auto-embedding wrote the upserted document's vector
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_embedded: Option<bool>,
    /// Number of documents re-embedded after the update (absent when none)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_embedded_count: Option<usize>,
    /// Why re-embedding after the stored update failed (absent when it did not)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_embed_error: Option<String>,
}

/// `update_many` response (no upsert: `update_many` ignores `upsert`)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UpdateManyResponse {
    /// Number of documents the filter matched
    pub matched_count: u64,
    /// Number of documents changed
    pub modified_count: u64,
    /// Number of documents re-embedded after the update (absent when none)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_embedded_count: Option<usize>,
    /// Why re-embedding after the stored update failed (absent when it did not)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_embed_error: Option<String>,
}

/// `delete_one` / `delete_many` response
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeleteResponse {
    /// Number of documents deleted (`delete_one`: 0 or 1)
    pub deleted_count: u64,
}

// ============================================================================
// Collection management
// ============================================================================

/// `collection_create` response. Creating an existing collection is not an
/// error: the response is the same.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CollectionCreateResponse {
    /// Always `true` (a failure is a tool error)
    pub success: bool,
    pub collection: String,
}

/// `collection_drop` response
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CollectionDropResponse {
    /// Always `true` (a failure, e.g. a missing collection, is a tool error)
    pub success: bool,
    /// The dropped collection
    pub dropped: String,
    /// Whether the collection's ACL entry was deleted with it
    pub acl_deleted: bool,
}

// ============================================================================
// Index management
// ============================================================================

/// `index_create` response of a single-field B+ tree index
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BTreeIndexCreated {
    pub index_name: String,
    pub field: String,
    pub unique: bool,
    pub sparse: bool,
}

/// `index_create` response of a compound B+ tree index (`fields` given)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompoundIndexCreated {
    pub index_name: String,
    pub fields: Vec<String>,
    pub unique: bool,
    pub sparse: bool,
}

/// `index_create` response of a fulltext index (`type: fulltext`); the
/// effective settings, defaults filled in
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FulltextIndexCreated {
    pub index_name: String,
    pub field: String,
    /// The `language` argument as given (default `none`)
    pub language: String,
    pub min_word_length: usize,
    pub accent_folding: bool,
}

/// `index_create` response of a fuzzy index (`type: fuzzy`)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FuzzyIndexCreated {
    pub index_name: String,
    pub field: String,
    /// The `algorithm` argument as given (default `jaro_winkler`); not the
    /// `index_list` spelling
    pub algorithm: String,
    pub threshold: f64,
}

/// `index_create` response of a vector index (`type: vector`)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VectorIndexCreated {
    pub index_name: String,
    pub field: String,
    pub dim: usize,
    /// The `metric` argument as given (`cosine`, `euclidean`, `dot_product`):
    /// a string, not [`VectorMetric`], whose `index_list` spelling is `dotproduct`
    pub metric: String,
    pub max_vectors: usize,
    pub hnsw_params: HnswParams,
}

/// `index_create` response.
///
/// One tool, five shapes, selected by the request (`type`, and `field` vs.
/// `fields` for B+ tree indexes). The wire has no tag, so the enum is untagged.
/// The compound, fulltext, fuzzy and vector variants each require a field no
/// other variant has (`fields`, `language`, `algorithm`, `dim`); the B+ tree
/// variant has no such field of its own, only the *combination* `field` +
/// `unique`, which no other shape carries together, so every other response
/// fails it. A real response therefore parses as exactly one variant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum IndexCreateResponse {
    BTree(BTreeIndexCreated),
    Compound(CompoundIndexCreated),
    Fulltext(FulltextIndexCreated),
    Fuzzy(FuzzyIndexCreated),
    Vector(VectorIndexCreated),
}

impl IndexCreateResponse {
    /// Name of the created index
    pub fn index_name(&self) -> &str {
        match self {
            Self::BTree(r) => &r.index_name,
            Self::Compound(r) => &r.index_name,
            Self::Fulltext(r) => &r.index_name,
            Self::Fuzzy(r) => &r.index_name,
            Self::Vector(r) => &r.index_name,
        }
    }
}

/// `index_drop` response
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndexDropResponse {
    /// Always `true` (a failure, e.g. a missing index, is a tool error)
    pub success: bool,
    /// The dropped index
    pub dropped: String,
}

/// `index_stats_refresh` response
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndexStatsRefreshResponse {
    /// Always `true` (a failure is a tool error)
    pub success: bool,
    /// Human-readable confirmation naming the collection
    pub message: String,
}

// ============================================================================
// Search
// ============================================================================
//
// The hits of `fulltext_search`, `fuzzy_search` and `vector_search` are the
// matched documents themselves, flat: the document's own fields at the top
// level, then the engine's metadata under `_`-prefixed keys. The document stays
// an open `Map` (`#[serde(flatten)]`); the metadata is typed. A document field
// named like a metadata key is overwritten by the metadata on the wire (the
// server builds the hit as one JSON object).

/// The snippets of one field (`fulltext_search` with `highlight`)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FulltextHighlight {
    pub field: String,
    /// Context around each match, the matched terms wrapped in `<mark>`…`</mark>`
    pub snippets: Vec<String>,
}

/// A `fulltext_search` hit
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FulltextHit {
    /// The document's fields (after `projection`)
    #[serde(flatten)]
    pub document: Map<String, Value>,
    /// BM25 score
    #[serde(rename = "_score")]
    pub score: f64,
    /// The query tokens (stemmed) the document matched
    #[serde(rename = "_matched_tokens")]
    pub matched_tokens: Vec<String>,
    /// Snippets per field; only with `highlight: true`
    #[serde(
        rename = "_highlights",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub highlights: Option<Vec<FulltextHighlight>>,
}

/// Which documents an AND (`mode: and`) query qualifies: all terms in one chunk,
/// or across the chunks of one document (`doc_id`)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MatchScope {
    Document,
    Chunk,
}

/// `fulltext_search` response
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FulltextSearchResponse {
    /// Hits in score order (after `skip` / `limit` / `min_score` / `filter`)
    pub results: Vec<FulltextHit>,
    /// `results.len()`
    pub count: usize,
    /// The scope the query ran with: `document` only for a document-scope AND
    /// (`mode: and`, `match_scope` not `chunk`), `chunk` otherwise
    pub match_scope: MatchScope,
    /// Number of documents a document-scope AND qualified; absent when none ran
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qualified_doc_ids: Option<usize>,
}

/// One word of `fulltext_analyze`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TokenAnalysis {
    /// The word as it appears in the text (surrounding punctuation trimmed)
    pub original: String,
    /// Lowercased (and accent-folded when folding is on)
    pub normalized: String,
    /// Stemmed, then folded: the token the index stores (for a dropped word,
    /// `normalized`)
    pub stemmed: String,
    /// Whether the word becomes a token
    pub kept: bool,
    /// Why it was dropped (too short, stop word); absent when kept
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filtered_reason: Option<String>,
}

/// `fulltext_analyze` response
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FulltextAnalyzeResponse {
    /// The analysed text
    pub input: String,
    /// The stemming language, capitalised (`Hungarian`, `English`, `German`,
    /// `None`)
    pub language: String,
    pub accent_folding: bool,
    pub min_word_length: usize,
    /// Every word, in text order
    pub tokens: Vec<TokenAnalysis>,
    /// The stop words dropped, as written in the text
    pub stop_words_removed: Vec<String>,
    /// The tokens the index would store / search for
    pub final_tokens: Vec<String>,
    /// `true` when the settings came from the fulltext index of the given
    /// `collection` + `field`, `false` for the explicit arguments
    pub inherited_from_index: bool,
}

/// A `fuzzy_search` hit
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FuzzyHit {
    /// The document's fields (after `projection`)
    #[serde(flatten)]
    pub document: Map<String, Value>,
    /// Similarity (0.0–1.0)
    #[serde(rename = "_score")]
    pub score: f64,
    /// The indexed value that matched
    #[serde(rename = "_matched_value")]
    pub matched_value: String,
    /// The matched value wrapped in `<mark>`…`</mark>`; only with `highlight: true`
    #[serde(
        rename = "_highlight",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub highlight: Option<String>,
}

/// `fuzzy_search` response
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FuzzySearchResponse {
    /// Hits in similarity order
    pub results: Vec<FuzzyHit>,
    /// `results.len()`
    pub count: usize,
}

/// Where a [`VectorSearchHit`] has its `_score` on the wire
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScorePosition {
    /// Before the document's fields: `vector_search` with a `projection`
    First,
    /// After the document's fields: `vector_search` without a `projection`
    Last,
}

/// A `vector_search` hit: the document's fields and `_score`.
///
/// The one irregular hit: the server appends `_score` to the document, but with
/// a `projection` it builds a new object that starts with `_score`
/// ([`ScorePosition`]). (De)serialized by hand to keep that order; on the way
/// in, `_score` is required and a number, and the position is read from where
/// it stands (a key-order-preserving `serde_json`, as in this workspace).
#[derive(Debug, Clone, PartialEq)]
pub struct VectorSearchHit {
    /// The document's fields (after `projection`)
    pub document: Map<String, Value>,
    /// The engine's similarity score under the index metric. Computed as
    /// `f32` and sent widened to `f64` (e.g. `0.9999997615814209`).
    pub score: f64,
    pub score_position: ScorePosition,
}

impl Serialize for VectorSearchHit {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.document.len() + 1))?;
        if self.score_position == ScorePosition::First {
            map.serialize_entry("_score", &self.score)?;
        }
        for (k, v) in &self.document {
            map.serialize_entry(k, v)?;
        }
        if self.score_position == ScorePosition::Last {
            map.serialize_entry("_score", &self.score)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for VectorSearchHit {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut document = Map::<String, Value>::deserialize(deserializer)?;
        let score_position = match document.keys().next() {
            Some(first) if first == "_score" => ScorePosition::First,
            _ => ScorePosition::Last,
        };
        let score = document
            .shift_remove("_score")
            .ok_or_else(|| D::Error::missing_field("_score"))?;
        let score = score
            .as_f64()
            .ok_or_else(|| D::Error::custom(format!("`_score` is not a number: {score}")))?;
        Ok(Self {
            document,
            score,
            score_position,
        })
    }
}

/// `vector_search` response
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VectorSearchResponse {
    /// Hits, closest first
    pub results: Vec<VectorSearchHit>,
    /// `results.len()`
    pub count: usize,
}

/// The relevance verdict of a `search` response. The contract has four states;
/// without calibration (Stage A, the current server) it is always `unknown`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchVerdict {
    Answered,
    Weak,
    #[serde(rename = "none")]
    NoAnswer,
    Unknown,
}

/// One passage of a `search` document
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchPassage {
    /// Body text: one chunk or adjacent chunks merged, possibly shortened to
    /// the context budget
    pub text: String,
}

/// A `search` document: `doc_id`, the document-level fields, `relevance`,
/// `passages`.
///
/// `doc_id` is the parent document's `doc_id` (normally a string; else its
/// `_id` as a string). A one-chunk document whose own `doc_id` field is not a
/// string sends that value instead (the document fields overwrite it on the
/// wire), so the type is open JSON: a client must not fail a whole result list
/// on one such document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchDocument {
    pub doc_id: Value,
    /// Document-level fields (those shared by every chunk, or the single
    /// chunk's), without the body text and the engine / chunk bookkeeping
    #[serde(flatten)]
    pub fields: Map<String, Value>,
    /// The document's best fused score (higher is more relevant; a rank-fusion
    /// score, not a probability)
    pub relevance: f64,
    /// Its passages in relevance order; never empty
    pub passages: Vec<SearchPassage>,
}

/// The body of a `search` response, selected by `format`: the documents
/// (`structured`) or one citation-marked text block (`context_block`)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SearchBody {
    #[serde(rename = "documents")]
    Documents(Vec<SearchDocument>),
    #[serde(rename = "context")]
    Context(String),
}

/// `search` diagnostics (`debug: true`)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchDiagnostics {
    pub stage: String,
    pub fusion: String,
    pub calibration: String,
    pub hyde: String,
    /// Chunks of the returned documents before the context budget
    pub total_chunks: usize,
    /// Documents a document-scope AND qualified: `null` (`search` runs OR)
    pub qualified_doc_ids: Option<usize>,
}

/// `search` response.
///
/// `count`, `trimmed` and the `dropped_*` counts describe what the context
/// budget left out: `trimmed` is `true` when a passage was shortened or left
/// out, or a document was dropped; `dropped_due_to_budget` documents matched
/// but did not fit; `dropped_empty_documents` matched but had no body text.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchResponse {
    pub verdict: SearchVerdict,
    #[serde(flatten)]
    pub body: SearchBody,
    /// Documents returned (in `context`, the documents rendered)
    pub count: usize,
    pub trimmed: bool,
    /// Absent when 0
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dropped_empty_documents: Option<usize>,
    /// Absent when 0
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dropped_due_to_budget: Option<usize>,
    /// Only with `debug: true`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostics: Option<SearchDiagnostics>,
    /// Why retrieval ran degraded (no embedding provider: BM25 only); absent
    /// otherwise
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub degraded: Option<String>,
}
