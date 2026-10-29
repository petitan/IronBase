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
//! - Reranking: phrase match, keyword density, title boost
//! - MMR diversity reranking: embedding cosine similarity based

use crate::adapter::IronBaseAdapter;
use crate::embedding::EmbeddingManager;
use crate::error::{McpError, Result};
use std::collections::HashMap;
use std::sync::Arc;

use super::defaults::{DEFAULT_EMBEDDING_FIELD, DEFAULT_TEXT_FIELD};
use ironbase_core::search::hybrid::{DocGroup, FusedResult, HybridQuery};
use ironbase_core::QualificationOutcome;

use super::fusion::{resolve_and_mode, validate_match_scope};
use super::helpers::parse_projection_value;
use super::params::{resolve_weights, HybridSearchParams};
use super::rag::get_rag_config;

/// RRF default constant - empirically optimal value (Cormack et al., 2009)
/// Used by test helper; runtime value comes from HybridSearchParams.rrf_k
#[cfg(test)]
const RRF_K: f64 = 60.0;

// MAX_INTERNAL_LIMIT imported from defaults.rs

/// Pick the chunk body / text field deterministically from a collection's
/// fulltext-indexed fields when there is no RAG config.
///
/// Prefers `content` (the RAG convention; `rag_document_import` always creates a
/// `content` FTS index) and otherwise falls back to the lexicographically-first
/// field. This MUST be deterministic: the result feeds `merge_adjacent_chunks`,
/// and `HashMap`-order nondeterminism previously let it resolve to `title`,
/// concatenating the title across merged chunks while leaving content unmerged
/// (issue #64).
pub(crate) fn pick_text_field(mut fields: Vec<String>) -> String {
    if fields.iter().any(|f| f == DEFAULT_TEXT_FIELD) {
        return DEFAULT_TEXT_FIELD.to_string();
    }
    fields.sort();
    fields
        .into_iter()
        .next()
        .unwrap_or_else(|| DEFAULT_TEXT_FIELD.to_string())
}

/// Output of the shared retrieve+fuse pipeline: the fused, reranked, merged
/// result set plus the resolved query and qualification the response builders
/// need. Shared by `hybrid_search` (flat + grouped) and the `search`
/// tool, so both run identical retrieval with no duplication or JSON round-trip.
pub(crate) struct FusionOutcome {
    pub collection: String,
    pub fused: Vec<FusedResult>,
    /// The resolved query the results were fused with: its `text_field` /
    /// `text_fields` are the effective fulltext fields
    pub query: HybridQuery,
    pub qual: QualificationOutcome,
}

