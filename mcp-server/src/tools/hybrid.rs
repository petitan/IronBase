//! Hybrid search tool handler (RRF-based fusion)
//!
//! Combines vector similarity and fulltext search using Reciprocal Rank Fusion (RRF).
//! RRF is collection-size independent, solving the score normalization problem between
//! TF-IDF (unbounded) and vector similarity (0-1).
//!
//! ## Design (2026-01)
//! - NO query preprocessing - consistent NLP for both paths:
//!   - Vector: client embeds original query → matches original-embedded docs
//!   - Fulltext: Snowball stems query → matches Snowball-stemmed index
//! - Reranking: heading boost, phrase match, keyword density
//! - MMR diversity reranking: embedding cosine similarity based

use crate::adapter::{FulltextSearchOptions, IronBaseAdapter};
use crate::error::{McpError, Result};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use super::helpers::{parse_projection_value, validate_collection_name};
use super::params::{HybridSearchParams, ParseParams};

/// RRF constant - empirically optimal value (Cormack et al., 2009)
const RRF_K: f64 = 60.0;

/// Maximum internal limit to prevent OOM (CLAUDE.md compliance)
/// Even if user requests limit=100000, we cap internal processing at this value
const MAX_INTERNAL_LIMIT: usize = 1000;

/// Dispatch hybrid tool calls
pub fn dispatch(name: &str, params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    match name {
        "hybrid_search" => handle_hybrid_search(params, adapter),
        _ => Err(McpError::invalid_params(format!(
            "Unknown hybrid tool: {}",
            name
        ))),
    }
}

/// Intermediate result structure for pipeline processing
#[derive(Debug)]
struct FusedResult {
    #[allow(dead_code)] // Used in tests
    id: String,
    doc: Value,
    rrf_score: f64,
    final_score: f64,
    rerank_boost: f64,
    v_rank: usize,
    t_rank: usize,
    v_score: Option<f32>,
    t_score: Option<f64>,
}

