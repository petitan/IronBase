//! Hybrid search tool handler (RRF-based fusion)
//!
//! Combines vector similarity and fulltext search using Reciprocal Rank Fusion (RRF).
//! RRF is collection-size independent, solving the score normalization problem between
//! TF-IDF (unbounded) and vector similarity (0-1).
//!
//! ## Design (2026-02)
//! - Unified tool: `hybrid_search` handles both explicit vector and auto-embed modes
//! - If `vector` is provided → explicit mode (client embeds query)
//! - If `vector` is omitted → auto-embed mode (server embeds query via RAG config/provider)
//! - `rag_search` is a deprecated alias that delegates here
//! - Reranking: phrase match, keyword density, title boost
//! - MMR diversity reranking: embedding cosine similarity based

use crate::adapter::{FulltextSearchOptions, IronBaseAdapter};
use crate::embedding::EmbeddingManager;
use crate::error::{McpError, Result};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use super::defaults::{DEFAULT_EMBEDDING_FIELD, DEFAULT_EMBEDDING_PROVIDER, DEFAULT_TEXT_FIELD};
use super::fusion::{
    apply_projection, id_to_string, merge_adjacent_chunks, mmr_reorder, rerank_results,
    FusedResult,
};
use super::helpers::{parse_projection_value, validate_collection_name};
use super::params::{resolve_weights, HybridSearchParams, ParseParams};
use super::rag::get_rag_config;

/// RRF default constant - empirically optimal value (Cormack et al., 2009)
/// Used by test helper; runtime value comes from HybridSearchParams.rrf_k
#[cfg(test)]
const RRF_K: f64 = 60.0;

/// Maximum internal limit to prevent OOM (CLAUDE.md compliance)
/// Even if user requests limit=100000, we cap internal processing at this value
const MAX_INTERNAL_LIMIT: usize = 1000;

/// Dispatch hybrid tool calls
pub fn dispatch(
    name: &str,
    params: Value,
    adapter: &Arc<IronBaseAdapter>,
    embedding_manager: &Option<Arc<EmbeddingManager>>,
) -> Result<Value> {
    match name {
        "hybrid_search" => handle_hybrid_search(params, adapter, embedding_manager),
        _ => Err(McpError::invalid_params(format!(
            "Unknown hybrid tool: {}",
            name
        ))),
    }
}

