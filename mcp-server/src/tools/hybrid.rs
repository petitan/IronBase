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

use crate::adapter::{FindOptions, FulltextSearchOptions, IronBaseAdapter};
use crate::embedding::EmbeddingManager;
use crate::error::{McpError, Result};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use super::defaults::{
    DEFAULT_EMBEDDING_FIELD, DEFAULT_EMBEDDING_PROVIDER, DEFAULT_TEXT_FIELD, MAX_INTERNAL_LIMIT,
};
use super::fusion::{
    apply_projection, id_to_string, merge_adjacent_chunks, mmr_reorder, rerank_results, FusedResult,
};
use super::helpers::{parse_projection_value, validate_collection_name};
use super::params::{resolve_weights, HybridSearchParams, ParseParams};
use super::rag::get_rag_config;

/// RRF default constant - empirically optimal value (Cormack et al., 2009)
/// Used by test helper; runtime value comes from HybridSearchParams.rrf_k
#[cfg(test)]
const RRF_K: f64 = 60.0;

// MAX_INTERNAL_LIMIT imported from defaults.rs

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
    // ========================================================================
    let (query_vector, effective_vector_field, effective_text_field, auto_embedded, provider_name) =
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
                        "Embedding not available. Set IRONBASE_FASTTEXT_MODEL environment variable.",
                    )
                })?;

                // Get RAG config or use defaults
                let rag_config = get_rag_config(adapter, &p.collection)?;
                let (emb_field, txt_field, prov_name) = match &rag_config {
                    Some(cfg) => (
                        cfg.embedding_field.clone(),
                        cfg.text_field.clone(),
                        p.provider.clone().unwrap_or_else(|| cfg.provider.clone()),
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
                    .embed_query(&p.query, Some(&prov_name))
                    .map_err(|e| McpError::internal(format!("Query embedding failed: {}", e)))?;

                // Use user-specified fields if provided, otherwise use resolved defaults.
                // NOTE: detection is heuristic — if user explicitly passes the default value
                // (e.g. vector_field="embedding"), it is treated as "not set" and RAG config
                // takes priority. This is acceptable because explicitly passing the default
                // value has no practical difference from not passing it.
                let eff_vf = if p.vector_field != "embedding" {
                    p.vector_field.clone()
                } else {
                    emb_field
                };
                let eff_tf = if p.text_field != "content" {
                    p.text_field.clone()
                } else {
                    txt_field
                };

                (qv, eff_vf, eff_tf, true, Some(prov_name))
            }
        };

    // Internal limit: higher when grouping to capture enough chunks per document
    let internal_multiplier = if p.group_by_document { 20 } else { 3 };
    let internal_limit = (p.limit * internal_multiplier).min(MAX_INTERNAL_LIMIT);

    // ========================================================================
    // STEP 1.5: Document-level AND qualification (match_scope="document")
    // ========================================================================
    let and_mode = p.mode.as_deref() != Some("or");
    // group_by_document implies document-level AND: select docs where ALL words appear,
    // then use OR mode to find all relevant chunks within those docs
    let is_doc_scope =
        and_mode && (p.match_scope.as_deref() == Some("document") || p.group_by_document);

    let doc_qualification_filter = if is_doc_scope {
        qualify_documents(adapter, &p.collection, &effective_text_field, &p.query)?
    } else {
        None
    };

    let qualified_doc_count = doc_qualification_filter.as_ref().and_then(|f| {
        f.get("doc_id")
            .and_then(|v| v.get("$in"))
            .and_then(|v| v.as_array())
            .map(|a| a.len())
    });

    // ========================================================================
    // STEP 2: Vector search → ranks + docs (single pass)
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

    let mut vector_ranks: HashMap<String, usize> = HashMap::with_capacity(vector_results.len());
    let mut vector_docs: HashMap<String, (Value, f32)> =
        HashMap::with_capacity(vector_results.len());
    for (rank, (doc, score)) in vector_results.into_iter().enumerate() {
        if let Some(id) = doc.get("_id").and_then(id_to_string) {
            vector_ranks.insert(id.clone(), rank + 1);
            vector_docs.insert(id, (doc, score));
        }
    }

    // ========================================================================
    // STEP 3: Fulltext search → ranks + docs (single pass)
    // ========================================================================

    // If document-level AND is active, switch fulltext to OR mode (doc qualification
    // already ensures all tokens appear across the document's chunks) and merge filters.
    let (fulltext_and_mode, fulltext_filter) =
        if let Some(ref qual_filter) = doc_qualification_filter {
            // Merge user filter + doc_id qualification filter with $and
            let merged = match &p.filter {
                Some(user_filter) => Some(json!({"$and": [user_filter, qual_filter]})),
                None => Some(qual_filter.clone()),
            };
            (false, merged) // OR mode — doc-level AND already satisfied
        } else {
            (and_mode, p.filter.clone())
        };

    let text_options = FulltextSearchOptions {
        limit: Some(internal_limit),
        skip: None,
        min_score: None,
        projection: None, // Get full docs for merging
        filter: fulltext_filter,
        and_mode: fulltext_and_mode,
        highlight: false,
        highlight_context: None,
        highlight_max_snippets: None,
        target_doc_ids: None,
    };

    // Multi-field search if `text_fields` is provided, otherwise single-field
    let text_results = if let Some(ref fields) = p.text_fields {
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
    let match_scope = if is_doc_scope { "document" } else { "chunk" };

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
    if let Some(count) = qualified_doc_count {
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

        let phase2_results = if let Some(ref fields) = p.text_fields {
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

/// Document-level AND qualification for RAG collections.
///
/// When `match_scope="document"` + `mode="and"`, we need all query tokens to appear
/// across a document's chunks (not necessarily in one chunk). This function:
/// 1. Tokenizes the query using the fulltext index config
/// 2. Gets posting lists ordered by rarity (smallest first for early termination)
/// 3. Intersects posting lists at document level using chunk→doc mapping
/// 4. Returns a filter that restricts fulltext search to qualified doc_ids
///
/// Returns `Ok(None)` if qualification is not needed (1 token, or no restriction).
/// Returns `Ok(Some(filter))` with `{"doc_id": {"$in": [...]}}` when docs are qualified.
fn qualify_documents(
    adapter: &Arc<IronBaseAdapter>,
    collection: &str,
    text_field: &str,
    query: &str,
) -> Result<Option<Value>> {
    use crate::adapter::QualificationResult;

    let qualify_start = std::time::Instant::now();
    match adapter.fulltext_qualify_documents_fast(collection, text_field, query)? {
        QualificationResult::NotRequired => {
            return Ok(None);
        }
        QualificationResult::Qualified(doc_ids) => {
            let count = doc_ids.len();
            let qualified_vec: Vec<Value> = doc_ids.into_iter().map(|s| json!(s)).collect();
            tracing::info!(
                collection = collection,
                qualified = count,
                elapsed_ms = qualify_start.elapsed().as_millis() as u64,
                "qualify_documents: fast path (chunk_doc_mapping)"
            );
            return Ok(Some(json!({"doc_id": {"$in": qualified_vec}})));
        }
        QualificationResult::LegacyFallback => {
            tracing::warn!(
                collection = collection,
                "qualify_documents: chunk_doc_mapping not available, using find-based fallback"
            );
        }
    }

    // Fallback: find-based qualification (for legacy indexes without chunk_doc_mapping)
    let tokens = adapter.fulltext_tokenize_query(collection, text_field, query)?;
    if tokens.len() <= 1 {
        return Ok(None);
    }

    let mut token_counts =
        adapter.fulltext_token_posting_counts(collection, text_field, &tokens)?;
    token_counts.sort_by_key(|(_, count)| *count);

    if token_counts[0].1 == 0 {
        return Ok(Some(json!({"doc_id": {"$in": []}})));
    }

    let mut qualified: Option<HashSet<String>> = None;

    for (token, _) in &token_counts {
        let chunk_ids = adapter.fulltext_token_chunk_ids(collection, text_field, token)?;

        let find_result = adapter.find(
            collection,
            json!({"_id": {"$in": chunk_ids}}),
            FindOptions {
                projection: Some(json!({"doc_id": 1, "_id": 0})),
                ..Default::default()
            },
        )?;

        let doc_ids: HashSet<String> = find_result
            .documents
            .iter()
            .filter_map(|d| {
                d.get("doc_id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })
            .collect();

        qualified = Some(match qualified {
            None => doc_ids,
            Some(prev) => prev.intersection(&doc_ids).cloned().collect(),
        });

        if qualified.as_ref().is_some_and(|q| q.is_empty()) {
            return Ok(Some(json!({"doc_id": {"$in": []}})));
        }
    }

    let qualified_vec: Vec<Value> = qualified
        .unwrap_or_default()
        .into_iter()
        .map(|s| json!(s))
        .collect();
    Ok(Some(json!({"doc_id": {"$in": qualified_vec}})))
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
        assert!(p.match_scope.is_none()); // default: None (= "chunk")
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
        // match_scope without mode="and" — should parse but won't activate doc qualification
        let params = json!({
            "collection": "test",
            "vector": [0.1, 0.2],
            "query": "test",
            "match_scope": "document"
        });
        let p: HybridSearchParams = HybridSearchParams::parse(params).unwrap();
        assert_eq!(p.match_scope.as_deref(), Some("document"));
        assert!(p.mode.is_none()); // no "and" → doc qualification won't activate
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
