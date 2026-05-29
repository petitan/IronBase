//! `search` tool — intent-shaped façade over hybrid retrieval (Stage A).
//!
//! See `docs/HYBRID_RETRIEVAL_REDESIGN.md` and `docs/STAGE_A_IMPLEMENTATION_PLAN.md`.
//! Stage A is additive and low-risk: `search` reuses the *existing* RRF fusion via
//! `hybrid_search group_by_document=true` and reshapes the result into the compact,
//! document-anchored contract (`contract.rs`). No new retrieval primitive; the
//! mechanism surface (weights, rrf_k, match_scope, …) is NOT exposed.

use crate::adapter::IronBaseAdapter;
use crate::embedding::EmbeddingManager;
use crate::error::{McpError, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

use super::helpers::{parse_projection_value, validate_collection_name};
use super::hybrid::{build_doc_groups, retrieve_and_fuse};
use super::params::{deserialize_nonempty_filter, HybridSearchParams, ParseParams};

mod contract;

/// Mechanism parameters that `hybrid_search` exposes but `search` does NOT — the
/// intent-only surface owns these server-side. Passing one is rejected with a
/// pointer to the config (P6: not silently ignored). We reject this *known* set
/// rather than deny-all-unknown, so benign/protocol-injected fields don't break
/// `search` while every other tool tolerates them (serde ignores unknowns).
const REJECTED_MECHANISM_KEYS: &[&str] = &[
    "vector",
    "vector_field",
    "text_field",
    "text_fields",
    "vector_weight",
    "fulltext_weight",
    "search_mode",
    "rrf_k",
    "mode",
    "match_scope",
    "rerank",
    "deduplicate",
    "mmr_lambda",
    "title_field",
    "merge_chunks",
    "group_by_document",
    "max_chunks_per_doc",
    "provider",
    "projection",
];

#[derive(Debug, Deserialize)]
pub struct SearchParams {
    pub collection: String,
    pub query: String,
    /// Structured metadata filter (e.g. {"year": 2026}). Empty `{}` → None.
    #[serde(default, deserialize_with = "deserialize_nonempty_filter")]
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

/// Dispatch retrieval tool calls.
pub fn dispatch(
    name: &str,
    params: Value,
    adapter: &Arc<IronBaseAdapter>,
    embedding_manager: &Option<Arc<EmbeddingManager>>,
) -> Result<Value> {
    match name {
        "search" => handle_search(params, adapter, embedding_manager),
        _ => Err(McpError::invalid_params(format!(
            "Unknown retrieval tool: {}",
            name
        ))),
    }
}

fn handle_search(
    params: Value,
    adapter: &Arc<IronBaseAdapter>,
    embedding_manager: &Option<Arc<EmbeddingManager>>,
) -> Result<Value> {
    // Arguments must be a JSON object (give a clear error rather than a generic
    // serde type error for arrays/scalars).
    let Some(obj) = params.as_object() else {
        return Err(McpError::invalid_params(
            "`search` arguments must be a JSON object with at least {collection, query}.",
        ));
    };
    // Reject known mechanism parameters explicitly (P6 — not silently ignored).
    // Unknown/benign fields are tolerated (serde ignores them) so protocol- or
    // client-injected extras don't break `search` the way deny-all-unknown would.
    for key in obj.keys() {
        if REJECTED_MECHANISM_KEYS.contains(&key.as_str()) {
            return Err(McpError::invalid_params(format!(
                "Parameter '{}' is not accepted by `search` (intent-only surface). \
                 Retrieval mechanism (weights, rrf_k, match_scope, merge, caps, …) is \
                 server-owned. Use: collection, query, filter, limit, format, debug.",
                key
            )));
        }
    }

    let p: SearchParams = SearchParams::parse(params)?;
    validate_collection_name(&p.collection)?;
    if p.query.is_empty() {
        return Err(McpError::invalid_params("Query cannot be empty"));
    }
    if p.format != "structured" && p.format != "context_block" {
        return Err(McpError::invalid_params(format!(
            "Invalid format '{}'. Use: 'structured' or 'context_block'",
            p.format
        )));
    }

    // Graceful degradation (P7): without an embedding provider, semantic
    // retrieval is impossible — fall back to BM25-only and SURFACE it (P6),
    // never silently. The robust core (fulltext) still answers.
    let semantic_available = embedding_manager.is_some();

    // Build the internal hybrid params: document-grouped, embedding projected out
    // (keeps the intermediate small; the contract strips internals regardless).
    // Mechanism stays server-owned — the caller never sets these.
    let mut hp_map = serde_json::Map::new();
    hp_map.insert("collection".to_string(), json!(p.collection));
    hp_map.insert("query".to_string(), json!(p.query));
    if let Some(ref f) = p.filter {
        hp_map.insert("filter".to_string(), f.clone());
    }
    hp_map.insert("limit".to_string(), json!(p.limit));
    hp_map.insert("group_by_document".to_string(), json!(true));
    hp_map.insert("projection".to_string(), json!({ "embedding": 0 }));
    if !semantic_available {
        hp_map.insert("vector_weight".to_string(), json!(0.0));
        hp_map.insert("fulltext_weight".to_string(), json!(1.0));
    }
    let hp: HybridSearchParams = HybridSearchParams::parse(Value::Object(hp_map))?;

    // Run the SHARED pipeline directly (no JSON round-trip into hybrid_search):
    // retrieve+fuse → grouped builder → compact contract.
    let fo = retrieve_and_fuse(&hp, adapter, embedding_manager)?;
    let projection = parse_projection_value(Some(json!({ "embedding": 0 })))?;
    let (groups, total_chunks) = build_doc_groups(
        &fo.fused,
        adapter,
        &hp.collection,
        &hp.query,
        hp.limit,
        &hp.filter,
        hp.max_chunks_per_doc,
        &projection,
        &fo.effective_text_field,
        &fo.effective_text_fields,
    )?;

    let mut out = contract::assemble(
        &groups,
        total_chunks,
        fo.qual.qualified_doc_count,
        &fo.effective_text_field,
        &p.format,
        p.debug,
    )?;
    if !semantic_available {
        if let Value::Object(ref mut o) = out {
            o.insert(
                "degraded".to_string(),
                json!("semantic search unavailable (no embedding provider configured) — BM25-only"),
            );
        }
    }
    Ok(out)
}
