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
//!
//! Embedding tool group: `embed_text`, `embed_batch`, `embed_models_list`,
//! `embed_cache_stats`, `embed_cache_clear`, `auto_embed_enable`,
//! `auto_embed_disable`, `auto_embed_status`, `embed_job_list`,
//! `embed_job_status`, `embed_job_cancel`.
//!
//! RAG tool group: `rag_collection_create`, `rag_document_import`,
//! `rag_collection_stats`, `rag_chunks_load`, `embed_document`.

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
// an open `Map`; the metadata is typed. The server builds a hit as one JSON
// object, so a document field named like a metadata key (or, in a `search`
// document, `relevance` / `passages`) meets the metadata on the wire:
//
// - Always written, so always the engine's: `_score` and `_matched_tokens`
//   (`fulltext_search`), `_score` and `_matched_value` (`fuzzy_search`),
//   `_score` (`vector_search` without a `projection`), `relevance` and
//   `passages` (`search`). The metadata replaces the field's value at the
//   field's position; the document's own value is not sent.
// - Written only when enabled: `_highlights` (`fulltext_search`) and
//   `_highlight` (`fuzzy_search`) with `highlight: true`. Otherwise the
//   document's own field reaches the wire as it is.
// - `vector_search` with a `projection` writes `_score` first, then the
//   document: a document `_score` the projection keeps replaces the engine's
//   score (any JSON value). A NaN or infinite score is sent as `null`.
//
// So a client parses leniently: a metadata key whose value does not have the
// engine's shape (an optional or overridable key) is taken as a document
// field and the metadata as absent; one hit never fails the whole list. A
// metadata key that stands among the document's fields (not after them) also
// stays in `document`, marking its position; on the way out, the typed
// metadata is written there. Either way a parsed hit serializes back to the
// same bytes. The shape of an enabled-only key is not always decisive: a
// document's own `_highlights` shaped like the engine's is taken as the
// metadata.

/// Takes the engine metadata `keys` (in the order the server appends them) out
/// of a hit's JSON object `map`. The trailing run of `keys`, in that order,
/// whose values `is_engine` accepts is removed; a key of `keys` still in `map`
/// whose value `is_engine` accepts is also returned, but stays in `map` (the
/// metadata stands at the document field's position). A key whose value
/// `is_engine` rejects is a document field: `None`, left in `map`.
fn take_metadata<const N: usize>(
    map: &mut Map<String, Value>,
    keys: [&str; N],
    is_engine: impl Fn(&str, &Value) -> bool,
) -> [Option<Value>; N] {
    let mut found: [Option<Value>; N] = std::array::from_fn(|_| None);
    let mut below = N;
    while let Some((key, value)) = map.iter().next_back() {
        match keys[..below].iter().position(|k| k == key) {
            Some(i) if is_engine(key, value) => {
                let key = key.clone();
                found[i] = map.shift_remove(&key);
                below = i;
            }
            _ => break,
        }
    }
    for (slot, key) in found.iter_mut().zip(keys) {
        if slot.is_none() {
            *slot = map.get(key).filter(|v| is_engine(key, v)).cloned();
        }
    }
    found
}

/// A required metadata value of [`take_metadata`] as `T`
fn required_metadata<T: serde::de::DeserializeOwned, E: serde::de::Error>(
    value: Option<Value>,
    key: &'static str,
) -> Result<T, E> {
    let value = value.ok_or_else(|| E::missing_field(key))?;
    serde_json::from_value(value).map_err(|e| E::custom(format!("`{key}`: {e}")))
}