/// Shared retrieve+fuse pipeline: resolves what the engine's hybrid search runs
/// on — the weights, the query vector (explicit, or embedded with the resolved
/// provider; none in BM25-only mode), the vector and text fields (RAG config,
/// fulltext indexes) — and runs `CollectionCore::hybrid_fuse` (document
/// qualification, both lanes, RRF, rerank, per-doc cap, merge, MMR).
pub(crate) fn retrieve_and_fuse(
    p: &HybridSearchParams,
    adapter: &Arc<IronBaseAdapter>,
    embedding_manager: &Option<Arc<EmbeddingManager>>,
) -> Result<FusionOutcome> {
    // Resolve weights: explicit overrides > search_mode preset > balanced default
    let (vector_weight, fulltext_weight) =
        resolve_weights(p.search_mode.as_deref(), p.vector_weight, p.fulltext_weight)?;

    // ========================================================================
    // STEP 1: Resolve vector + field names (explicit vs auto-embed mode)
    //         If vector_weight == 0.0 and no explicit vector → skip embedding
    //         entirely (BM25-only mode)
    // ========================================================================
    let skip_vector = vector_weight == 0.0 && p.vector.is_none();

    let (
        query_vector,
        effective_vector_field,
        effective_text_field,
        _auto_embedded,
        _provider_name,
    ) = if skip_vector {
        // BM25-only mode: resolve text field without requiring embedding
        if p.query.is_empty() {
            return Err(McpError::invalid_params("Query cannot be empty"));
        }

        let rag_config = get_rag_config(adapter, &p.collection)?;
        let txt_field = match &rag_config {
            Some(cfg) => cfg.text_field.clone(),
            None => adapter
                .get_fulltext_field_names(&p.collection)
                .ok()
                .map(pick_text_field)
                .unwrap_or_else(|| DEFAULT_TEXT_FIELD.to_string()),
        };

        let eff_tf = if p.text_field != DEFAULT_TEXT_FIELD {
            p.text_field.clone()
        } else {
            txt_field
        };

        (vec![], p.vector_field.clone(), eff_tf, false, None)
    } else {
        match &p.vector {
            Some(v) => {
                // Explicit mode: client provided the vector
                let qv: Vec<f32> = v.iter().map(|&x| x as f32).collect();
                (
                    qv,
                    p.vector_field.clone(),
                    p.text_field.clone(),
                    false,
                    None,
                )
            }
            None => {
                // Auto-embed mode: server embeds the query
                if p.query.is_empty() {
                    return Err(McpError::invalid_params("Query cannot be empty"));
                }

                let manager = embedding_manager.as_ref().ok_or_else(|| {
                    McpError::internal(
                        "Embedding not available. Configure an [embedding] section in config.toml.",
                    )
                })?;

                // Resolve provider: user explicit > AutoEmbeddingConfig > RAG config > manager default
                // Single DB lookup for auto config (no duplication across match arms)
                let auto_provider = adapter
                    .get_auto_embedding_config(&p.collection)
                    .ok()
                    .flatten()
                    .map(|c| c.provider);

                let rag_config = get_rag_config(adapter, &p.collection)?;
                let (emb_field, txt_field, prov_name) = match &rag_config {
                    Some(cfg) => (
                        cfg.embedding_field.clone(),
                        cfg.text_field.clone(),
                        p.provider
                            .clone()
                            .or(auto_provider)
                            .unwrap_or_else(|| cfg.provider.clone()),
                    ),
                    None => {
                        let detected_text_field = adapter
                            .get_fulltext_field_names(&p.collection)
                            .ok()
                            .map(pick_text_field)
                            .unwrap_or_else(|| DEFAULT_TEXT_FIELD.to_string());

                        (
                            DEFAULT_EMBEDDING_FIELD.to_string(),
                            detected_text_field,
                            p.provider
                                .clone()
                                .or(auto_provider)
                                .unwrap_or_else(|| manager.default_provider_name().to_string()),
                        )
                    }
                };

                // Embed the query
                let qv = manager
                    .embed_query(&p.query, Some(&prov_name))
                    .map_err(|e| McpError::internal(format!("Query embedding failed: {}", e)))?;

                let eff_vf = if p.vector_field != DEFAULT_EMBEDDING_FIELD {
                    p.vector_field.clone()
                } else {
                    emb_field
                };
                let eff_tf = if p.text_field != DEFAULT_TEXT_FIELD {
                    p.text_field.clone()
                } else {
                    txt_field
                };

                (qv, eff_vf, eff_tf, true, Some(prov_name))
            }
        }
    };

    // Effective fulltext fields: the caller's explicit text_fields, else the
    // collection's configured multi-field set (#66) so search defaults match how
    // the collection was set up. Single-field collections stay on effective_text_field.
    //
    // The config-derived default is intersected with the fields that ACTUALLY have
    // a fulltext index — a field whose index failed to build or was later dropped
    // must not reach fulltext_search_multi (it would hard-error the whole search).
    let config_fields = get_rag_config(adapter, &p.collection)
        .ok()
        .flatten()
        .map(|c| c.effective_text_fields())
        .unwrap_or_default();
    let indexed = adapter
        .get_fulltext_field_names(&p.collection)
        .unwrap_or_default();
    let effective_text_fields =
        super::rag::resolve_search_text_fields(p.text_fields.clone(), config_fields, &indexed);

    validate_match_scope(p.match_scope.as_deref())?;
    let query = HybridQuery {
        query: p.query.clone(),
        vector: (!skip_vector).then_some(query_vector),
        vector_field: effective_vector_field,
        searched_vector_field: None,
        text_field: effective_text_field,
        text_fields: effective_text_fields,
        vector_weight,
        fulltext_weight,
        rrf_k: p.rrf_k,
        limit: p.limit,
        filter: p.filter.clone(),
        and_mode: resolve_and_mode(p.mode.as_deref()),
        document_scope: p.match_scope.as_deref() != Some("chunk"),
        rerank: p.rerank,
        title_field: p.title_field.clone(),
        max_chunks_per_doc: p.max_chunks_per_doc,
        merge_chunks: p.merge_chunks,
        deduplicate: p.deduplicate,
        mmr_lambda: p.mmr_lambda,
        group_by_document: p.group_by_document,
        projection: parse_projection_value(p.projection.clone())?,
    };
    let fusion = adapter.hybrid_fuse(&p.collection, &query)?;

    Ok(FusionOutcome {
        collection: p.collection.clone(),
        fused: fusion.fused,
        qual: fusion.qualification,
        query,
    })
}

