//! Vector index and similarity search tool handlers

use crate::adapter::IronBaseAdapter;
use crate::error::{McpError, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

use super::defaults::{default_embedding_field, default_vector_limit, DEFAULT_MAX_VECTORS};
use super::helpers::{parse_projection_value, validate_collection_name};
use super::params::ParseParams;

// ============================================================================
// Parameter Structs
// ============================================================================

/// Parameters for the vector sub-route of `index_create` (type='vector')
#[derive(Debug, Deserialize)]
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
#[derive(Debug, Deserialize)]
pub struct VectorSearchParams {
    pub collection: String,
    /// Field with vector index (default: "embedding")
    #[serde(default = "default_embedding_field")]
    pub field: String,
    pub vector: Vec<f64>,
    #[serde(default = "default_vector_limit")]
    pub limit: usize,
    pub projection: Option<Value>,
}

// ============================================================================
// Dispatch
// ============================================================================

/// Dispatch vector tool calls. Vector index lifecycle (create/list/drop/stats) is
/// routed through the generic `index_*` tools; only similarity search lives here.
pub fn dispatch(name: &str, params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    match name {
        "vector_search" => handle_vector_search(params, adapter),
        _ => Err(McpError::invalid_params(format!(
            "Unknown vector tool: {}",
            name
        ))),
    }
}

// ============================================================================
// Tool Handlers
// ============================================================================

/// Create a vector (HNSW) index. Invoked by the generic `index_create` tool when
/// `type: "vector"`.
pub(super) fn handle_index_create_vector(
    params: Value,
    adapter: &Arc<IronBaseAdapter>,
) -> Result<Value> {
    let p: VectorIndexCreateParams = VectorIndexCreateParams::parse(params)?;
    validate_collection_name(&p.collection)?;

    let name = adapter.create_vector_index(
        &p.collection,
        &p.field,
        p.dim,
        &p.metric,
        p.max_vectors,
        p.m,
        p.ef_construction,
        p.ef_search,
    )?;

    Ok(json!({
        "index_name": name,
        "field": p.field,
        "dim": p.dim,
        "metric": p.metric,
        "max_vectors": p.max_vectors,
        "hnsw_params": {
            "m": p.m,
            "ef_construction": p.ef_construction,
            "ef_search": p.ef_search
        }
    }))
}

fn handle_vector_search(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let p: VectorSearchParams = VectorSearchParams::parse(params)?;
    validate_collection_name(&p.collection)?;

    // Convert f64 to f32
    let query_vector: Vec<f32> = p.vector.iter().map(|&v| v as f32).collect();

    let projection = parse_projection_value(p.projection)?;
    let results = adapter.vector_search(&p.collection, &p.field, &query_vector, p.limit)?;

    // Apply projection if specified, or convert to simple format
    let results_json: Vec<Value> = if let Some(proj) = projection {
        apply_projection_to_results(results, &proj)
    } else {
        results_to_json(results)
    };

    Ok(json!({
        "results": results_json,
        "count": results_json.len()
    }))
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Convert vector search results to JSON array
fn results_to_json(results: Vec<(Value, f32)>) -> Vec<Value> {
    results
        .into_iter()
        .map(|(doc, score)| {
            let mut result = doc;
            if let Value::Object(ref mut obj) = result {
                obj.insert("_score".to_string(), json!(score));
            }
            result
        })
        .collect()
}

/// Apply projection to vector search results
/// Follows MongoDB projection semantics:
/// - In include mode: only include specified fields; _id is included by default unless explicitly excluded
/// - In exclude mode: exclude specified fields; _id is always included
fn apply_projection_to_results(
    results: Vec<(Value, f32)>,
    projection: &HashMap<String, i32>,
) -> Vec<Value> {
    let is_include_mode = projection.values().any(|&v| v == 1);
    // Check if _id is explicitly excluded ({"_id": 0})
    let id_explicitly_excluded = projection.get("_id").copied() == Some(0);

    results
        .into_iter()
        .map(|(doc, score)| {
            let mut result = json!({ "_score": score });
            if let Value::Object(obj) = doc {
                for (key, value) in obj {
                    let should_include = if is_include_mode {
                        // Include mode: only include specified fields
                        // _id is included by default UNLESS explicitly excluded with {"_id": 0}
                        if key == "_id" {
                            !id_explicitly_excluded
                        } else {
                            projection.get(&key).copied().unwrap_or(0) == 1
                        }
                    } else {
                        // Exclude mode: exclude specified fields, _id always included
                        projection.get(&key).copied().unwrap_or(1) != 0
                    };
                    if should_include {
                        result[&key] = value;
                    }
                }
            }
            result
        })
        .collect()
}
