//! Hybrid (vector + fulltext) search: the post-fusion steps on
//! [`FusedResult`]s and chunk documents — reranking, adjacent-chunk merge,
//! MMR diversity reordering — and their shared helpers.
//!
//! Moved from the MCP server (docs/MCP_TO_CORE_PLAN.md, point 3, step 1);
//! behavior unchanged.

use serde_json::{json, Value};
use std::collections::HashMap;

mod merge;
mod mmr;
mod rerank;
#[cfg(test)]
mod tests;

pub use merge::{
    merge_adjacent_chunks, merge_adjacent_values, MAX_MERGED_PASSAGE_CHARS, MERGE_BEST_OFFSET_KEY,
};
pub use mmr::mmr_reorder;
pub use rerank::rerank_results;

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