/// Group the fused results by document (`CollectionCore::hybrid_group_documents`)
/// with the query they were fused with, the returned chunks projected by
/// `projection`
pub(crate) fn build_doc_groups(
    adapter: &Arc<IronBaseAdapter>,
    fo: &FusionOutcome,
    projection: &Option<HashMap<String, i32>>,
) -> Result<(Vec<DocGroup>, usize)> {
    if *projection == fo.query.projection {
        return adapter.hybrid_group_documents(&fo.collection, &fo.fused, &fo.query);
    }
    let query = HybridQuery {
        projection: projection.clone(),
        ..fo.query.clone()
    };
    adapter.hybrid_group_documents(&fo.collection, &fo.fused, &query)
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
    use crate::tools::params::ParseParams;
    use serde_json::json;

    // -------------------------------------------------------------------------
    // pick_text_field (#64 — deterministic merge field resolution)
    // -------------------------------------------------------------------------

    #[test]
    fn test_pick_text_field_prefers_content() {
        // Order must not matter: content always wins (prevents merge picking title).
        let fields = vec!["title".into(), "customer".into(), "content".into()];
        assert_eq!(pick_text_field(fields), "content");
        let fields = vec!["content".into(), "title".into()];
        assert_eq!(pick_text_field(fields), "content");
    }

    #[test]
    fn test_pick_text_field_deterministic_without_content() {
        // No "content" field → lexicographically-first, deterministically.
        let fields = vec!["title".into(), "body".into(), "abstract".into()];
        assert_eq!(pick_text_field(fields), "abstract");
        // Same set, different input order → same result.
        let fields = vec!["body".into(), "abstract".into(), "title".into()];
        assert_eq!(pick_text_field(fields), "abstract");
    }

    #[test]
    fn test_pick_text_field_empty_falls_back_to_default() {
        assert_eq!(pick_text_field(vec![]), DEFAULT_TEXT_FIELD);
    }

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
        assert!(!p.deduplicate); // default: false
        assert!((p.mmr_lambda - 0.7).abs() < f64::EPSILON); // default
        assert!(!p.group_by_document); // default: false
        assert!(p.max_chunks_per_doc.is_none()); // default: no cap
        assert!(p.provider.is_none()); // no provider → use collection config
        assert!(p.filter.is_none()); // default: no filter
        assert!(p.mode.is_none()); // default: None (= "or", disjunctive)
        assert!(p.text_fields.is_none()); // default: None (= single text_field)
        assert!(p.vector.is_some()); // explicit vector provided
    }

    #[test]
    fn test_params_with_max_chunks_per_doc() {
        let params = json!({
            "collection": "test",
            "vector": [0.1, 0.2],
            "query": "test",
            "max_chunks_per_doc": 3
        });
        let p: HybridSearchParams = HybridSearchParams::parse(params).unwrap();
        assert_eq!(p.max_chunks_per_doc, Some(3));
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
        assert!(p.match_scope.is_none()); // default: None (= "document"); applies only under explicit mode="and"
    }

    #[test]
    fn test_params_with_match_scope_document() {
        let params = json!({
            "collection": "test",
            "vector": [0.1, 0.2],
            "query": "Ifju János fékpad ár",
            "mode": "and",
            "match_scope": "document"
        });
        let p: HybridSearchParams = HybridSearchParams::parse(params).unwrap();
        assert_eq!(p.mode.as_deref(), Some("and"));
        assert_eq!(p.match_scope.as_deref(), Some("document"));
    }

    #[test]
    fn test_params_with_match_scope_chunk() {
        let params = json!({
            "collection": "test",
            "vector": [0.1, 0.2],
            "query": "test query",
            "mode": "and",
            "match_scope": "chunk"
        });
        let p: HybridSearchParams = HybridSearchParams::parse(params).unwrap();
        assert_eq!(p.match_scope.as_deref(), Some("chunk"));
    }

    #[test]
    fn test_params_match_scope_without_mode_or_default() {
        // match_scope="document" with mode=None: the default is now DISJUNCTIVE (OR),
        // so document-level AND qualification does NOT activate without explicit mode="and".
        let params = json!({
            "collection": "test",
            "vector": [0.1, 0.2],
            "query": "test",
            "match_scope": "document"
        });
        let p: HybridSearchParams = HybridSearchParams::parse(params).unwrap();
        assert_eq!(p.match_scope.as_deref(), Some("document"));
        assert!(p.mode.is_none()); // None = OR default → match_scope inert until mode="and"
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
            "provider": "ollama"
        });

        let p: HybridSearchParams = HybridSearchParams::parse(params).unwrap();
        assert_eq!(p.provider.as_deref(), Some("ollama"));
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
        // Empty filter {} is normalized to None to avoid unnecessary collection scans
        let params = json!({
            "collection": "test",
            "vector_field": "embedding",
            "text_field": "content",
            "vector": [0.1, 0.2],
            "query": "test",
            "filter": {}
        });

        let p: HybridSearchParams = HybridSearchParams::parse(params).unwrap();
        assert!(
            p.filter.is_none(),
            "Empty filter {{}} should be normalized to None"
        );
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