/// Serializes a hit as the server builds it: `leading` first, then the
/// `document` (without the `leading` key), each document field named like a
/// present `metadata` key with the metadata's value, then the other present
/// `metadata`, in order
fn serialize_hit<S: Serializer>(
    serializer: S,
    leading: Option<(&str, &Value)>,
    document: &Map<String, Value>,
    metadata: &[(&str, Option<Value>)],
) -> Result<S::Ok, S::Error> {
    let leading_key = leading.map(|(k, _)| k);
    let fields = document
        .iter()
        .filter(|(k, _)| Some(k.as_str()) != leading_key);
    let appended = metadata
        .iter()
        .filter(|(k, v)| v.is_some() && !document.contains_key(*k));
    let mut map = serializer.serialize_map(Some(
        usize::from(leading.is_some()) + fields.clone().count() + appended.clone().count(),
    ))?;
    if let Some((k, v)) = leading {
        map.serialize_entry(k, v)?;
    }
    for (k, v) in fields {
        let meta = metadata
            .iter()
            .find(|(m, _)| m == k)
            .and_then(|(_, v)| v.as_ref());
        map.serialize_entry(k, meta.unwrap_or(v))?;
    }
    for (k, v) in appended {
        map.serialize_entry(k, v)?;
    }
    map.end()
}

/// A metadata value as JSON, for [`serialize_hit`]
fn metadata_value<T: Serialize, E: serde::ser::Error>(value: &T) -> Result<Value, E> {
    serde_json::to_value(value).map_err(E::custom)
}

/// The snippets of one field (`fulltext_search` with `highlight`)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FulltextHighlight {
    pub field: String,
    /// Context around each match, the matched terms wrapped in `<mark>`…`</mark>`
    pub snippets: Vec<String>,
}

/// A `fulltext_search` hit. (De)serialized by hand: see the section notes on
/// document fields named like the metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct FulltextHit {
    /// The document's fields (after `projection`); also a document
    /// `_highlights` that is not shaped like the engine's
    pub document: Map<String, Value>,
    /// BM25 score (`_score`)
    pub score: f64,
    /// The query tokens (stemmed) the document matched (`_matched_tokens`)
    pub matched_tokens: Vec<String>,
    /// Snippets per field (`_highlights`); only with `highlight: true`
    pub highlights: Option<Vec<FulltextHighlight>>,
}

impl Serialize for FulltextHit {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let metadata = [
            ("_score", Some(metadata_value(&self.score)?)),
            (
                "_matched_tokens",
                Some(metadata_value(&self.matched_tokens)?),
            ),
            (
                "_highlights",
                self.highlights.as_ref().map(metadata_value).transpose()?,
            ),
        ];
        serialize_hit(serializer, None, &self.document, &metadata)
    }
}

impl<'de> Deserialize<'de> for FulltextHit {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut document = Map::<String, Value>::deserialize(deserializer)?;
        let [score, matched_tokens, highlights] = take_metadata(
            &mut document,
            ["_score", "_matched_tokens", "_highlights"],
            |key, value| {
                key != "_highlights" || Vec::<FulltextHighlight>::deserialize(value).is_ok()
            },
        );
        Ok(Self {
            score: required_metadata(score, "_score")?,
            matched_tokens: required_metadata(matched_tokens, "_matched_tokens")?,
            highlights: highlights
                .map(|v| required_metadata(Some(v), "_highlights"))
                .transpose()?,
            document,
        })
    }
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

/// A `fuzzy_search` hit. (De)serialized by hand: see the section notes on
/// document fields named like the metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct FuzzyHit {
    /// The document's fields (after `projection`); also a document
    /// `_highlight` other than the engine's
    pub document: Map<String, Value>,
    /// Similarity, 0.0–1.0 (`_score`)
    pub score: f64,
    /// The indexed value that matched (`_matched_value`)
    pub matched_value: String,
    /// The matched value wrapped in `<mark>`…`</mark>` (`_highlight`); only with
    /// `highlight: true` and a non-empty matched value. Any other `_highlight`
    /// is the document's own field.
    pub highlight: Option<String>,
}

impl Serialize for FuzzyHit {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let metadata = [
            ("_score", Some(metadata_value(&self.score)?)),
            (
                "_matched_value",
                Some(Value::from(self.matched_value.as_str())),
            ),
            ("_highlight", self.highlight.as_deref().map(Value::from)),
        ];
        serialize_hit(serializer, None, &self.document, &metadata)
    }
}