/// Handle hybrid_search - RRF fusion of vector and fulltext search
///
/// Two modes:
/// - **Explicit mode**: `vector` is provided → use it directly (client-embedded)
/// - **Auto-embed mode**: `vector` is omitted → embed query using RAG config or provider param
fn handle_hybrid_search(
    params: Value,
    adapter: &Arc<IronBaseAdapter>,
    embedding_manager: &Option<Arc<EmbeddingManager>>,
) -> Result<Value> {
    if params.get("dedup_threshold").is_some() {
        return Err(McpError::invalid_params(
            "Parameter 'dedup_threshold' has been removed. Use 'mmr_lambda' (0.0-1.0) instead.",
        ));
    }
    let p: HybridSearchParams = HybridSearchParams::parse(params)?;
    validate_collection_name(&p.collection)?;

    // Resolve weights: explicit overrides > search_mode preset > balanced default
    let (vector_weight, fulltext_weight) =
        resolve_weights(p.search_mode.as_deref(), p.vector_weight, p.fulltext_weight)?;

    // ========================================================================
    // STEP 1: Resolve vector + field names (explicit vs auto-embed mode)
    // ========================================================================
    let (query_vector, effective_vector_field, effective_text_field, auto_embedded, provider_name) =
        match p.vector {
            Some(ref v) => {
                // Explicit mode: client provided the vector
                let qv: Vec<f32> = v.iter().map(|&x| x as f32).collect();
                (qv, p.vector_field.clone(), p.text_field.clone(), false, None)
            }
            None => {
                // Auto-embed mode: server embeds the query
                if p.query.is_empty() {
                    return Err(McpError::invalid_params("Query cannot be empty"));
                }

                let manager = embedding_manager.as_ref().ok_or_else(|| {
                    McpError::internal(
                        "Embedding not available. Set IRONBASE_FASTTEXT_MODEL environment variable.",
                    )
                })?;

                // Get RAG config or use defaults
                let rag_config = get_rag_config(adapter, &p.collection)?;
                let (emb_field, txt_field, prov_name) = match &rag_config {
                    Some(cfg) => (
                        cfg.embedding_field.clone(),
                        cfg.text_field.clone(),
                        p.provider
                            .clone()
                            .unwrap_or_else(|| cfg.provider.clone()),
                    ),
                    None => {
                        // Auto-detect fulltext indexed field from collection metadata
                        let detected_text_field = adapter
                            .get_fulltext_field_names(&p.collection)
                            .ok()
                            .and_then(|fields| fields.into_iter().next())
                            .unwrap_or_else(|| DEFAULT_TEXT_FIELD.to_string());

                        if detected_text_field != DEFAULT_TEXT_FIELD {
                            tracing::info!(
                                "hybrid_search: auto-detected fulltext field '{}' for collection '{}' (no RAG config)",
                                detected_text_field, p.collection
                            );
                        }

                        (
                            DEFAULT_EMBEDDING_FIELD.to_string(),
                            detected_text_field,
                            p.provider
                                .clone()
                                .unwrap_or_else(|| DEFAULT_EMBEDDING_PROVIDER.to_string()),
                        )
                    }
                };

                // Embed the query
                let qv = manager
                    .embed(&p.query, Some(&prov_name))
                    .map_err(|e| McpError::internal(format!("Query embedding failed: {}", e)))?;

                // Use user-specified fields if provided, otherwise use resolved defaults
                let eff_vf = if p.vector_field != "embedding" {
                    p.vector_field.clone() // User explicitly set vector_field
                } else {
                    emb_field
                };
                let eff_tf = if p.text_field != "content" {
                    p.text_field.clone() // User explicitly set text_field
                } else {
                    txt_field
                };

                (qv, eff_vf, eff_tf, true, Some(prov_name))
            }
        };

    // Internal limit multiplier for better fusion coverage
    // Need more results when reranking/deduplication will filter some out
    // Cap at MAX_INTERNAL_LIMIT to prevent OOM (CLAUDE.md compliance)
    let internal_limit = (p.limit * 3).min(MAX_INTERNAL_LIMIT);

    // ========================================================================
    // STEP 2: Vector search → get ranks
    // ========================================================================
    let vector_results = if let Some(ref filter) = p.filter {
        adapter.vector_search_with_filter(
            &p.collection,
            &effective_vector_field,
            &query_vector,
            filter,
            internal_limit,
        )?
    } else {
        adapter.vector_search(
            &p.collection,
            &effective_vector_field,
            &query_vector,
            internal_limit,
        )?
    };

    // Build vector rank map (1-indexed) with pre-allocated capacity (OOM protection)
    let mut vector_ranks: HashMap<String, usize> = HashMap::with_capacity(vector_results.len());
    for (rank, (doc, _score)) in vector_results.iter().enumerate() {
        if let Some(id) = doc.get("_id").and_then(id_to_string) {
            vector_ranks.insert(id, rank + 1);
        }
    }

    // Store vector docs for later retrieval with pre-allocated capacity
    let mut vector_docs: HashMap<String, (Value, f32)> =
        HashMap::with_capacity(vector_results.len());
    for (doc, score) in vector_results.into_iter() {
        if let Some(id) = doc.get("_id").and_then(id_to_string) {
            vector_docs.insert(id, (doc, score));
        }
    }

    // ========================================================================
    // STEP 3: Fulltext search → get ranks (original query - Snowball handles stemming)
    // ========================================================================
    let text_options = FulltextSearchOptions {
        limit: Some(internal_limit),
        skip: None,
        min_score: None,
        projection: None, // Get full docs for merging
        filter: p.filter.clone(),
        and_mode: p.mode.as_deref() == Some("and"),
        highlight: false,
        highlight_context: None,
        highlight_max_snippets: None,
    };

    // Multi-field search if `text_fields` is provided, otherwise single-field
    let text_results = if let Some(ref fields) = p.text_fields {
        let field_refs: Vec<&str> = fields.iter().map(|s| s.as_str()).collect();
        adapter.fulltext_search_multi(&p.collection, &field_refs, &p.query, text_options)?
    } else {
        adapter.fulltext_search(&p.collection, &effective_text_field, &p.query, text_options)?
    };

    // Build fulltext rank map (1-indexed) with pre-allocated capacity (OOM protection)
    let mut text_ranks: HashMap<String, usize> = HashMap::with_capacity(text_results.len());
    for (rank, res) in text_results.iter().enumerate() {
        if let Some(id) = res.document.get("_id").and_then(id_to_string) {
            text_ranks.insert(id, rank + 1);
        }
    }

    // Store fulltext docs for later retrieval with pre-allocated capacity
    let mut text_docs: HashMap<String, (Value, f64)> = HashMap::with_capacity(text_results.len());
    for res in text_results.into_iter() {
        if let Some(id) = res.document.get("_id").and_then(id_to_string) {
            text_docs.insert(id, (res.document, res.score));
        }
    }

    // ========================================================================
    // STEP 4: RRF Fusion (with pre-allocated capacity for OOM protection)
    // ========================================================================
    let max_ids = vector_ranks.len() + text_ranks.len();
    let mut all_ids: HashSet<String> = HashSet::with_capacity(max_ids);
    all_ids.extend(vector_ranks.keys().cloned());
    all_ids.extend(text_ranks.keys().cloned());

    let default_rank = internal_limit + 1;

    // Pre-allocate fused results vector
    let mut fused: Vec<FusedResult> = Vec::with_capacity(all_ids.len());
    for id in all_ids.iter() {
        let v_rank = *vector_ranks.get(id).unwrap_or(&default_rank);
        let t_rank = *text_ranks.get(id).unwrap_or(&default_rank);

        // RRF score formula: weight * 1/(k + rank)
        let rrf_score = vector_weight * (1.0 / (p.rrf_k + v_rank as f64))
            + fulltext_weight * (1.0 / (p.rrf_k + t_rank as f64));

        let v_score = vector_docs.get(id).map(|(_, s)| *s);
        let t_score = text_docs.get(id).map(|(_, s)| *s);

        // Get document from either source (prefer vector for consistency)
        let doc = match vector_docs
            .get(id)
            .map(|(d, _)| d.clone())
            .or_else(|| text_docs.get(id).map(|(d, _)| d.clone()))
        {
            Some(d) => d,
            None => continue, // Skip if no doc found (shouldn't happen)
        };

        fused.push(FusedResult {
            id: id.clone(),
            doc,
            rrf_score,
            final_score: rrf_score, // Will be updated by reranking
            rerank_boost: 1.0,
            v_rank,
            t_rank,
            v_score,
            t_score,
        });
    }

    // Sort by RRF score initially
    fused.sort_by(|a, b| {
        b.rrf_score
            .partial_cmp(&a.rrf_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // ========================================================================
    // STEP 5: Reranking (optional)
    // ========================================================================
    if p.rerank {
        rerank_results(
            &mut fused,
            &p.query,
            &effective_text_field,
            p.title_field.as_deref(),
        );
    }

    // ========================================================================
    // STEP 5.5: Merge adjacent chunks from same document (overlap dedup)
    // ========================================================================
    let chunks_merged = if p.merge_chunks {
        merge_adjacent_chunks(&mut fused, &effective_text_field)
    } else {
        0
    };

    // ========================================================================
    // STEP 6: MMR diversity reranking (optional)
    // ========================================================================
    let dedup_removed = if p.deduplicate {
        mmr_reorder(
            &mut fused,
            &effective_vector_field,
            p.mmr_lambda,
            p.limit,
        )
    } else {
        fused.truncate(p.limit);
        0
    };

    // ========================================================================
    // STEP 7: Apply projection and build response
    // ========================================================================
    let projection = parse_projection_value(p.projection)?;

    // Pre-allocate with try_reserve for OOM protection
    let mut results: Vec<Value> = Vec::new();
    results.try_reserve(fused.len()).map_err(|e| {
        McpError::internal(format!(
            "OOM: cannot allocate {} results: {}",
            fused.len(),
            e
        ))
    })?;

    for item in fused {
        let doc_projected = if let Some(ref proj) = projection {
            apply_projection(&item.doc, proj)
        } else {
            item.doc
        };

        let mut result = doc_projected;
        if let Value::Object(ref mut obj) = result {
            obj.insert("_rrf_score".to_string(), json!(item.rrf_score));
            obj.insert("_final_score".to_string(), json!(item.final_score));
            obj.insert("_rerank_boost".to_string(), json!(item.rerank_boost));
            obj.insert("_vector_rank".to_string(), json!(item.v_rank));
            obj.insert("_text_rank".to_string(), json!(item.t_rank));
            if let Some(vs) = item.v_score {
                obj.insert("_vector_score".to_string(), json!(vs));
            }
            if let Some(ts) = item.t_score {
                obj.insert("_text_score".to_string(), json!(ts));
            }
        }

        results.push(result);
    }

    Ok(json!({
        "results": results,
        "count": results.len(),
        "algorithm": "rrf",
        "rrf_k": p.rrf_k,
        "weights": {
            "vector": vector_weight,
            "fulltext": fulltext_weight
        },
        "search_mode": p.search_mode.as_deref().unwrap_or("balanced"),
        "query": p.query,
        "auto_embedded": auto_embedded,
        "provider": provider_name,
        "dedup_removed": dedup_removed,
        "chunks_merged": chunks_merged
    }))
}

/// Calculate RRF score for given ranks and weights
/// Exposed for testing
#[cfg(test)]
fn calculate_rrf_score(v_rank: usize, t_rank: usize, v_weight: f64, t_weight: f64) -> f64 {
    v_weight * (1.0 / (RRF_K + v_rank as f64)) + t_weight * (1.0 / (RRF_K + t_rank as f64))
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // -------------------------------------------------------------------------
    // RRF Score Calculation Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_rrf_score_equal_ranks_equal_weights() {
        // Same rank in both → symmetric score
        let score = calculate_rrf_score(1, 1, 0.5, 0.5);
        // Expected: 0.5 * 1/(60+1) + 0.5 * 1/(60+1) = 1/(61) ≈ 0.01639
        assert!((score - 1.0 / 61.0).abs() < 1e-10);
    }

    #[test]
    fn test_rrf_score_different_ranks_equal_weights() {
        // v_rank=1, t_rank=10, equal weights
        let score = calculate_rrf_score(1, 10, 0.5, 0.5);
        // Expected: 0.5 * 1/61 + 0.5 * 1/70
        let expected = 0.5 / 61.0 + 0.5 / 70.0;
        assert!((score - expected).abs() < 1e-10);
    }

    #[test]
    fn test_rrf_score_weighted_vector_heavy() {
        // v_rank=1, t_rank=10, vector_weight=0.8
        let score = calculate_rrf_score(1, 10, 0.8, 0.2);
        let expected = 0.8 / 61.0 + 0.2 / 70.0;
        assert!((score - expected).abs() < 1e-10);
    }

    #[test]
    fn test_rrf_score_weighted_text_heavy() {
        // v_rank=10, t_rank=1, text_weight=0.8
        let score = calculate_rrf_score(10, 1, 0.2, 0.8);
        let expected = 0.2 / 70.0 + 0.8 / 61.0;
        assert!((score - expected).abs() < 1e-10);
    }

    #[test]
    fn test_rrf_score_symmetry() {
        // With equal weights, swapping ranks should give same score
        let score1 = calculate_rrf_score(1, 10, 0.5, 0.5);
        let score2 = calculate_rrf_score(10, 1, 0.5, 0.5);
        assert!((score1 - score2).abs() < 1e-10);
    }

    #[test]
    fn test_rrf_score_ordering() {
        // Better ranks should give higher scores
        let score_best = calculate_rrf_score(1, 1, 0.5, 0.5);
        let score_mid = calculate_rrf_score(5, 5, 0.5, 0.5);
        let score_worst = calculate_rrf_score(100, 100, 0.5, 0.5);

        assert!(score_best > score_mid);
        assert!(score_mid > score_worst);
    }

    #[test]
    fn test_rrf_k_constant() {
        // Verify RRF_K is 60 (empirically optimal for test scoring)
        assert_eq!(RRF_K, 60.0);
    }

    // -------------------------------------------------------------------------
    // HybridSearchParams Parsing Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_params_full() {
        let params = json!({
            "collection": "test",
            "vector_field": "embedding",
            "text_field": "content",
            "vector": [0.1, 0.2, 0.3],
            "query": "test query",
            "limit": 20,
            "vector_weight": 0.7,
            "fulltext_weight": 0.3
        });

        let p: HybridSearchParams = HybridSearchParams::parse(params).unwrap();
        assert_eq!(p.collection, "test");
        assert_eq!(p.limit, 20);
        assert_eq!(p.vector_weight, Some(0.7));
        assert_eq!(p.fulltext_weight, Some(0.3));
    }

    #[test]
    fn test_params_defaults() {
        let params = json!({
            "collection": "test",
            "vector_field": "embedding",
            "text_field": "content",
            "vector": [0.1, 0.2],
            "query": "test"
        });

        let p: HybridSearchParams = HybridSearchParams::parse(params).unwrap();
        assert_eq!(p.limit, 10); // default
        assert!(p.vector_weight.is_none()); // no explicit weight
        assert!(p.fulltext_weight.is_none()); // no explicit weight
        assert!(p.search_mode.is_none()); // no mode → balanced default
        // v2 defaults
        assert!(p.rerank); // default: true
        assert!(p.deduplicate); // default: true
        assert!((p.mmr_lambda - 0.5).abs() < f64::EPSILON); // default
        assert!(p.provider.is_none()); // no provider → use collection config
        assert!(p.filter.is_none()); // default: no filter
        assert!(p.mode.is_none()); // default: None (= "or")
        assert!(p.text_fields.is_none()); // default: None (= single text_field)
        assert!(p.vector.is_some()); // explicit vector provided
    }

    #[test]
    fn test_params_auto_embed_mode() {
        // No vector → auto-embed mode (only collection + query required)
        let params = json!({
            "collection": "docs",
            "query": "semantic search test"
        });
        let p: HybridSearchParams = HybridSearchParams::parse(params).unwrap();
        assert_eq!(p.collection, "docs");
        assert!(p.vector.is_none());
        assert!(p.provider.is_none());
    }

    #[test]
    fn test_params_with_mode_and() {
        let params = json!({
            "collection": "test",
            "vector": [0.1, 0.2],
            "query": "ajánlat karbantartás",
            "mode": "and"
        });
        let p: HybridSearchParams = HybridSearchParams::parse(params).unwrap();
        assert_eq!(p.mode.as_deref(), Some("and"));
    }

    #[test]
    fn test_params_with_mode_or() {
        let params = json!({
            "collection": "test",
            "vector": [0.1, 0.2],
            "query": "test",
            "mode": "or"
        });
        let p: HybridSearchParams = HybridSearchParams::parse(params).unwrap();
        assert_eq!(p.mode.as_deref(), Some("or"));
    }

    #[test]
    fn test_params_with_text_fields() {
        let params = json!({
            "collection": "test",
            "vector": [0.1, 0.2],
            "query": "Juhai ajánlat",
            "text_fields": ["content_text", "title", "customer"]
        });
        let p: HybridSearchParams = HybridSearchParams::parse(params).unwrap();
        let fields = p.text_fields.unwrap();
        assert_eq!(fields, vec!["content_text", "title", "customer"]);
    }

    #[test]
    fn test_params_text_fields_overrides_text_field() {
        let params = json!({
            "collection": "test",
            "text_field": "content",
            "vector": [0.1, 0.2],
            "query": "test",
            "text_fields": ["title", "body"]
        });
        let p: HybridSearchParams = HybridSearchParams::parse(params).unwrap();
        assert_eq!(p.text_field, "content"); // still parsed
        assert!(p.text_fields.is_some()); // but text_fields takes priority in handler
        assert_eq!(p.text_fields.unwrap(), vec!["title", "body"]);
    }

    #[test]
    fn test_params_with_mode_and_text_fields_combined() {
        let params = json!({
            "collection": "test",
            "vector": [0.1, 0.2],
            "query": "ajánlat karbantartás",
            "mode": "and",
            "text_fields": ["content_text", "title"]
        });
        let p: HybridSearchParams = HybridSearchParams::parse(params).unwrap();
        assert_eq!(p.mode.as_deref(), Some("and"));
        assert_eq!(p.text_fields.unwrap(), vec!["content_text", "title"]);
    }

    #[test]
    fn test_params_v2_full() {
        let params = json!({
            "collection": "test",
            "vector_field": "embedding",
            "text_field": "content",
            "vector": [0.1, 0.2, 0.3],
            "query": "Milyen jellemzői vannak?",
            "limit": 10,
            "rerank": true,
            "deduplicate": true,
            "mmr_lambda": 0.7,
            "provider": "fasttext"
        });

        let p: HybridSearchParams = HybridSearchParams::parse(params).unwrap();
        assert_eq!(p.provider.as_deref(), Some("fasttext"));
        assert!(p.rerank);
        assert!(p.deduplicate);
        assert!((p.mmr_lambda - 0.7).abs() < f64::EPSILON);
    }

    #[test]
    fn test_params_v2_disable_features() {
        let params = json!({
            "collection": "test",
            "vector_field": "embedding",
            "text_field": "content",
            "vector": [0.1, 0.2],
            "query": "test",
            "rerank": false,
            "deduplicate": false
        });

        let p: HybridSearchParams = HybridSearchParams::parse(params).unwrap();
        assert!(!p.rerank);
        assert!(!p.deduplicate);
    }

    #[test]
    fn test_params_with_filter() {
        let params = json!({
            "collection": "test",
            "vector_field": "embedding",
            "text_field": "content",
            "vector": [0.1, 0.2, 0.3],
            "query": "test query",
            "filter": {"doc_type": "ajanlat", "status": "active"}
        });

        let p: HybridSearchParams = HybridSearchParams::parse(params).unwrap();
        assert!(p.filter.is_some());
        let filter = p.filter.unwrap();
        assert_eq!(filter["doc_type"], "ajanlat");
        assert_eq!(filter["status"], "active");
    }

    #[test]
    fn test_params_with_empty_filter() {
        let params = json!({
            "collection": "test",
            "vector_field": "embedding",
            "text_field": "content",
            "vector": [0.1, 0.2],
            "query": "test",
            "filter": {}
        });

        let p: HybridSearchParams = HybridSearchParams::parse(params).unwrap();
        assert!(p.filter.is_some());
        assert!(p.filter.unwrap().as_object().unwrap().is_empty());
    }

    #[test]
    fn test_params_missing_required_query() {
        // query is required
        let params = json!({
            "collection": "test",
            "vector_field": "embedding"
        });

        let result = HybridSearchParams::parse(params);
        assert!(result.is_err());
    }

    // -------------------------------------------------------------------------
    // RRF K Configurable Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_rrf_k_score_spread() {
        // K=20 should give wider spread than K=60
        let k20_rank1 = 0.5 * (1.0 / (20.0 + 1.0)) + 0.5 * (1.0 / (20.0 + 1.0));
        let k20_rank10 = 0.5 * (1.0 / (20.0 + 10.0)) + 0.5 * (1.0 / (20.0 + 10.0));
        let k20_spread = k20_rank1 - k20_rank10;

        let k60_rank1 = 0.5 * (1.0 / (60.0 + 1.0)) + 0.5 * (1.0 / (60.0 + 1.0));
        let k60_rank10 = 0.5 * (1.0 / (60.0 + 10.0)) + 0.5 * (1.0 / (60.0 + 10.0));
        let k60_spread = k60_rank1 - k60_rank10;

        // K=20 spread should be significantly wider than K=60
        assert!(k20_spread > k60_spread * 5.0);
    }
}