// ============================================================================
// Pipeline integration tests (ported from the removed `hybrid_search` MCP tool)
//
// When the deprecated `hybrid_search` tool was removed, its #68/#71/#72
// regression tests moved here so they exercise the RETAINED shared pipeline
// (`retrieve_and_fuse` + `build_doc_groups`) directly — identical coverage with
// no public mechanism surface. The former tool's flat/grouped JSON wrapper is
// reproduced by asserting on `FusionOutcome::fused` (flat) and `DocGroup`
// (grouped), exactly what `handle_hybrid_search` built its response from.
// ============================================================================
#[cfg(test)]
mod pipeline_integration_tests {
    use super::{build_doc_groups, retrieve_and_fuse, DocGroup, FusedResult, FusionOutcome};
    use crate::adapter::IronBaseAdapter;
    use crate::tools::dispatch_tool;
    use crate::tools::helpers::parse_projection_value;
    use crate::tools::params::{HybridSearchParams, ParseParams};
    use serde_json::{json, Value};
    use std::collections::HashMap;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn adapter() -> (Arc<IronBaseAdapter>, TempDir) {
        let tmp = TempDir::new().expect("temp dir");
        let path = tmp.path().join("test.mlite");
        let a = IronBaseAdapter::new(path.to_string_lossy().to_string()).expect("adapter");
        (Arc::new(a), tmp)
    }

    fn ok(a: &Arc<IronBaseAdapter>, name: &str, params: Value) -> Value {
        dispatch_tool(name, params, a, None, None, None, None, &None, &None)
            .unwrap_or_else(|e| panic!("tool '{name}' should succeed: {e:?}"))
    }

    /// Run the shared retrieve+fuse pipeline (what `handle_hybrid_search` used to do).
    fn fuse(a: &Arc<IronBaseAdapter>, params: Value) -> (HybridSearchParams, FusionOutcome) {
        let p = HybridSearchParams::parse(params).expect("parse params");
        let fo = retrieve_and_fuse(&p, a, &None).expect("retrieve_and_fuse");
        (p, fo)
    }

    /// Grouped builder over a fused set — the kept logic the tool used in grouped mode.
    fn group(
        a: &Arc<IronBaseAdapter>,
        p: &HybridSearchParams,
        fo: &FusionOutcome,
    ) -> (Vec<DocGroup>, usize) {
        let proj = parse_projection_value(p.projection.clone()).expect("projection");
        build_doc_groups(a, fo, &proj).expect("build_doc_groups")
    }

    fn count_by_doc(fused: &[FusedResult]) -> HashMap<String, usize> {
        let mut counts = HashMap::new();
        for item in fused {
            if let Some(d) = item.doc.get("doc_id").and_then(|v| v.as_str()) {
                *counts.entry(d.to_string()).or_insert(0) += 1;
            }
        }
        counts
    }

    /// Hybrid review H1: grouping must keep what the vector lane found. Phase 2
    /// re-queries BM25 only, so a document whose matching chunk shares no query
    /// token ("sem") was dropped, and a chunk matched only by the vector lane
    /// ("mixed" chunk 0) was dropped from a document that also had a BM25 hit.
    #[test]
    fn grouped_keeps_vector_only_documents_and_chunks() {
        let (a, _t) = adapter();
        let chunks = [
            (
                "sem",
                0,
                "The automobile needs periodic servicing.",
                [1.0, 0.0, 0.0, 0.0],
            ),
            (
                "mixed",
                0,
                "Engine oil and tyre pressure.",
                [0.9, 0.1, 0.0, 0.0],
            ),
            (
                "mixed",
                1,
                "Maintenance log template.",
                [0.0, 0.0, 1.0, 0.0],
            ),
            (
                "lex",
                0,
                "Maintenance schedule for printers.",
                [0.0, 1.0, 0.0, 0.0],
            ),
        ];
        for (id, idx, content, emb) in chunks {
            ok(
                &a,
                "insert_one",
                json!({"collection":"kb","document":{
                "doc_id": id, "chunk_index": idx, "content": content, "embedding": emb }}),
            );
        }
        ok(
            &a,
            "index_create",
            json!({"collection":"kb","type":"fulltext","field":"content"}),
        );
        ok(
            &a,
            "index_create",
            json!({"collection":"kb","type":"vector","field":"embedding","dim":4,"metric":"cosine"}),
        );

        let (p, fo) = fuse(
            &a,
            json!({"collection":"kb","query":"car maintenance",
            "vector":[1.0,0.0,0.0,0.0],"group_by_document":true,"limit":5,
            // per-chunk membership is checked; merging is covered separately
            "merge_chunks":false}),
        );
        let (groups, total) = group(&a, &p, &fo);

        let contents = |doc: &str| -> Vec<String> {
            groups
                .iter()
                .find(|g| g.doc_id == doc)
                .map(|g| {
                    g.chunks
                        .iter()
                        .map(|c| c["content"].as_str().unwrap_or_default().to_string())
                        .collect()
                })
                .unwrap_or_default()
        };
        assert_eq!(
            contents("sem"),
            vec!["The automobile needs periodic servicing."],
            "vector-only document dropped from the groups"
        );
        let fused_mixed: Vec<String> = fo
            .fused
            .iter()
            .filter(|f| f.doc["doc_id"] == "mixed")
            .map(|f| f.doc["content"].as_str().unwrap_or_default().to_string())
            .collect();
        assert_eq!(
            fused_mixed.len(),
            2,
            "both chunks must be fused: {fused_mixed:?}"
        );
        assert_eq!(
            contents("mixed"),
            fused_mixed,
            "vector-only chunk dropped, or passages not in fused order"
        );
        assert!(!contents("lex").is_empty(), "BM25 document lost");
        assert_eq!(total, groups.iter().map(|g| g.chunks.len()).sum::<usize>());
    }

