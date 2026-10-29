//! MMR (Maximal Marginal Relevance) reordering.

use serde_json::Value;

use super::FusedResult;

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
pub fn mmr_reorder(
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

    // Incremental max similarity cache: max_sim_cache[i] = max cosine similarity
    // between candidate i and any already-selected result. Updated incrementally
    // when a new result is selected, avoiding O(selected) re-scan per candidate.
    let mut max_sim_cache: Vec<f64> = vec![0.0; original_len];

    // Select first: highest relevance (results are pre-sorted by final_score)
    selected_indices.push(0);
    candidate_mask[0] = false;

    // Update cache for the first selected result
    if let Some(ref emb_first) = embeddings[0] {
        for (i, is_candidate) in candidate_mask.iter().enumerate() {
            if !is_candidate {
                continue;
            }
            if let Some(ref emb_i) = embeddings[i] {
                if emb_i.len() == emb_first.len() && !emb_i.is_empty() {
                    let sim = crate::vector::simd::cosine_similarity(emb_i, emb_first) as f64;
                    if sim > max_sim_cache[i] {
                        max_sim_cache[i] = sim;
                    }
                }
            }
        }
    }

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

            let mmr_score = lambda * relevance - (1.0 - lambda) * max_sim_cache[i];

            if mmr_score > best_mmr {
                best_mmr = mmr_score;
                best_idx = Some(i);
            }
        }

        match best_idx {
            Some(idx) => {
                selected_indices.push(idx);
                candidate_mask[idx] = false;

                // Update cache: compare each remaining candidate against newly selected
                if let Some(ref emb_new) = embeddings[idx] {
                    for (i, is_candidate) in candidate_mask.iter().enumerate() {
                        if !is_candidate {
                            continue;
                        }
                        if let Some(ref emb_i) = embeddings[i] {
                            if emb_i.len() == emb_new.len() && !emb_i.is_empty() {
                                let sim =
                                    crate::vector::simd::cosine_similarity(emb_i, emb_new) as f64;
                                if sim > max_sim_cache[i] {
                                    max_sim_cache[i] = sim;
                                }
                            }
                        }
                    }
                }
            }
            None => break, // No more candidates
        }
    }

    // Reorder results: keep only selected, in selection order
    let reordered: Vec<FusedResult> = selected_indices
        .into_iter()
        .map(|i| {
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