impl<'de> Deserialize<'de> for FuzzyHit {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut document = Map::<String, Value>::deserialize(deserializer)?;
        // The engine's highlight, as the core builds it
        let engine_highlight = document
            .get("_matched_value")
            .and_then(Value::as_str)
            .filter(|m| !m.is_empty())
            .map(|m| format!("<mark>{m}</mark>"));
        let [score, matched_value, highlight] = take_metadata(
            &mut document,
            ["_score", "_matched_value", "_highlight"],
            |key, value| {
                key != "_highlight"
                    || engine_highlight.is_some() && value.as_str() == engine_highlight.as_deref()
            },
        );
        Ok(Self {
            score: required_metadata(score, "_score")?,
            matched_value: required_metadata(matched_value, "_matched_value")?,
            highlight: highlight
                .map(|v| required_metadata(Some(v), "_highlight"))
                .transpose()?,
            document,
        })
    }
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
/// ([`ScorePosition`]), and there a document `_score` the projection keeps
/// replaces the engine's. (De)serialized by hand to keep that order; on the way
/// in, `_score` is required, the position is read from where it stands (a
/// key-order-preserving `serde_json`, as in this workspace), and a `_score`
/// that is not a float (a document's own value, or `null` for a NaN / infinite
/// score) stays in `document`, the score unavailable (`None`). See the section
/// notes on document fields named like the metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct VectorSearchHit {
    /// The document's fields (after `projection`); also a `_score` that is not
    /// the engine's number
    pub document: Map<String, Value>,
    /// The engine's similarity score under the index metric. Computed as
    /// `f32` and sent widened to `f64` (e.g. `0.9999997615814209`). `None`
    /// when the hit carries no usable score: NaN / infinite (sent as `null`),
    /// or replaced by the document's own `_score` (`document["_score"]`).
    pub score: Option<f64>,
    pub score_position: ScorePosition,
}

impl Serialize for VectorSearchHit {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let score = self.score.as_ref().map(metadata_value).transpose()?;
        match self.score_position {
            // The document's own `_score` replaces the engine's
            ScorePosition::First => {
                let leading = self.document.get("_score").or(score.as_ref());
                serialize_hit(
                    serializer,
                    leading.map(|v| ("_score", v)),
                    &self.document,
                    &[],
                )
            }
            ScorePosition::Last => {
                serialize_hit(serializer, None, &self.document, &[("_score", score)])
            }
        }
    }
}

impl<'de> Deserialize<'de> for VectorSearchHit {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut document = Map::<String, Value>::deserialize(deserializer)?;
        if !document.contains_key("_score") {
            return Err(D::Error::missing_field("_score"));
        }
        let score_position = match document.keys().next() {
            Some(first) if first == "_score" => ScorePosition::First,
            _ => ScorePosition::Last,
        };
        // The engine's score is a float: an integer, string, `null`, … is a
        // document's own `_score` (or a NaN score)
        let [score] = take_metadata(&mut document, ["_score"], |_, v| v.is_f64());
        let score = score.and_then(|v| v.as_f64());
        if score_position == ScorePosition::First && score.is_some() {
            document.shift_remove("_score");
        }
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
///
/// A document field named `relevance` or `passages` stands among the fields on
/// the wire, with the engine's value; it stays in `fields`, marking the
/// position (see the section notes on document fields named like the metadata).
/// (De)serialized by hand for that.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchDocument {
    pub doc_id: Value,
    /// Document-level fields (those shared by every chunk, or the single
    /// chunk's), without the body text and the engine / chunk bookkeeping
    pub fields: Map<String, Value>,
    /// The document's best fused score (higher is more relevant; a rank-fusion
    /// score, not a probability)
    pub relevance: f64,
    /// Its passages in relevance order; never empty
    pub passages: Vec<SearchPassage>,
}

impl Serialize for SearchDocument {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // A `doc_id` in the fields replaces the parent's (see above)
        let doc_id = self.fields.get("doc_id").unwrap_or(&self.doc_id);
        let metadata = [
            ("relevance", Some(metadata_value(&self.relevance)?)),
            ("passages", Some(metadata_value(&self.passages)?)),
        ];
        serialize_hit(
            serializer,
            Some(("doc_id", doc_id)),
            &self.fields,
            &metadata,
        )
    }
}