    /// Hybrid review T2: the lanes leave the embedding unparsed only when the
    /// output drops it and MMR does not read it; the fused results are
    /// otherwise the same, and so are the grouped passages
    #[test]
    fn lanes_skip_the_embedding_only_when_projected_out() {
        let (a, _t) = adapter();
        for i in 0..6 {
            ok(
                &a,
                "insert_one",
                json!({"collection":"kb","document":{
                "doc_id": format!("d{}", i / 2), "chunk_index": i % 2,
                "content": format!("maintenance step {i}"),
                "embedding": [1.0, i as f64, 0.5, 0.0] }}),
            );
        }
        ok(
            &a,
            "index_create",
            json!({"collection":"kb","type":"fulltext","field":"content"}),
        );
        ok(
            &a,
            "index_create",
            json!({"collection":"kb","type":"vector","field":"embedding","dim":4,"metric":"cosine"}),
        );
        let params = |extra: Value| {
            let mut p = json!({"collection":"kb","query":"maintenance",
                "vector":[1.0,2.0,0.5,0.0],"limit":6,"merge_chunks":false});
            for (k, v) in extra.as_object().unwrap() {
                p[k] = v.clone();
            }
            p
        };
        let summary = |fo: &FusionOutcome| -> Vec<(String, f64)> {
            fo.fused
                .iter()
                .map(|f| (f.id.clone(), f.final_score))
                .collect()
        };

        let (_, full) = fuse(&a, params(json!({})));
        assert!(full.fused.iter().all(|f| f.doc.get("embedding").is_some()));

        let (_, lean) = fuse(&a, params(json!({"projection": {"embedding": 0}})));
        assert!(lean.fused.iter().all(|f| f.doc.get("embedding").is_none()));
        assert_eq!(summary(&lean), summary(&full));

        let (_, mmr) = fuse(
            &a,
            params(json!({"projection": {"embedding": 0}, "deduplicate": true})),
        );
        assert!(
            mmr.fused.iter().all(|f| f.doc.get("embedding").is_some()),
            "MMR reads the embedding"
        );

        let grouped = |extra: Value| {
            let (p, fo) = fuse(&a, params(extra));
            let (groups, _) = group(&a, &p, &fo);
            groups
                .into_iter()
                .map(|g| (g.doc_id, g.chunks))
                .collect::<Vec<_>>()
        };
        let lean_groups =
            grouped(json!({"group_by_document": true, "projection": {"embedding": 0}}));
        let full_groups = grouped(json!({"group_by_document": true}));
        assert_eq!(lean_groups.len(), full_groups.len());
        for ((lean_id, lean_chunks), (id, chunks)) in lean_groups.iter().zip(&full_groups) {
            assert_eq!(lean_id, id);
            let expected: Vec<Value> = chunks
                .iter()
                .map(|c| {
                    let mut c = c.clone();
                    c.as_object_mut().unwrap().remove("embedding");
                    c
                })
                .collect();
            assert_eq!(lean_chunks, &expected);
        }
    }

    /// Hybrid review M3: a lane that did not return a document contributes
    /// nothing (standard RRF). It used to count as rank `internal_limit + 1`,
    /// a penalty that shrinks as `limit` grows, so the order of the same
    /// documents depended on the requested limit: here "a" (text rank 1, no
    /// vector) beat "b" (rank 3 in both lanes) at limit 1 but not at limit 10.
    #[test]
    fn rrf_order_does_not_depend_on_limit() {
        let (a, _t) = adapter();
        let docs = [
            json!({"_id":"a","content":"alpha alpha alpha alpha"}),
            json!({"_id":"t2","content":"alpha alpha other"}),
            json!({"_id":"b","content":"alpha with several more filler words here","embedding":[0.9,0.1,0.0,0.0]}),
            json!({"_id":"v1","content":"unrelated","embedding":[1.0,0.0,0.0,0.0]}),
            json!({"_id":"v2","content":"unrelated too","embedding":[0.95,0.05,0.0,0.0]}),
        ];
        for d in docs {
            ok(&a, "insert_one", json!({"collection":"kb","document":d}));
        }
        ok(
            &a,
            "index_create",
            json!({"collection":"kb","type":"fulltext","field":"content"}),
        );
        ok(
            &a,
            "index_create",
            json!({"collection":"kb","type":"vector","field":"embedding","dim":4,"metric":"cosine"}),
        );
        let top = |limit: usize| -> String {
            let (_p, fo) = fuse(
                &a,
                json!({"collection":"kb","query":"alpha","vector":[1.0,0.0,0.0,0.0],
                       "rerank":false,"limit":limit}),
            );
            fo.fused[0].id.clone()
        };
        assert_eq!(top(1), top(10), "the best document changed with the limit");
        assert_eq!(top(1), "b");
    }

