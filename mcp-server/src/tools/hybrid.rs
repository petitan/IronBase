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

use crate::adapter::{FulltextSearchOptions, IronBaseAdapter};
use crate::embedding::EmbeddingManager;
use crate::error::{McpError, Result};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use super::defaults::{DEFAULT_EMBEDDING_FIELD, DEFAULT_TEXT_FIELD, MAX_INTERNAL_LIMIT};
use super::fusion::{
    apply_document_qualification, apply_projection, id_to_string, merge_adjacent_chunks,
    mmr_reorder, rerank_results, FusedResult,
};
use super::helpers::{parse_projection_value, validate_collection_name};
use super::params::{resolve_weights, HybridSearchParams, ParseParams};
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
fn pick_text_field(mut fields: Vec<String>) -> String {
    if fields.iter().any(|f| f == DEFAULT_TEXT_FIELD) {
        return DEFAULT_TEXT_FIELD.to_string();
    }
    fields.sort();
    fields
        .into_iter()
        .next()
        .unwrap_or_else(|| DEFAULT_TEXT_FIELD.to_string())
}

/// Apply projection and add score metadata to a fused result
fn enrich_result(item: FusedResult, projection: &Option<HashMap<String, i32>>) -> Value {
    let mut result = if let Some(ref proj) = projection {
        apply_projection(&item.doc, proj)
    } else {
        item.doc
    };
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
    result
}

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
    //         If vector_weight == 0.0 and no explicit vector → skip embedding
    //         entirely (BM25-only mode)
    // ========================================================================
    let skip_vector = vector_weight == 0.0 && p.vector.is_none();

    let (query_vector, effective_vector_field, effective_text_field, auto_embedded, provider_name) =
        if skip_vector {
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
            match p.vector {
                Some(ref v) => {
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
                        .map_err(|e| {
                            McpError::internal(format!("Query embedding failed: {}", e))
                        })?;

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

    // Internal limit: higher when grouping to capture enough chunks per document
    let internal_multiplier = if p.group_by_document { 20 } else { 3 };
    let internal_limit = (p.limit * internal_multiplier).min(MAX_INTERNAL_LIMIT);

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

    // ========================================================================
    // STEP 1.5: Document-level AND qualification (match_scope="document")
    //           Uses shared orchestration from fusion.rs
    // ========================================================================
    let qual_fields: Vec<&str> = if let Some(ref fields) = effective_text_fields {
        fields.iter().map(|s| s.as_str()).collect()
    } else {
        vec![&effective_text_field]
    };

    let qual = apply_document_qualification(
        adapter,
        &p.collection,
        &qual_fields,
        &p.query,
        p.mode.as_deref(),
        p.match_scope.as_deref(),
        p.filter.clone(),
    )?;

    // ========================================================================
    // STEP 2: Vector search → ranks + docs (single pass)
    //         Skipped when vector_weight == 0.0 (BM25-only mode)
    // ========================================================================
    let mut vector_ranks: HashMap<String, usize> = HashMap::new();
    let mut vector_docs: HashMap<String, (Value, f32)> = HashMap::new();

    if !skip_vector {
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

        vector_ranks.reserve(vector_results.len());
        vector_docs.reserve(vector_results.len());
        for (rank, (doc, score)) in vector_results.into_iter().enumerate() {
            if let Some(id) = doc.get("_id").and_then(id_to_string) {
                vector_ranks.insert(id.clone(), rank + 1);
                vector_docs.insert(id, (doc, score));
            }
        }
    }

    // ========================================================================
    // STEP 3: Fulltext search → ranks + docs (single pass)
    //         Uses qualification outcome from STEP 1.5 (and_mode, filter)
    // ========================================================================
    let text_options = FulltextSearchOptions {
        limit: Some(internal_limit),
        skip: None,
        min_score: None,
        projection: None, // Get full docs for merging
        filter: qual.effective_filter.clone(),
        and_mode: qual.effective_and_mode,
        highlight: false,
        highlight_context: None,
        highlight_max_snippets: None,
        target_doc_ids: None,
    };

    // Multi-field search if `text_fields` is provided, otherwise single-field
    let text_results = if let Some(ref fields) = effective_text_fields {
        let field_refs: Vec<&str> = fields.iter().map(|s| s.as_str()).collect();
        adapter.fulltext_search_multi(&p.collection, &field_refs, &p.query, text_options)?
    } else {
        adapter.fulltext_search(&p.collection, &effective_text_field, &p.query, text_options)?
    };

    let mut text_ranks: HashMap<String, usize> = HashMap::with_capacity(text_results.len());
    let mut text_docs: HashMap<String, (Value, f64)> = HashMap::with_capacity(text_results.len());
    for (rank, res) in text_results.into_iter().enumerate() {
        if let Some(id) = res.document.get("_id").and_then(id_to_string) {
            text_ranks.insert(id.clone(), rank + 1);
            text_docs.insert(id, (res.document, res.score));
        }
    }

    // ========================================================================
    // STEP 4: RRF Fusion (zero-copy drain, no intermediate HashSet)
    // ========================================================================
    let default_rank = internal_limit + 1;
    let mut fused: Vec<FusedResult> = Vec::with_capacity(vector_docs.len() + text_docs.len());

    // Vector docs first — consume matching text docs via remove (no clone)
    for (id, (doc, v_score)) in vector_docs.drain() {
        let v_rank = vector_ranks.get(&id).copied().unwrap_or(default_rank);
        let t_rank = text_ranks.get(&id).copied().unwrap_or(default_rank);
        let t_score = text_docs.remove(&id).map(|(_, s)| s);

        let rrf_score =
            vector_weight / (p.rrf_k + v_rank as f64) + fulltext_weight / (p.rrf_k + t_rank as f64);

        fused.push(FusedResult {
            id,
            doc,
            rrf_score,
            final_score: rrf_score,
            rerank_boost: 1.0,
            v_rank,
            t_rank,
            v_score: Some(v_score),
            t_score,
        });
    }

    // Text-only docs (remaining after vector drain consumed overlaps)
    for (id, (doc, t_score)) in text_docs.drain() {
        let t_rank = text_ranks.get(&id).copied().unwrap_or(default_rank);

        let rrf_score = vector_weight / (p.rrf_k + default_rank as f64)
            + fulltext_weight / (p.rrf_k + t_rank as f64);

        fused.push(FusedResult {
            id,
            doc,
            rrf_score,
            final_score: rrf_score,
            rerank_boost: 1.0,
            v_rank: default_rank,
            t_rank,
            v_score: None,
            t_score: Some(t_score),
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
    // STEP 6: MMR diversity reranking (skipped when grouping — limit applies
    //         at document level in STEP 7)
    // ========================================================================
    let dedup_removed = if p.deduplicate && !p.group_by_document {
        mmr_reorder(&mut fused, &effective_vector_field, p.mmr_lambda, p.limit)
    } else if !p.group_by_document {
        // Flat mode: truncate to limit here
        fused.truncate(p.limit);
        0
    } else {
        // group_by_document: no truncation, limit applied at doc level in STEP 7
        0
    };

    // ========================================================================
    // STEP 7: Projection + response
    // ========================================================================
    let projection = parse_projection_value(p.projection)?;
    let match_scope = if qual.is_doc_scope {
        "document"
    } else {
        "chunk"
    };

    // Common response metadata (shared between flat and grouped modes)
    let mut response = serde_json::Map::new();
    response.insert("algorithm".into(), json!("rrf"));
    response.insert("rrf_k".into(), json!(p.rrf_k));
    response.insert(
        "weights".into(),
        json!({
            "vector": vector_weight,
            "fulltext": fulltext_weight
        }),
    );
    response.insert(
        "search_mode".into(),
        json!(p.search_mode.as_deref().unwrap_or("balanced")),
    );
    response.insert("query".into(), json!(p.query));
    response.insert("auto_embedded".into(), json!(auto_embedded));
    response.insert("provider".into(), json!(provider_name));
    response.insert("dedup_removed".into(), json!(dedup_removed));
    response.insert("chunks_merged".into(), json!(chunks_merged));
    response.insert("match_scope".into(), json!(match_scope));
    if let Some(count) = qual.qualified_doc_count {
        response.insert("qualified_doc_ids".into(), json!(count));
    }

    if p.group_by_document {
        // ----------------------------------------------------------------
        // Grouped response: two-phase document grouping
        //
        // Phase 1: From fused results, identify top N unique doc_ids
        //          (fused is sorted by score → first occurrence = best score)
        // Phase 2: Single fulltext OR search filtered to those N doc_ids
        //          → fetches ALL relevant chunks from the top documents
        //
        // This is O(1) extra search (not O(limit) like per-doc search).
        // Document selection uses AND (all words in doc), chunk retrieval
        // uses OR (any word in chunk) — exactly what the user expects.
        // ----------------------------------------------------------------

        // Phase 1: Extract top N unique doc_ids from fused results
        let mut doc_best_scores: HashMap<String, f64> = HashMap::new();
        let mut doc_order: Vec<String> = Vec::new();

        for item in &fused {
            let doc_id = item
                .doc
                .get("doc_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| {
                    item.doc
                        .get("_id")
                        .and_then(id_to_string)
                        .unwrap_or_else(|| item.id.clone())
                });

            if !doc_best_scores.contains_key(&doc_id) {
                doc_best_scores.insert(doc_id.clone(), item.final_score);
                doc_order.push(doc_id);
                if doc_order.len() >= p.limit {
                    break;
                }
            }
        }

        // Phase 2: Single fulltext OR search for top doc_ids' chunks
        // Use target_doc_ids for in-memory pre-filtering (no disk I/O for irrelevant chunks)
        let target_doc_id_set: HashSet<String> = doc_order.iter().cloned().collect();

        let phase2_limit = (p.limit * 100).min(MAX_INTERNAL_LIMIT);
        let phase2_options = FulltextSearchOptions {
            limit: Some(phase2_limit), // Scale with requested doc count
            skip: None,
            min_score: None,
            projection: None,
            filter: p.filter.clone(), // Only user filter, doc_id filtering via target_doc_ids
            and_mode: false,          // OR mode: any query word → relevant chunk
            highlight: false,
            highlight_context: None,
            highlight_max_snippets: None,
            target_doc_ids: Some(target_doc_id_set),
        };

        let phase2_results = if let Some(ref fields) = effective_text_fields {
            let field_refs: Vec<&str> = fields.iter().map(|s| s.as_str()).collect();
            adapter.fulltext_search_multi(&p.collection, &field_refs, &p.query, phase2_options)?
        } else {
            adapter.fulltext_search(
                &p.collection,
                &effective_text_field,
                &p.query,
                phase2_options,
            )?
        };

        // Group Phase 2 chunks by doc_id
        let mut doc_groups: HashMap<String, Vec<Value>> = HashMap::with_capacity(doc_order.len());
        for res in phase2_results {
            let doc_id = match res.document.get("doc_id").and_then(|v| v.as_str()) {
                Some(did) => did.to_string(),
                None => {
                    tracing::warn!(
                        "Phase 2 chunk without doc_id, skipping: {:?}",
                        res.document.get("_id")
                    );
                    continue;
                }
            };

            let mut chunk = if let Some(ref proj) = projection {
                apply_projection(&res.document, proj)
            } else {
                res.document
            };
            if let Value::Object(ref mut obj) = chunk {
                obj.insert("_text_score".to_string(), json!(res.score));
            }

            doc_groups.entry(doc_id).or_default().push(chunk);
        }

        // Build grouped response in doc_order (best score first)
        let mut total_chunks: usize = 0;
        let mut grouped_results: Vec<Value> = Vec::new();
        grouped_results.try_reserve(doc_order.len()).map_err(|e| {
            McpError::internal(format!(
                "OOM: cannot allocate {} grouped results: {}",
                doc_order.len(),
                e
            ))
        })?;
        grouped_results.extend(doc_order.into_iter().filter_map(|doc_id| {
            let best_score = doc_best_scores.get(&doc_id).copied().unwrap_or(0.0);
            let chunks = doc_groups.remove(&doc_id).unwrap_or_default();
            if chunks.is_empty() {
                return None;
            }
            total_chunks += chunks.len();
            Some(json!({
                "doc_id": doc_id,
                "best_score": best_score,
                "chunk_count": chunks.len(),
                "chunks": chunks
            }))
        }));

        let doc_count = grouped_results.len();
        response.insert("results".into(), json!(grouped_results));
        response.insert("count".into(), json!(doc_count));
        response.insert("total_chunks".into(), json!(total_chunks));
        response.insert("group_by_document".into(), json!(true));
    } else {
        // ----------------------------------------------------------------
        // Flat response (default): list of chunks ordered by score
        // ----------------------------------------------------------------
        let mut results: Vec<Value> = Vec::new();
        results.try_reserve(fused.len()).map_err(|e| {
            McpError::internal(format!(
                "OOM: cannot allocate {} results: {}",
                fused.len(),
                e
            ))
        })?;

        for item in fused {
            results.push(enrich_result(item, &projection));
        }

        let count = results.len();
        response.insert("results".into(), json!(results));
        response.insert("count".into(), json!(count));
    }

    Ok(Value::Object(response))
}

// qualify_documents moved to fusion.rs (shared between hybrid and fulltext search)

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
        assert!(p.provider.is_none()); // no provider → use collection config
        assert!(p.filter.is_none()); // default: no filter
        assert!(p.mode.is_none()); // default: None (= "and")
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
        assert!(p.match_scope.is_none()); // default: None (= "document" — doc-level AND)
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
    fn test_params_match_scope_without_mode_and() {
        // match_scope="document" with mode=None (default AND) — doc qualification WILL activate
        let params = json!({
            "collection": "test",
            "vector": [0.1, 0.2],
            "query": "test",
            "match_scope": "document"
        });
        let p: HybridSearchParams = HybridSearchParams::parse(params).unwrap();
        assert_eq!(p.match_scope.as_deref(), Some("document"));
        assert!(p.mode.is_none()); // None = AND default → doc qualification activates
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