impl<'de> Deserialize<'de> for SearchDocument {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut fields = Map::<String, Value>::deserialize(deserializer)?;
        let doc_id = fields
            .shift_remove("doc_id")
            .ok_or_else(|| D::Error::missing_field("doc_id"))?;
        let [relevance, passages] =
            take_metadata(&mut fields, ["relevance", "passages"], |_, _| true);
        Ok(Self {
            doc_id,
            relevance: required_metadata(relevance, "relevance")?,
            passages: required_metadata(passages, "passages")?,
            fields,
        })
    }
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

// ============================================================================
// Shared helpers
// ============================================================================

/// A JSON boolean that is always `B` on the wire. It tells the variants of an
/// untagged response apart by the value of a flag field (`enabled`, `found`,
/// `success`), the way the server's shapes differ: a `false` does not parse
/// as `Flag<true>`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Flag<const B: bool>;

impl<const B: bool> Serialize for Flag<B> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bool(B)
    }
}

impl<'de, const B: bool> Deserialize<'de> for Flag<B> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        if bool::deserialize(deserializer)? == B {
            Ok(Flag)
        } else {
            Err(D::Error::custom(format_args!("expected `{B}`")))
        }
    }
}

// ============================================================================
// Embedding
// ============================================================================

/// `embed_text` response
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmbedTextResponse {
    /// The text's embedding
    pub vector: Vec<f32>,
    /// `vector.len()`
    pub dimension: usize,
    /// The provider that embedded the text (the default one when none was given)
    pub provider: String,
    /// The provider's model name (empty when the provider is gone)
    pub model: String,
}

/// `embed_batch` response
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmbedBatchResponse {
    /// One embedding per text, in the request's order
    pub vectors: Vec<Vec<f32>>,
    /// `vectors.len()`
    pub count: usize,
    /// Length of the first vector (`0` when there is none)
    pub dimension: usize,
    pub provider: String,
    pub model: String,
}

/// An embedding provider the server has configured
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelInfo {
    /// The name the provider is registered (and requested) under
    pub provider: String,
    /// The provider's model name
    pub model: String,
    /// Dimension of the vectors it returns
    pub dimension: usize,
    pub description: String,
    /// Always `true` (only configured providers are listed)
    pub available: bool,
}

/// `embed_models_list` response
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmbedModelsListResponse {
    /// The configured providers, in the server's (unspecified) order
    pub models: Vec<ModelInfo>,
    /// `models.len()`
    pub count: usize,
    /// The provider used when a request names none (empty when none is configured)
    pub default_provider: String,
}

/// Statistics of the server's embedding cache
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CacheStats {
    /// Number of lookups that found a vector
    pub hits: u64,
    /// Number of lookups that found none (absent or expired)
    pub misses: u64,
    /// Current number of entries
    pub entries: usize,
    /// Capacity (least recently used entries are evicted beyond it)
    pub max_entries: usize,
    /// Approximate memory used by the entries, in bytes
    pub bytes_used: usize,
    /// `hits / (hits + misses)` as a percentage (0–100); `0` before any lookup
    pub hit_rate: f64,
}

/// `embed_cache_stats` response
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum EmbedCacheStatsResponse {
    Enabled {
        enabled: Flag<true>,
        stats: CacheStats,
    },
    /// The server runs without an embedding cache
    Disabled {
        enabled: Flag<false>,
        message: String,
    },
}

/// `embed_cache_clear` response
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum EmbedCacheClearResponse {
    Cleared {
        success: Flag<true>,
        /// Number of entries the cache held
        entries_cleared: usize,
        message: String,
    },
    /// The server runs without an embedding cache: nothing was cleared
    Disabled {
        success: Flag<false>,
        message: String,
    },
}

// ============================================================================
// Auto-embedding
// ============================================================================