    /// Hybrid review H1: grouped mode merges a document's overlapping
    /// neighbour chunks like flat mode does, so the shared text is returned
    /// once. It never merged, so each overlap reached the response twice.
    #[test]
    fn grouped_merges_adjacent_chunks() {
        let (a, _t) = adapter();
        // "alpha one two three four alpha five six" in overlapping chunks
        let chunks = [
            (0, "alpha one two three", 0, 19),
            (1, "two three four alpha", 10, 30),
            (2, "four alpha five six", 20, 39),
        ];
        for (idx, content, start, end) in chunks {
            ok(
                &a,
                "insert_one",
                json!({"collection":"kb","document":{
                "doc_id":"d","chunk_index":idx,"content":content,
                "start_char":start,"end_char":end,"embedding":[1.0,0.0,0.0,0.0]}}),
            );
        }
        ok(
            &a,
            "index_create",
            json!({"collection":"kb","type":"fulltext","field":"content"}),
        );
        ok(
            &a,
            "index_create",
            json!({"collection":"kb","type":"vector","field":"embedding","dim":4,"metric":"cosine"}),
        );

        let (p, fo) = fuse(
            &a,
            json!({"collection":"kb","query":"alpha","vector":[1.0,0.0,0.0,0.0],
                   "group_by_document":true,"limit":5}),
        );
        let (groups, total) = group(&a, &p, &fo);
        assert_eq!(groups.len(), 1);
        let chunks = &groups[0].chunks;
        assert_eq!(
            chunks.len(),
            1,
            "adjacent chunks were not merged: {chunks:?}"
        );
        assert_eq!(chunks[0]["chunks_in_merge"], json!(3));
        assert_eq!(
            chunks[0]["content"],
            json!("alpha one two three four alpha five six")
        );
        assert_eq!(total, 1);
    }

    /// #71: grouped chunks carry the full Phase-1 engine score shape (not just _text_score).
    #[test]
    fn grouped_chunk_score_fields_issue71() {
        let (a, _t) = adapter();
        let docs = [
            ("alpha", 0, "fékpad PEF-35 leírás", [0.9, 0.1, 0.0, 0.0]),
            ("alpha", 1, "fékpad PEF-35 ár", [0.8, 0.2, 0.0, 0.0]),
            (
                "beta",
                0,
                "fékpad PEF-35 specifikáció",
                [0.1, 0.9, 0.0, 0.0],
            ),
            ("beta", 1, "fékpad PEF-35 garancia", [0.0, 0.8, 0.2, 0.0]),
        ];
        for (id, idx, content, emb) in docs {
            ok(
                &a,
                "insert_one",
                json!({"collection":"kb","document":{
                "doc_id": id, "chunk_index": idx, "content": content,
                "title": format!("{id} title"), "embedding": emb }}),
            );
        }
        ok(
            &a,
            "index_create",
            json!({"collection":"kb","type":"fulltext","field":"content"}),
        );
        ok(
            &a,
            "index_create",
            json!({"collection":"kb","type":"vector","field":"embedding","dim":4,"metric":"cosine"}),
        );

        let (p, fo) = fuse(
            &a,
            json!({"collection":"kb","query":"PEF-35",
            "vector":[0.9,0.1,0.0,0.0],"group_by_document":true,"limit":5}),
        );
        let (groups, _total) = group(&a, &p, &fo);
        assert!(!groups.is_empty());
        let mut phase1_seen = false;
        for g in &groups {
            assert!(!g.chunks.is_empty());
            for c in &g.chunks {
                if c.get("_final_score").is_some() {
                    phase1_seen = true;
                    assert!(c["_rrf_score"].is_number(), "missing _rrf_score: {c}");
                    assert!(c["_rerank_boost"].is_number(), "missing _rerank_boost: {c}");
                    assert!(c["_text_rank"].is_number(), "missing _text_rank: {c}");
                    assert!(c["_vector_rank"].is_number(), "missing _vector_rank: {c}");
                    assert!(c["_text_score"].is_number(), "missing _text_score: {c}");
                }
            }
        }
        assert!(
            phase1_seen,
            "no chunk carried _final_score — Phase 1 enrichment did not run"
        );
    }