/// Handle hybrid_search - RRF fusion of vector and fulltext search
/// With reranking and MMR diversity reranking (NO query preprocessing for consistency)
fn handle_hybrid_search(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    if params.get("dedup_threshold").is_some() {
        return Err(McpError::invalid_params(
            "Parameter 'dedup_threshold' has been removed. Use 'mmr_lambda' (0.0-1.0) instead.",
        ));
    }
    let p: HybridSearchParams = HybridSearchParams::parse(params)?;
    validate_collection_name(&p.collection)?;

    // ========================================================================
    // NO preprocessing - consistent NLP design:
    // - Vector: client embeds original query → matches original-embedded docs
    // - Fulltext: Snowball internally stems query → matches Snowball-stemmed index
    // ========================================================================

    // Internal limit multiplier for better fusion coverage
    // Need more results when reranking/deduplication will filter some out
    // Cap at MAX_INTERNAL_LIMIT to prevent OOM (CLAUDE.md compliance)
    let internal_limit = (p.limit * 3).min(MAX_INTERNAL_LIMIT);

    // ========================================================================
    // 2. Vector search → get ranks
    // ========================================================================
    let query_vector: Vec<f32> = p.vector.iter().map(|&v| v as f32).collect();
    let vector_results = if let Some(ref filter) = p.filter {
        adapter.vector_search_with_filter(
            &p.collection,
            &p.vector_field,
            &query_vector,
            filter,
            internal_limit,
        )?
    } else {
        adapter.vector_search(
            &p.collection,
            &p.vector_field,
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
    // 3. Fulltext search → get ranks (original query - Snowball handles stemming)
    // ========================================================================
    let text_options = FulltextSearchOptions {
        limit: Some(internal_limit),
        skip: None,
        min_score: None,
        projection: None, // Get full docs for merging
        filter: p.filter.clone(),
        and_mode: false, // Hybrid uses RRF fusion, not AND mode
        highlight: false,
        highlight_context: None,
        highlight_max_snippets: None,
    };

    // Use original query - Snowball stemmer in fulltext handles NLP consistently
    let text_results =
        adapter.fulltext_search(&p.collection, &p.text_field, &p.query, text_options)?;

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
    // 4. RRF Fusion (with pre-allocated capacity for OOM protection)
    // ========================================================================
    // Max unique IDs = vector_ranks + text_ranks (worst case: no overlap)
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
        let rrf_score = p.vector_weight * (1.0 / (RRF_K + v_rank as f64))
            + p.fulltext_weight * (1.0 / (RRF_K + t_rank as f64));

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
    // 5. Reranking (optional)
    // ========================================================================
    if p.rerank {
        // Use original query for both phrase match and keyword density
        rerank_results(&mut fused, &p.query, &p.query, &p.text_field);
    }

    // ========================================================================
    // 6. MMR diversity reranking (optional) — replaces prefix-based dedup
    // ========================================================================
    let dedup_removed = if p.deduplicate {
        mmr_reorder(&mut fused, &p.vector_field, p.mmr_lambda, p.limit)
    } else {
        fused.truncate(p.limit);
        0
    };

    // ========================================================================
    // 7. Apply projection and build response
    // ========================================================================
    let projection = parse_projection_value(p.projection)?;

    let results: Vec<Value> = fused
        .into_iter()
        .map(|item| {
            // Apply projection if specified
            let doc_projected = if let Some(ref proj) = projection {
                apply_projection(&item.doc, proj)
            } else {
                item.doc
            };

            // Build result with metadata
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

            result
        })
        .collect();

    Ok(json!({
        "results": results,
        "count": results.len(),
        "algorithm": "rrf",
        "rrf_k": RRF_K,
        "weights": {
            "vector": p.vector_weight,
            "fulltext": p.fulltext_weight
        },
        "query": p.query,
        "dedup_removed": dedup_removed,
        "nlp_design": "consistent: vector=original, fulltext=snowball"
    }))
}

// ============================================================================
// Reranking
// ============================================================================

/// Strip punctuation for phrase matching (fixes exact phrase bug)
fn strip_punctuation(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Rerank results by phrase match, keyword density, and content length
///
/// Simplified reranking using only text_field (no extra fields needed):
/// - Exact phrase boost: 1.3x if query found in content (punctuation ignored)
/// - Keyword density: 1.0-1.1x based on query word occurrence ratio
/// - Content length penalty: 0.8x for content < 100 chars
///
/// Uses original_query for phrase matching (what user typed)
/// and processed_query for keyword density (matches fulltext search behavior)
fn rerank_results(
    results: &mut [FusedResult],
    original_query: &str,
    processed_query: &str,
    text_field: &str,
) {
    // Build query word sets from PROCESSED query (consistent with fulltext search)
    let query_words: HashSet<String> = processed_query
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= 3)
        .map(|s| s.to_string())
        .collect();

    // Query for exact phrase matching (punctuation stripped) - use ORIGINAL query
    let query_normalized = strip_punctuation(&original_query.to_lowercase());

    for item in results.iter_mut() {
        let mut boost = 1.0;

        let content = item
            .doc
            .get(text_field)
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let content_lower = content.to_lowercase();
        let content_normalized = strip_punctuation(&content_lower);

        // 1. Exact phrase boost (1.3x) - query found in content (punctuation ignored)
        // Use chars().count() for UTF-8 correctness
        if query_normalized.chars().count() > 10 && content_normalized.contains(&query_normalized) {
            boost *= 1.3;
        }

        // 2. Keyword density (1.0-1.1x)
        let content_words: Vec<&str> = content_lower
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .collect();

        if !content_words.is_empty() {
            #[allow(clippy::unnecessary_to_owned)]
            let matches = content_words
                .iter()
                .filter(|w| query_words.contains(&w.to_string()))
                .count();
            let density = matches as f64 / content_words.len() as f64;
            boost *= 1.0 + density.min(0.1); // Cap at 1.1x
        }

        // 3. Content length penalty (0.8x for short content)
        if content.len() < 100 {
            boost *= 0.8;
        }

        item.rerank_boost = boost;
        item.final_score = item.rrf_score * boost;
    }

    // Re-sort by final_score (descending)
    results.sort_by(|a, b| {
        b.final_score
            .partial_cmp(&a.final_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

// ============================================================================
// MMR (Maximal Marginal Relevance) Reordering
// ============================================================================

/// Extract embedding vector from a document's embedding field (JSON array → Vec<f32>)
fn extract_embedding(doc: &Value, embedding_field: &str) -> Option<Vec<f32>> {
    doc.get(embedding_field)
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_f64().map(|f| f as f32))
                .collect()
        })
}

/// MMR reordering: selects `limit` results balancing relevance and diversity.
///
/// Algorithm: greedily selects the candidate that maximizes:
///   mmr(c) = λ * relevance(c) - (1-λ) * max_sim(c, selected)
///
/// where relevance is the normalized final_score and max_sim is the maximum
/// cosine similarity between c's embedding and any already-selected result.
///
/// Results without embeddings are kept in relevance order (no diversity penalty).
///
/// Returns the number of candidates not selected (analogous to dedup_removed).
fn mmr_reorder(
    results: &mut Vec<FusedResult>,
    embedding_field: &str,
    lambda: f64,
    limit: usize,
) -> usize {
    let original_len = results.len();
    if results.is_empty() || limit == 0 {
        results.clear();
        return original_len;
    }

    let target = limit.min(original_len);

    // Extract embeddings (None if missing from doc)
    let embeddings: Vec<Option<Vec<f32>>> = results
        .iter()
        .map(|r| extract_embedding(&r.doc, embedding_field))
        .collect();

    // Normalize relevance scores to [0, 1] for MMR formula
    let max_score = results
        .iter()
        .map(|r| r.final_score)
        .fold(f64::NEG_INFINITY, f64::max);
    let min_score = results
        .iter()
        .map(|r| r.final_score)
        .fold(f64::INFINITY, f64::min);
    let score_range = max_score - min_score;

    // Track which indices are selected and which are candidates
    let mut selected_indices: Vec<usize> = Vec::with_capacity(target);
    let mut candidate_mask: Vec<bool> = vec![true; original_len];

    // Select first: highest relevance (results are pre-sorted by final_score)
    selected_indices.push(0);
    candidate_mask[0] = false;

    // Greedy MMR selection
    while selected_indices.len() < target {
        let mut best_idx = None;
        let mut best_mmr = f64::NEG_INFINITY;

        for (i, is_candidate) in candidate_mask.iter().enumerate() {
            if !is_candidate {
                continue;
            }

            // Normalized relevance [0, 1]
            let relevance = if score_range > 0.0 {
                (results[i].final_score - min_score) / score_range
            } else {
                1.0
            };

            // Max similarity to any selected result
            let max_sim = if let Some(ref emb_i) = embeddings[i] {
                selected_indices
                    .iter()
                    .filter_map(|&si| {
                        embeddings[si].as_ref().map(|emb_s| {
                            if emb_i.len() == emb_s.len() && !emb_i.is_empty() {
                                ironbase_core::vector::simd::cosine_similarity(emb_i, emb_s)
                                    as f64
                            } else {
                                0.0 // Dimension mismatch → no penalty
                            }
                        })
                    })
                    .fold(f64::NEG_INFINITY, f64::max)
            } else {
                0.0 // No embedding → no diversity penalty
            };
            let max_sim = if max_sim == f64::NEG_INFINITY {
                0.0
            } else {
                max_sim
            };

            let mmr_score = lambda * relevance - (1.0 - lambda) * max_sim;

            if mmr_score > best_mmr {
                best_mmr = mmr_score;
                best_idx = Some(i);
            }
        }

        match best_idx {
            Some(idx) => {
                selected_indices.push(idx);
                candidate_mask[idx] = false;
            }
            None => break, // No more candidates
        }
    }

    // Reorder results: keep only selected, in selection order
    let reordered: Vec<FusedResult> = selected_indices
        .into_iter()
        .map(|i| {
            // We need to take ownership; use a placeholder swap
            let mut placeholder = FusedResult {
                id: String::new(),
                doc: Value::Null,
                rrf_score: 0.0,
                final_score: 0.0,
                rerank_boost: 0.0,
                v_rank: 0,
                t_rank: 0,
                v_score: None,
                t_score: None,
            };
            std::mem::swap(&mut results[i], &mut placeholder);
            placeholder
        })
        .collect();

    *results = reordered;
    original_len - results.len()
}

/// Convert Value _id to String for HashMap key
fn id_to_string(id: &Value) -> Option<String> {
    match id {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => Some(id.to_string()),
    }
}

/// Apply MongoDB-style projection to a document
fn apply_projection(doc: &Value, projection: &HashMap<String, i32>) -> Value {
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
        // Verify RRF_K is 60 (empirically optimal)
        assert_eq!(RRF_K, 60.0);
    }

    // -------------------------------------------------------------------------
    // id_to_string Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_id_to_string_string() {
        let id = json!("doc123");
        assert_eq!(id_to_string(&id), Some("doc123".to_string()));
    }

    #[test]
    fn test_id_to_string_number() {
        let id = json!(42);
        assert_eq!(id_to_string(&id), Some("42".to_string()));
    }

    #[test]
    fn test_id_to_string_uuid() {
        let id = json!("a1aaed76-f22c-472f-82cb-a9ed005cb624");
        assert_eq!(
            id_to_string(&id),
            Some("a1aaed76-f22c-472f-82cb-a9ed005cb624".to_string())
        );
    }

    #[test]
    fn test_id_to_string_object() {
        // ObjectId-style (MongoDB)
        let id = json!({"$oid": "507f1f77bcf86cd799439011"});
        // Should convert to string representation
        assert!(id_to_string(&id).is_some());
    }

    // -------------------------------------------------------------------------
    // apply_projection Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_projection_include_mode() {
        let doc = json!({
            "_id": "doc1",
            "name": "Alice",
            "email": "alice@test.com",
            "age": 30
        });
        let mut proj = HashMap::new();
        proj.insert("name".to_string(), 1);
        proj.insert("email".to_string(), 1);

        let result = apply_projection(&doc, &proj);

        assert_eq!(result["_id"], "doc1"); // _id included by default
        assert_eq!(result["name"], "Alice");
        assert_eq!(result["email"], "alice@test.com");
        assert!(result.get("age").is_none());
    }

    #[test]
    fn test_projection_exclude_mode() {
        let doc = json!({
            "_id": "doc1",
            "name": "Alice",
            "content": "very long content...",
            "embedding": [0.1, 0.2, 0.3]
        });
        let mut proj = HashMap::new();
        proj.insert("content".to_string(), 0);
        proj.insert("embedding".to_string(), 0);

        let result = apply_projection(&doc, &proj);

        assert_eq!(result["_id"], "doc1");
        assert_eq!(result["name"], "Alice");
        assert!(result.get("content").is_none());
        assert!(result.get("embedding").is_none());
    }

    #[test]
    fn test_projection_exclude_id() {
        let doc = json!({
            "_id": "doc1",
            "name": "Alice"
        });
        let mut proj = HashMap::new();
        proj.insert("name".to_string(), 1);
        proj.insert("_id".to_string(), 0);

        let result = apply_projection(&doc, &proj);

        assert!(result.get("_id").is_none());
        assert_eq!(result["name"], "Alice");
    }

    #[test]
    fn test_projection_empty() {
        let doc = json!({
            "_id": "doc1",
            "name": "Alice"
        });
        let proj = HashMap::new();

        let result = apply_projection(&doc, &proj);

        // Empty projection in exclude mode = include all
        assert_eq!(result["_id"], "doc1");
        assert_eq!(result["name"], "Alice");
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
        assert_eq!(p.vector_weight, 0.7);
        assert_eq!(p.fulltext_weight, 0.3);
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
        assert_eq!(p.vector_weight, 0.5); // default
        assert_eq!(p.fulltext_weight, 0.5); // default
                                            // v2 defaults
        assert!(p.rerank); // default: true
        assert!(p.deduplicate); // default: true
        assert!((p.mmr_lambda - 0.5).abs() < f64::EPSILON); // default
        assert!(p.language.is_none());
        assert!(p.filter.is_none()); // default: no filter
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
            "language": "hungarian",
            "rerank": true,
            "deduplicate": true,
            "mmr_lambda": 0.7
        });

        let p: HybridSearchParams = HybridSearchParams::parse(params).unwrap();
        assert_eq!(p.language, Some("hungarian".to_string()));
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
    fn test_params_missing_required() {
        let params = json!({
            "collection": "test",
            "vector_field": "embedding"
            // missing: text_field, vector, query
        });

        let result = HybridSearchParams::parse(params);
        assert!(result.is_err());
    }

    // -------------------------------------------------------------------------
    // Reranking Tests
    // -------------------------------------------------------------------------

    fn make_fused_result(id: &str, content: &str, heading: &str, rrf_score: f64) -> FusedResult {
        FusedResult {
            id: id.to_string(),
            doc: json!({
                "_id": id,
                "content": content,
                "heading": heading
            }),
            rrf_score,
            final_score: rrf_score,
            rerank_boost: 1.0,
            v_rank: 1,
            t_rank: 1,
            v_score: Some(0.9),
            t_score: Some(10.0),
        }
    }

    #[test]
    fn test_rerank_exact_phrase_boost() {
        let mut results = vec![
            make_fused_result("doc1", "Some words about stuff", "Title", 0.01),
            make_fused_result(
                "doc2",
                "This document contains the exact phrase test query here",
                "Title",
                0.01,
            ),
        ];

        rerank_results(
            &mut results,
            "the exact phrase test query",
            "the exact phrase test query",
            "content",
        );

        // doc2 should be boosted (contains exact phrase)
        assert!(results[0].id == "doc2");
        assert!(results[0].rerank_boost > 1.0);
    }

    #[test]
    fn test_rerank_exact_phrase_ignores_punctuation() {
        // Tests the bug fix: punctuation should not break phrase matching
        // Content must be > 100 chars to avoid short content penalty
        let mut results = vec![
            make_fused_result(
                "doc1",
                "Some other content here that is long enough to avoid the short content penalty threshold of one hundred characters",
                "Title",
                0.01,
            ),
            make_fused_result(
                "doc2",
                "Ez a dokumentum tartalmazza: milyen lépései vannak a kalibrálásnak - itt van a kalibrálás leírása részletesen kifejtve",
                "Title",
                0.01,
            ),
        ];

        // Query has "?" at end, content has ":" before and "-" after
        // original_query and processed_query same for this test (no preprocessing)
        rerank_results(
            &mut results,
            "milyen lépései vannak a kalibrálásnak?",
            "milyen lépései vannak a kalibrálásnak?",
            "content",
        );

        // doc2 should be boosted (phrase matches ignoring punctuation)
        assert!(results[0].id == "doc2");
        assert!(results[0].rerank_boost >= 1.3); // exact phrase boost (no short penalty)
    }

    #[test]
    fn test_rerank_short_content_penalty() {
        let mut results = vec![
            make_fused_result("doc1", "Short", "Title", 0.01),
            make_fused_result(
                "doc2",
                "This is a much longer content that exceeds the 100 character threshold and should not receive a penalty for being too short",
                "Title",
                0.01,
            ),
        ];

        rerank_results(&mut results, "test", "test", "content");

        // doc1 should be penalized (content < 100 chars)
        let doc1 = results.iter().find(|r| r.id == "doc1").unwrap();
        let doc2 = results.iter().find(|r| r.id == "doc2").unwrap();
        assert!(doc1.rerank_boost < doc2.rerank_boost);
    }

    // -------------------------------------------------------------------------
    // MMR Reordering Tests
    // -------------------------------------------------------------------------

    fn make_fused_with_embedding(
        id: &str,
        content: &str,
        embedding: Vec<f32>,
        score: f64,
    ) -> FusedResult {
        FusedResult {
            id: id.to_string(),
            doc: json!({
                "_id": id,
                "content": content,
                "embedding": embedding
            }),
            rrf_score: score,
            final_score: score,
            rerank_boost: 1.0,
            v_rank: 1,
            t_rank: 1,
            v_score: Some(0.9),
            t_score: Some(10.0),
        }
    }

    #[test]
    fn test_mmr_selects_diverse_results() {
        // doc1 and doc2 have identical embeddings, doc3 is different
        let mut results = vec![
            make_fused_with_embedding("doc1", "Content A", vec![1.0, 0.0, 0.0], 0.03),
            make_fused_with_embedding("doc2", "Content A copy", vec![1.0, 0.0, 0.0], 0.02),
            make_fused_with_embedding("doc3", "Content B", vec![0.0, 1.0, 0.0], 0.01),
        ];

        let removed = mmr_reorder(&mut results, "embedding", 0.5, 2);

        assert_eq!(removed, 1);
        assert_eq!(results.len(), 2);
        // doc1 (highest score) should be first
        assert_eq!(results[0].id, "doc1");
        // doc3 (diverse) should be preferred over doc2 (duplicate of doc1)
        assert_eq!(results[1].id, "doc3");
    }

    #[test]
    fn test_mmr_pure_relevance() {
        // λ=1.0 → pure relevance ordering
        let mut results = vec![
            make_fused_with_embedding("doc1", "Content A", vec![1.0, 0.0, 0.0], 0.03),
            make_fused_with_embedding("doc2", "Content A copy", vec![1.0, 0.0, 0.0], 0.02),
            make_fused_with_embedding("doc3", "Content B", vec![0.0, 1.0, 0.0], 0.01),
        ];

        mmr_reorder(&mut results, "embedding", 1.0, 3);

        // Pure relevance → same order as input (sorted by score)
        assert_eq!(results[0].id, "doc1");
        assert_eq!(results[1].id, "doc2");
        assert_eq!(results[2].id, "doc3");
    }

    #[test]
    fn test_mmr_without_embeddings() {
        // Docs without embedding field → pure relevance order (no diversity penalty)
        let mut results = vec![
            make_fused_result("doc1", "Content A", "Title", 0.03),
            make_fused_result("doc2", "Content B", "Title", 0.02),
            make_fused_result("doc3", "Content C", "Title", 0.01),
        ];

        let removed = mmr_reorder(&mut results, "embedding", 0.5, 2);

        assert_eq!(removed, 1);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].id, "doc1");
        assert_eq!(results[1].id, "doc2");
    }

    #[test]
    fn test_mmr_limit_larger_than_results() {
        let mut results = vec![
            make_fused_with_embedding("doc1", "A", vec![1.0, 0.0], 0.02),
            make_fused_with_embedding("doc2", "B", vec![0.0, 1.0], 0.01),
        ];

        let removed = mmr_reorder(&mut results, "embedding", 0.5, 10);

        // All results kept (limit > available)
        assert_eq!(removed, 0);
        assert_eq!(results.len(), 2);
    }
}