/// Chunking of an auto-embedded source field
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutoEmbedChunking {
    pub mode: String,
    pub chunk_size: usize,
    pub overlap: usize,
}

/// A collection's auto-embedding configuration, as `auto_embed_status`
/// reports it (`auto_embed_enable` adds `enabled` in front of it)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutoEmbedConfigView {
    /// The text field that is embedded
    pub source_field: String,
    /// The field the vector is written into
    pub target_field: String,
    pub provider: String,
    /// The model of the stored vectors (`null` for a legacy configuration;
    /// during a re-embed still the previous model)
    pub model: Option<String>,
    /// Vector dimension (`null` for a legacy configuration)
    pub dimension: Option<usize>,
    /// Documents that already have a vector are not re-embedded on insert
    pub skip_if_exists: bool,
    /// `null`: the whole source field is embedded as one text
    pub chunking: Option<AutoEmbedChunking>,
}

/// The configuration `auto_embed_enable` stored
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutoEmbedEnabledConfig {
    /// Always `true`
    pub enabled: bool,
    #[serde(flatten)]
    pub config: AutoEmbedConfigView,
}

/// `auto_embed_enable` response
///
/// Serialized by hand: the server writes `message` right after `reembed` for a
/// re-embed (before `backfill_job_id`), and after `backfill_job_id` for a
/// plain backfill.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct AutoEmbedEnableResponse {
    pub success: bool,
    pub collection: String,
    pub config: AutoEmbedEnabledConfig,
    /// Number of documents to embed: those without a vector, or every one
    /// for a re-embed
    pub existing_documents: u64,
    /// `true` when the vectors already stored came from another provider,
    /// model, preprocessing, dimension or source field and are all being
    /// re-embedded; absent otherwise
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reembed: Option<bool>,
    /// The job embedding the existing documents (`embed_job_status`); absent
    /// when none was started (nothing to embed, or no job manager / capacity)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backfill_job_id: Option<String>,
    /// What was started (re-embed or backfill); absent when nothing was
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Set when the collection holds chunk documents, whose import-time
    /// vectors the verbatim auto-embedding will not reproduce
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

impl Serialize for AutoEmbedEnableResponse {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("success", &self.success)?;
        map.serialize_entry("collection", &self.collection)?;
        map.serialize_entry("config", &self.config)?;
        map.serialize_entry("existing_documents", &self.existing_documents)?;
        if let Some(reembed) = &self.reembed {
            map.serialize_entry("reembed", reembed)?;
        }
        let message_first = self.reembed.is_some();
        if message_first {
            if let Some(message) = &self.message {
                map.serialize_entry("message", message)?;
            }
        }
        if let Some(job_id) = &self.backfill_job_id {
            map.serialize_entry("backfill_job_id", job_id)?;
        }
        if !message_first {
            if let Some(message) = &self.message {
                map.serialize_entry("message", message)?;
            }
        }
        if let Some(warning) = &self.warning {
            map.serialize_entry("warning", warning)?;
        }
        map.end()
    }
}

/// `auto_embed_disable` response
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutoEmbedDisableResponse {
    pub success: bool,
    pub collection: String,
    pub message: String,
}

/// `auto_embed_status` response
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AutoEmbedStatusResponse {
    Configured {
        collection: String,
        enabled: bool,
        /// Boxed: the variant is much larger than [`Self::NotConfigured`]
        config: Box<AutoEmbedConfigView>,
        /// A re-embed into a new embedding space has not finished yet
        /// (restarted at every server start until it has)
        reembed_pending: bool,
        /// The shadow field a running re-embed writes into (`<target>__next`)
        reembed_field: Option<String>,
        /// `true` when the configured provider was removed from the server
        #[serde(default, skip_serializing_if = "Option::is_none")]
        requires_reconfiguration: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reconfiguration_reason: Option<String>,
    },
    /// The collection has no auto-embedding configuration (or no collection
    /// of that name exists)
    NotConfigured {
        collection: String,
        enabled: Flag<false>,
        /// Always `null`
        config: (),
    },
}