    /// #71: max_chunks_per_doc caps grouped chunks; lift runs BEFORE the cap.
    #[test]
    fn grouped_max_chunks_per_doc_issue71() {
        let (a, _t) = adapter();
        for idx in 0..4 {
            ok(
                &a,
                "insert_one",
                json!({"collection":"kb","document":{
                "doc_id":"alpha","chunk_index":idx,
                "content": format!("fékpad PEF-35 chunk {idx}"),
                "title":"alpha title","embedding":[0.9,0.1,0.0,0.0]}}),
            );
        }
        ok(
            &a,
            "index_create",
            json!({"collection":"kb","type":"fulltext","field":"content"}),
        );
        ok(
            &a,
            "index_create",
            json!({"collection":"kb","type":"vector","field":"embedding","dim":4,"metric":"cosine"}),
        );

        let (p, fo) = fuse(
            &a,
            json!({"collection":"kb","query":"PEF-35",
            "vector":[0.9,0.1,0.0,0.0],"group_by_document":true,"limit":5,"max_chunks_per_doc":2,
            // the cap applies to separate chunks; merging is covered separately
            "merge_chunks":false}),
        );
        let (groups, total) = group(&a, &p, &fo);
        assert_eq!(groups.len(), 1, "expected single doc group");
        let g = &groups[0];
        assert!(g.chunks.len() <= 2, "cap violated: {}", g.chunks.len());
        // Lift runs on the full Phase-2 set then cap → title lifted to group level.
        assert_eq!(
            g.lifted.get("title"),
            Some(&json!("alpha title")),
            "title not lifted; cap must run AFTER lift_common_fields"
        );
        for c in &g.chunks {
            assert!(
                c.get("title").is_none(),
                "lifted field leaked back into chunk: {c}"
            );
        }
        assert_eq!(total, g.chunks.len(), "total counts post-cap chunks");
    }

    /// #71 AC2: flat-mode and grouped-mode doc order agree for identical inputs.
    #[test]
    fn flat_grouped_doc_order_agreement_issue71_ac2() {
        let (a, _t) = adapter();
        let inserts = [
            (
                "alpha",
                0,
                "fékpad PEF-35 ajánlat",
                "alpha cikk PEF-35",
                [0.7, 0.3, 0.0, 0.0],
            ),
            (
                "alpha",
                1,
                "fékpad PEF-35 ár 4.2M Ft",
                "alpha cikk PEF-35",
                [0.6, 0.4, 0.0, 0.0],
            ),
            (
                "beta",
                0,
                "fékpad telepítés helyszínen",
                "beta cikk",
                [0.4, 0.6, 0.0, 0.0],
            ),
            (
                "beta",
                1,
                "PEF-35 leírás külön",
                "beta cikk",
                [0.3, 0.7, 0.0, 0.0],
            ),
            (
                "gamma",
                0,
                "fékpad általános leírás",
                "gamma cikk",
                [0.2, 0.8, 0.0, 0.0],
            ),
        ];
        for (id, idx, content, title, emb) in inserts {
            ok(
                &a,
                "insert_one",
                json!({"collection":"kb","document":{
                "doc_id": id, "chunk_index": idx, "content": content, "title": title, "embedding": emb }}),
            );
        }
        ok(
            &a,
            "index_create",
            json!({"collection":"kb","type":"fulltext","field":"content"}),
        );
        ok(
            &a,
            "index_create",
            json!({"collection":"kb","type":"fulltext","field":"title"}),
        );
        ok(
            &a,
            "index_create",
            json!({"collection":"kb","type":"vector","field":"embedding","dim":4,"metric":"cosine"}),
        );

        let common = json!({"collection":"kb","query":"PEF-35 fékpad",
            "vector":[0.5,0.5,0.0,0.0],"title_field":"title","limit":5});

        let (_pf, fo_flat) = fuse(&a, common.clone());
        let mut flat_order: Vec<String> = Vec::new();
        for item in &fo_flat.fused {
            if let Some(d) = item.doc.get("doc_id").and_then(|v| v.as_str()) {
                if !flat_order.contains(&d.to_string()) {
                    flat_order.push(d.to_string());
                }
            }
        }

        let mut grouped_params = common.as_object().unwrap().clone();
        grouped_params.insert("group_by_document".into(), json!(true));
        let (pg, fo_g) = fuse(&a, Value::Object(grouped_params));
        let (groups, _) = group(&a, &pg, &fo_g);
        let grouped_order: Vec<String> = groups.iter().map(|g| g.doc_id.clone()).collect();

        assert!(!flat_order.is_empty(), "flat returned no results");
        assert!(!grouped_order.is_empty(), "grouped returned no groups");
        let n = flat_order.len().min(grouped_order.len());
        assert_eq!(
            &flat_order[..n],
            &grouped_order[..n],
            "flat vs grouped doc order diverges. flat={flat_order:?} grouped={grouped_order:?}"
        );
    }

