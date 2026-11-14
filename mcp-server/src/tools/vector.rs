//! Vector index and similarity search tool handlers

use crate::adapter::IronBaseAdapter;
use crate::error::{McpError, Result};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::sync::Arc;

use super::helpers::{
    document_fields, parse_projection_value, validate_collection_name, DEFAULT_QUERY_LIMIT,
};
use super::params::ParseParams;
use ironbase_mcp_types::responses::{
    HnswParams, IndexCreateResponse, ScorePosition, VectorIndexCreated, VectorSearchHit,
    VectorSearchResponse,
};

// Parameter structs: shared with clients, re-exported under their old path.
pub use ironbase_mcp_types::params::{VectorIndexCreateParams, VectorSearchParams};

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
    p: VectorIndexCreateParams,
    adapter: &Arc<IronBaseAdapter>,
) -> Result<IndexCreateResponse> {
    validate_collection_name(&p.collection)?;

    let index_name = adapter.create_vector_index(
        &p.collection,
        &p.field,
        p.dim,
        &p.metric,
        p.max_vectors,
        p.m,
        p.ef_construction,
        p.ef_search,
    )?;

    Ok(IndexCreateResponse::Vector(VectorIndexCreated {
        index_name,
        field: p.field,
        dim: p.dim,
        metric: p.metric,
        max_vectors: p.max_vectors,
        hnsw_params: HnswParams {
            m: p.m,
            ef_construction: p.ef_construction,
            ef_search: p.ef_search,
        },
    }))
}

fn handle_vector_search(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let p: VectorSearchParams = VectorSearchParams::parse(params)?;
    validate_collection_name(&p.collection)?;

    // Convert f64 to f32
    let query_vector: Vec<f32> = p.vector.iter().map(|&v| v as f32).collect();

    let projection = parse_projection_value(p.projection)?;
    // Bounded like find: a huge k walks the whole graph and loads every document
    let limit = p.limit.min(DEFAULT_QUERY_LIMIT);
    let results = adapter.vector_search(&p.collection, &p.field, &query_vector, limit)?;

    // Apply projection if specified, or convert to simple format
    let results = if let Some(proj) = projection {
        apply_projection_to_results(results, &proj)
    } else {
        results_to_hits(results)?
    };

    Ok(serde_json::to_value(VectorSearchResponse {
        count: results.len(),
        results,
    })?)
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Vector search results as hits: the document with `_score` appended
fn results_to_hits(results: Vec<(Value, f32)>) -> Result<Vec<VectorSearchHit>> {
    results
        .into_iter()
        .map(|(doc, score)| {
            Ok(VectorSearchHit {
                document: document_fields(doc)?,
                score: Some(f64::from(score)),
                score_position: ScorePosition::Last,
            })
        })
        .collect()
}

/// Apply projection to vector search results (`_score` first in each hit)
/// Follows MongoDB projection semantics:
/// - In include mode: only include specified fields; _id is included by default unless explicitly excluded
/// - In exclude mode: exclude specified fields; _id is always included
fn apply_projection_to_results(
    results: Vec<(Value, f32)>,
    projection: &HashMap<String, i32>,
) -> Vec<VectorSearchHit> {
    let is_include_mode = projection.values().any(|&v| v == 1);
    // Check if _id is explicitly excluded ({"_id": 0})
    let id_explicitly_excluded = projection.get("_id").copied() == Some(0);

    results
        .into_iter()
        .map(|(doc, score)| {
            let mut document = Map::new();
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
                        document.insert(key, value);
                    }
                }
            }
            VectorSearchHit {
                document,
                score: Some(f64::from(score)),
                score_position: ScorePosition::First,
            }
        })
        .collect()
}