// ============================================================================
// Embedding jobs
// ============================================================================

/// Identifier of a background job (`job_<n>`)
pub type JobId = String;

/// State of a background job; `status` names the state on the wire, next to
/// its fields
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "status")]
pub enum JobStatus {
    /// Job is waiting to be processed
    #[serde(rename = "pending")]
    Pending,

    /// Job is currently running
    #[serde(rename = "running")]
    Running {
        /// Progress percentage (0.0 - 100.0)
        progress: f64,
        /// Current status message
        message: String,
    },

    /// Job completed successfully
    #[serde(rename = "completed")]
    Completed {
        /// Job result
        result: Value,
    },

    /// Job failed with error
    #[serde(rename = "failed")]
    Failed {
        /// Error message
        error: String,
    },

    /// Job was cancelled
    #[serde(rename = "cancelled")]
    Cancelled,
}

impl JobStatus {
    /// Check if job is in a terminal state
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            JobStatus::Completed { .. } | JobStatus::Failed { .. } | JobStatus::Cancelled
        )
    }

    /// Check if job is running
    pub fn is_running(&self) -> bool {
        matches!(self, JobStatus::Running { .. })
    }
}

/// Kind of a background job (externally tagged: a unit kind is a bare
/// string, e.g. `"Compact"`; the others an object under their name)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum JobType {
    /// Backfill embeddings for existing documents
    EmbedBackfill {
        collection: String,
        provider: String,
    },
    /// Import and embed a document with chunking
    EmbedDocument { collection: String, doc_id: String },
    /// Database compaction (background, non-blocking)
    Compact,
    /// Custom job type
    Custom { name: String },
}

/// A background job, as the job tools report it
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobInfo {
    /// Unique job identifier
    pub id: JobId,
    /// Job type
    pub job_type: JobType,
    /// Current status
    pub status: JobStatus,
    /// Seconds since the job was created (measured when it is reported)
    pub created_at_secs: u64,
    /// Seconds since the job was last updated (measured when it is reported)
    pub updated_at_secs: u64,
    /// Items processed so far
    pub items_processed: usize,
    /// Total items to process (if known)
    pub items_total: Option<usize>,
}

/// `embed_job_list` response
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmbedJobListResponse {
    /// The jobs, in the server's (unspecified) order; with `active_only`, the
    /// running ones
    pub jobs: Vec<JobInfo>,
    /// `jobs.len()`
    pub count: usize,
    /// Number of running jobs (whether listed or not)
    pub active_count: usize,
}

/// `embed_job_status` response
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum EmbedJobStatusResponse {
    Found {
        found: Flag<true>,
        job: JobInfo,
    },
    /// No job of that id (never existed, or removed after it finished)
    NotFound {
        found: Flag<false>,
        message: String,
    },
}

/// `embed_job_cancel` response
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmbedJobCancelResponse {
    /// `false` when the job is unknown or already finished
    pub success: bool,
    pub job_id: String,
    pub message: String,
    /// The finished job's state; only when it was already terminal
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<JobStatus>,
}

// ============================================================================
// RAG
// ============================================================================

/// The configuration `rag_collection_create` stored
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RagCreatedConfig {
    pub embedding_field: String,
    /// The primary text field
    pub text_field: String,
    /// Every field with a fulltext index (the primary one included)
    pub text_fields: Vec<String>,
    /// Document fields prepended to each chunk's embedding text
    pub context_fields: Vec<String>,
    pub provider: String,
    /// Fulltext language (`none`: no stemming)
    pub language: String,
    pub dimension: usize,
}

/// What `rag_collection_create` created; an existing collection or index is
/// kept (`false` / not counted)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RagCreatedIndexes {
    pub collection_created: bool,
    pub vector_created: bool,
    /// Number of fulltext indexes created (one per new text field)
    pub fulltext_created: usize,
}

/// `rag_collection_create` response
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RagCollectionCreateResponse {
    pub success: bool,
    pub collection: String,
    pub config: RagCreatedConfig,
    pub indexes: RagCreatedIndexes,
}

