//! Hybrid (vector + fulltext) search: retrieval and Reciprocal Rank Fusion
//! ([`CollectionCore::hybrid_fuse`](crate::CollectionCore::hybrid_fuse)),
//! document grouping
//! ([`CollectionCore::hybrid_group_documents`](crate::CollectionCore::hybrid_group_documents))
//! and the post-fusion steps — reranking, adjacent-chunk merge, MMR diversity
//! reordering.
//!
//! The caller resolves what the query runs on: the query vector (embedding is
//! not the engine's job), the fields and the weights. Moved from the MCP
//! server (docs/MCP_TO_CORE_PLAN.md, point 3); behavior unchanged.

use serde_json::{json, Value};
use std::collections::HashMap;

mod fuse;
mod group;
mod merge;
mod mmr;
mod rerank;
#[cfg(test)]
mod tests;

pub use fuse::HybridFusion;
pub use group::{lift_common_fields, DocGroup};
pub use merge::{
    merge_adjacent_chunks, merge_adjacent_values, MAX_MERGED_PASSAGE_CHARS, MERGE_BEST_OFFSET_KEY,
};
pub use mmr::mmr_reorder;
pub use rerank::rerank_results;

/// The most candidates a lane returns (and the most chunks grouping re-reads)
pub const MAX_CANDIDATES: usize = 10_000;

/// A hybrid query with everything resolved: the query vector, the fields
/// and the lane weights.
#[derive(Debug, Clone)]
pub struct HybridQuery {
    /// Text for the fulltext lane and the reranking
    pub query: String,
    /// Query embedding; `None` runs the fulltext lane only
    pub vector: Option<Vec<f32>>,
    /// The documents' embedding field: read by MMR and named by projections
    pub vector_field: String,
    /// The vector index to search, when it is not `vector_field`'s (a shadow
    /// re-embed field)
    pub searched_vector_field: Option<String>,
    /// Fulltext field: searched unless `text_fields` is set; reranked and
    /// merged on
    pub text_field: String,
    /// Several fulltext fields searched together (best field per document)
    pub text_fields: Option<Vec<String>>,
    /// RRF weight of the vector lane
    pub vector_weight: f64,
    /// RRF weight of the fulltext lane
    pub fulltext_weight: f64,
    /// RRF constant: `weight / (rrf_k + rank)`
    pub rrf_k: f64,
    /// Results (flat) or documents (grouped)
    pub limit: usize,
    /// MongoDB-style filter applied to both lanes
    pub filter: Option<Value>,
    /// All query tokens must match
    pub and_mode: bool,
    /// With `and_mode`: the tokens may be spread over a document's chunks
    /// (`match_scope="document"`) instead of all in one chunk
    pub document_scope: bool,
    /// Rerank by phrase match, keyword density, length and title match
    pub rerank: bool,
    /// Field whose matches boost a result in the reranking
    pub title_field: Option<String>,
    /// At most this many chunks per document
    pub max_chunks_per_doc: Option<usize>,
    /// Merge adjacent chunks of a document
    pub merge_chunks: bool,
    /// MMR diversity reordering (flat mode)
    pub deduplicate: bool,
    /// MMR trade-off: 1.0 = relevance only, 0.0 = diversity only
    pub mmr_lambda: f64,
    /// Group the results by document (`doc_id`)
    pub group_by_document: bool,
    /// Projection of the returned documents (as [`apply_projection`]); the
    /// lanes leave a projected-out embedding unparsed
    pub projection: Option<HashMap<String, i32>>,
}

/// Intermediate result structure for RRF fusion pipeline processing
#[derive(Debug)]
pub struct FusedResult {
    /// Stable chunk id — also the deterministic tie-break key in the fusion sorts.
    pub id: String,
    pub doc: Value,
    pub rrf_score: f64,
    pub final_score: f64,
    pub rerank_boost: f64,
    /// 1-based rank in the vector / text lane; 0 = the lane did not return it
    pub v_rank: usize,
    pub t_rank: usize,
    pub v_score: Option<f32>,
    pub t_score: Option<f64>,
}

/// Convert Value _id to String for HashMap key
pub fn id_to_string(id: &Value) -> Option<String> {
    match id {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => Some(id.to_string()),
    }
}

/// Apply MongoDB-style projection to a document
pub fn apply_projection(doc: &Value, projection: &HashMap<String, i32>) -> Value {
    let is_include_mode = projection.values().any(|&v| v == 1);
    let id_explicitly_excluded = projection.get("_id").copied() == Some(0);

    let mut result = json!({});
    if let Value::Object(obj) = doc {
        for (key, value) in obj {
            let should_include = if is_include_mode {
                if key == "_id" {
                    !id_explicitly_excluded
                } else {
                    projection.get(key).copied().unwrap_or(0) == 1
                }
            } else {
                projection.get(key).copied().unwrap_or(1) != 0
            };
            if should_include {
                result[key] = value.clone();
            }
        }
    }
    result
}