    /// #72: flat-mode max_chunks_per_doc caps per-doc chunk count in the global top-K.
    #[test]
    fn flat_max_chunks_per_doc_issue72() {
        let (a, _t) = adapter();
        for doc_id in ["alpha", "beta"] {
            for idx in 0..4 {
                ok(
                    &a,
                    "insert_one",
                    json!({"collection":"kb","document":{
                    "doc_id": doc_id, "chunk_index": idx,
                    "content": format!("PEF-35 fékpad chunk {doc_id}-{idx}"),
                    "embedding": [0.5, 0.5, 0.0, 0.0]}}),
                );
            }
        }
        ok(
            &a,
            "index_create",
            json!({"collection":"kb","type":"fulltext","field":"content"}),
        );
        ok(
            &a,
            "index_create",
            json!({"collection":"kb","type":"vector","field":"embedding","dim":4,"metric":"cosine"}),
        );

        let (_p, fo) = fuse(
            &a,
            json!({"collection":"kb","query":"PEF-35",
            "vector":[0.5,0.5,0.0,0.0],"limit":50,"max_chunks_per_doc":2,"merge_chunks":false}),
        );
        let counts = count_by_doc(&fo.fused);
        for (doc_id, cnt) in &counts {
            assert!(*cnt <= 2, "doc {doc_id} exceeded cap: {cnt} > 2");
        }
        assert!(counts.contains_key("alpha"), "alpha missing: {counts:?}");
        assert!(counts.contains_key("beta"), "beta missing: {counts:?}");
    }

    /// #72 AC2: no cap (default) keeps all chunks competing for the limit slot.
    #[test]
    fn flat_max_chunks_per_doc_default_backward_compatible_issue72() {
        let (a, _t) = adapter();
        for idx in 0..4 {
            ok(
                &a,
                "insert_one",
                json!({"collection":"kb","document":{
                "doc_id":"alpha","chunk_index":idx,
                "content": format!("PEF-35 fékpad chunk alpha-{idx}"),
                "embedding":[0.5,0.5,0.0,0.0]}}),
            );
        }
        ok(
            &a,
            "index_create",
            json!({"collection":"kb","type":"fulltext","field":"content"}),
        );
        ok(
            &a,
            "index_create",
            json!({"collection":"kb","type":"vector","field":"embedding","dim":4,"metric":"cosine"}),
        );

        let (_p, fo) = fuse(
            &a,
            json!({"collection":"kb","query":"PEF-35",
            "vector":[0.5,0.5,0.0,0.0],"limit":50,"merge_chunks":false}),
        );
        let counts = count_by_doc(&fo.fused);
        assert_eq!(
            counts.get("alpha").copied(),
            Some(4),
            "expected all 4 alpha chunks without cap: {counts:?}"
        );
    }

    /// Regression: multi-field document-scope qualification on a NON-RAG
    /// collection. The fulltext index carries no parent doc_id field, so
    /// the qualification keys the filter on `_id`; the multi-field union must
    /// honor that key. It previously hardcoded `doc_id`, zeroing out every
    /// multi-field document-scope query on non-RAG data (the single-field fix
    /// did not cover the union path).
    #[test]
    fn multi_field_non_rag_document_scope_qualifies() {
        let (a, _t) = adapter();
        // Non-RAG: plain documents, two fulltext-indexed fields, no vector index.
        ok(
            &a,
            "insert_one",
            json!({"collection":"kb","document":{
            "content":"fékpad PEF-35 berendezés leírás", "title":"alpha"}}),
        );
        ok(
            &a,
            "insert_one",
            json!({"collection":"kb","document":{
            "content":"csak fékpad egyedül", "title":"beta"}}),
        );
        ok(
            &a,
            "index_create",
            json!({"collection":"kb","type":"fulltext","field":"content"}),
        );
        ok(
            &a,
            "index_create",
            json!({"collection":"kb","type":"fulltext","field":"title"}),
        );

        // BM25-only (vector_weight=0 → skip embedding), multi-field, explicit
        // mode="and" + match_scope="document" (document-level AND qualification is
        // now OPT-IN; the default is disjunctive OR). Doc 1 has ALL query tokens;
        // doc 2 has only one and must be excluded by document-level AND qualification.
        let (_p, fo) = fuse(
            &a,
            json!({"collection":"kb","query":"fékpad PEF-35",
                "text_fields":["content","title"], "vector_weight":0.0,
                "fulltext_weight":1.0, "limit":10,
                "mode":"and", "match_scope":"document"}),
        );

        assert!(
            !fo.fused.is_empty(),
            "multi-field non-RAG document-scope qualification returned nothing"
        );
        let contents: Vec<String> = fo
            .fused
            .iter()
            .filter_map(|i| {
                i.doc
                    .get("content")
                    .and_then(|v| v.as_str())
                    .map(String::from)
            })
            .collect();
        assert!(
            contents.iter().any(|c| c.contains("PEF-35")),
            "qualifying doc (both tokens) missing: {contents:?}"
        );
        assert!(
            contents.iter().all(|c| !c.starts_with("csak fékpad")),
            "non-qualifying doc (one token) must be excluded by doc-scope AND: {contents:?}"
        );
    }
}