/// A chunked import skipped because the `doc_id` already has chunks
/// (`if_exists: skip`)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChunkImportSkipped {
    pub success: bool,
    pub collection: String,
    pub doc_id: String,
    /// Always `0`
    pub chunks_created: usize,
    pub skipped: Flag<true>,
    pub message: String,
}

/// A chunked import whose content produced no chunk (e.g. whitespace only)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChunkImportEmpty {
    pub success: bool,
    pub collection: String,
    /// Always `0`
    pub chunks_created: usize,
    pub message: String,
}

/// A document `rag_document_import` chunked, embedded and stored
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RagImported {
    pub success: bool,
    pub collection: String,
    /// The given `doc_id`, or a generated one
    pub doc_id: String,
    pub chunks_created: usize,
    /// Chunks of the same `doc_id` removed first (`if_exists: replace`)
    pub chunks_replaced: u64,
    pub dimension: usize,
    /// The provider that embedded the chunks
    pub provider: String,
    /// `true` when the request's `language` differs from the collection's
    /// existing fulltext index, which keeps its own
    pub language_ignored: bool,
}

/// `rag_document_import` response
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RagDocumentImportResponse {
    Imported(RagImported),
    Skipped(ChunkImportSkipped),
    Empty(ChunkImportEmpty),
}

/// A document `embed_document` chunked, embedded and stored
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmbedDocumentImported {
    pub success: bool,
    pub collection: String,
    /// The given `doc_id`, or a generated one
    pub doc_id: String,
    pub chunks_created: usize,
    /// Chunks of the same `doc_id` removed first (`if_exists: replace`)
    pub chunks_replaced: u64,
    pub dimension: usize,
    /// The request's `provider` (`null` when the default one was used)
    pub provider: Option<String>,
    /// Whether the request asked for the vector index (an existing one is kept)
    pub vector_index_created: bool,
    /// `_id`s of the inserted chunk documents
    pub inserted_ids: Vec<Value>,
}

/// `embed_document` response
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum EmbedDocumentResponse {
    Embedded(EmbedDocumentImported),
    Skipped(ChunkImportSkipped),
    Empty(ChunkImportEmpty),
}

/// A collection's stored RAG configuration, as `rag_collection_stats`
/// reports it
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RagConfigView {
    pub embedding_field: String,
    pub text_field: String,
    /// The fields searched by fulltext (`[text_field]` for a legacy configuration)
    pub text_fields: Vec<String>,
    pub provider: String,
    pub language: String,
    pub dimension: usize,
    /// When the configuration was stored (RFC 3339)
    pub created_at: String,
}

/// Contents and indexes of a RAG collection
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RagStats {
    /// Number of documents (chunks) in the collection
    pub chunk_count: u64,
    /// Number of distinct `doc_id`s
    pub source_document_count: usize,
    pub vector_indexes: Vec<VectorIndexInfo>,
    pub fulltext_indexes: Vec<FulltextIndexInfo>,
}

/// The configured provider of a RAG collection, as the server has it
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RagProviderInfo {
    pub name: String,
    pub dimension: usize,
    pub model: String,
}

/// `rag_collection_stats` response
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RagCollectionStatsResponse {
    pub collection: String,
    /// The collection has a stored RAG configuration (`rag_collection_create`,
    /// or the first `rag_document_import`)
    pub rag_enabled: bool,
    /// `null` without a RAG configuration
    pub config: Option<RagConfigView>,
    pub stats: RagStats,
    /// `null` without a RAG configuration or when its provider is not
    /// available on the server
    pub provider_info: Option<RagProviderInfo>,
}

/// `rag_chunks_load` response
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RagChunksLoadResponse {
    /// The chunk documents (adjacent ones merged unless `merge_chunks: false`);
    /// with a `query`, each carries its fulltext `_score`
    pub results: Vec<Value>,
    /// `results.len()`
    pub count: usize,
    /// Number of chunks folded into a neighbour by the merge
    pub chunks_merged: usize,
    /// `true` when a `query` scored the chunks
    pub scored: bool,
}
