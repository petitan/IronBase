//! Shared fusion utilities for hybrid and RAG search
//!
//! Contains the reranking pipeline, MMR diversity reranking, and common helpers
//! shared between `hybrid_search` and `rag_search` tools.
//!
//! Extracted from duplicated code in hybrid.rs and rag.rs (2026-02-18).

use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

// ============================================================================
// Types
// ============================================================================

/// Intermediate result structure for RRF fusion pipeline processing
#[derive(Debug)]
pub(crate) struct FusedResult {
    #[allow(dead_code)] // Used in tests
    pub(crate) id: String,
    pub(crate) doc: Value,
    pub(crate) rrf_score: f64,
    pub(crate) final_score: f64,
    pub(crate) rerank_boost: f64,
    pub(crate) v_rank: usize,
    pub(crate) t_rank: usize,
    pub(crate) v_score: Option<f32>,
    pub(crate) t_score: Option<f64>,
}

// ============================================================================
// Common helpers
// ============================================================================

/// Convert Value _id to String for HashMap key
pub(crate) fn id_to_string(id: &Value) -> Option<String> {
    match id {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => Some(id.to_string()),
    }
}

/// Apply MongoDB-style projection to a document
pub(crate) fn apply_projection(doc: &Value, projection: &HashMap<String, i32>) -> Value {
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

// ============================================================================
// Reranking
// ============================================================================

/// Strip punctuation for phrase matching (optimized: single allocation)
fn strip_punctuation(s: &str) -> String {
    let filtered: String = s
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect();
    // Normalize whitespace in-place without extra Vec allocation
    let mut result = String::with_capacity(filtered.len());
    let mut prev_space = true; // Start true to skip leading spaces
    for c in filtered.chars() {
        if c.is_whitespace() {
            if !prev_space {
                result.push(' ');
                prev_space = true;
            }
        } else {
            result.push(c);
            prev_space = false;
        }
    }
    // Trim trailing space
    if result.ends_with(' ') {
        result.pop();
    }
    result
}

/// Rerank results by phrase match, keyword density, content length, and title match
///
/// Reranking boosts:
/// - Exact phrase boost: 1.5x if query found in content (punctuation ignored)
/// - Keyword density: 1.0-1.3x based on query word occurrence ratio
/// - Content length penalty: 0.8x for content < 50 chars
/// - Title match boost: up to 1.5x if query words appear in title field
pub(crate) fn rerank_results(
    results: &mut [FusedResult],
    query: &str,
    text_field: &str,
    title_field: Option<&str>,
) {
    // Build query word sets — use chars().count() for UTF-8 correctness
    let query_words: HashSet<String> = query
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= 3)
        .map(|s| s.to_string())
        .collect();

    // Query for exact phrase matching (punctuation stripped)
    let query_normalized = strip_punctuation(&query.to_lowercase());

    for item in results.iter_mut() {
        let mut boost = 1.0;

        let content = item
            .doc
            .get(text_field)
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let content_lower = content.to_lowercase();
        let content_normalized = strip_punctuation(&content_lower);

        // 1. Exact phrase boost (1.5x) - query found in content (punctuation ignored)
        // Use chars().count() for UTF-8 correctness
        if query_normalized.chars().count() > 10 && content_normalized.contains(&query_normalized) {
            boost *= 1.5;
        }

        // 2. Keyword density (1.0-1.3x)
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
            boost *= 1.0 + density.min(0.3); // Cap at 1.3x
        }

        // 3. Content length penalty (0.8x for short content)
        if content.len() < 50 {
            boost *= 0.8;
        }

        // 4. Title match boost (up to 1.5x) — query words in title
        if let Some(tf) = title_field {
            if let Some(title) = item.doc.get(tf).and_then(|v| v.as_str()) {
                let title_lower = title.to_lowercase();
                let title_matches = query_words
                    .iter()
                    .filter(|w| title_lower.contains(w.as_str()))
                    .count();
                if title_matches > 0 && !query_words.is_empty() {
                    let title_ratio = title_matches as f64 / query_words.len() as f64;
                    boost *= 1.0 + 0.5 * title_ratio; // 1.0–1.5x scale
                }
            }
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
pub(crate) fn mmr_reorder(
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

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_strip_punctuation() {
        assert_eq!(strip_punctuation("hello, world!"), "hello world");
        assert_eq!(strip_punctuation("a. b. c."), "a b c");
        assert_eq!(strip_punctuation("király   mesék"), "király mesék");
    }

    #[test]
    fn test_strip_punctuation_leading_trailing() {
        assert_eq!(strip_punctuation("  hello  "), "hello");
        assert_eq!(strip_punctuation("...test..."), "test");
    }

    #[test]
    fn test_id_to_string_string() {
        assert_eq!(id_to_string(&json!("abc")), Some("abc".to_string()));
    }

    #[test]
    fn test_id_to_string_number() {
        assert_eq!(id_to_string(&json!(123)), Some("123".to_string()));
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
        let id = json!({"$oid": "507f1f77bcf86cd799439011"});
        assert!(id_to_string(&id).is_some());
    }

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
            "content",
            None,
        );

        // doc2 should be boosted (contains exact phrase)
        assert!(results[0].id == "doc2");
        assert!(results[0].rerank_boost > 1.0);
    }

    #[test]
    fn test_rerank_exact_phrase_ignores_punctuation() {
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

        rerank_results(
            &mut results,
            "milyen lépései vannak a kalibrálásnak?",
            "content",
            None,
        );

        // doc2 should be boosted (phrase matches ignoring punctuation)
        assert!(results[0].id == "doc2");
        assert!(results[0].rerank_boost >= 1.5);
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

        rerank_results(&mut results, "test", "content", None);

        // doc1 should be penalized (content < 50 chars)
        let doc1 = results.iter().find(|r| r.id == "doc1").unwrap();
        let doc2 = results.iter().find(|r| r.id == "doc2").unwrap();
        assert!(doc1.rerank_boost < doc2.rerank_boost);
    }

    #[test]
    fn test_rerank_title_match_boost() {
        let mut results = vec![
            FusedResult {
                id: "doc1".to_string(),
                doc: json!({
                    "_id": "doc1",
                    "content": "Ez egy hosszabb tartalom ami nem tartalmaz releváns szavakat de elég hosszú az ötven karakteres küszöbhöz",
                    "title": "Nem kapcsolódó cím"
                }),
                rrf_score: 0.02,
                final_score: 0.02,
                rerank_boost: 1.0,
                v_rank: 1,
                t_rank: 1,
                v_score: Some(0.9),
                t_score: Some(10.0),
            },
            FusedResult {
                id: "doc2".to_string(),
                doc: json!({
                    "_id": "doc2",
                    "content": "Ez egy hosszabb tartalom ami nem tartalmaz releváns szavakat de elég hosszú az ötven karakteres küszöbhöz",
                    "title": "Fékerőmérő kalibrálás és beállítás"
                }),
                rrf_score: 0.02,
                final_score: 0.02,
                rerank_boost: 1.0,
                v_rank: 2,
                t_rank: 2,
                v_score: Some(0.8),
                t_score: Some(8.0),
            },
        ];

        rerank_results(
            &mut results,
            "fékerőmérő kalibrálás",
            "content",
            Some("title"),
        );

        // doc2 should be boosted because title contains both query words
        assert_eq!(results[0].id, "doc2");
        assert!(results[0].rerank_boost > 1.0);
        let doc1 = results.iter().find(|r| r.id == "doc1").unwrap();
        assert!(doc1.rerank_boost <= 1.0 || doc1.rerank_boost < results[0].rerank_boost);
    }

    #[test]
    fn test_rerank_title_match_partial() {
        let mut results = vec![
            FusedResult {
                id: "doc1".to_string(),
                doc: json!({
                    "_id": "doc1",
                    "content": "Tartalom ami elég hosszú ahhoz hogy ne kapjon rövid tartalom büntetést a rerankertől",
                    "title": "Fékerőmérő javítás"  // 1 of 2 query words
                }),
                rrf_score: 0.02,
                final_score: 0.02,
                rerank_boost: 1.0,
                v_rank: 1,
                t_rank: 1,
                v_score: Some(0.9),
                t_score: Some(10.0),
            },
        ];

        rerank_results(
            &mut results,
            "fékerőmérő kalibrálás",
            "content",
            Some("title"),
        );

        // Partial match: 1/2 words → boost = 1.0 + 0.5 * 0.5 = 1.25x
        assert!(results[0].rerank_boost >= 1.2);
        assert!(results[0].rerank_boost < 1.5);
    }

    #[test]
    fn test_rerank_no_title_field() {
        let mut results = vec![
            FusedResult {
                id: "doc1".to_string(),
                doc: json!({
                    "_id": "doc1",
                    "content": "Tartalom ami elég hosszú ahhoz hogy ne kapjon rövid tartalom büntetést a rerankertől",
                    "title": "Fékerőmérő kalibrálás"
                }),
                rrf_score: 0.02,
                final_score: 0.02,
                rerank_boost: 1.0,
                v_rank: 1,
                t_rank: 1,
                v_score: Some(0.9),
                t_score: Some(10.0),
            },
        ];

        rerank_results(
            &mut results,
            "fékerőmérő kalibrálás",
            "content",
            None, // no title_field
        );

        // Without title_field, boost should be close to 1.0 (only density boost applies)
        assert!(results[0].rerank_boost < 1.4);
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
        let mut results = vec![
            make_fused_with_embedding("doc1", "Content A", vec![1.0, 0.0, 0.0], 0.03),
            make_fused_with_embedding("doc2", "Content A copy", vec![1.0, 0.0, 0.0], 0.02),
            make_fused_with_embedding("doc3", "Content B", vec![0.0, 1.0, 0.0], 0.01),
        ];

        let removed = mmr_reorder(&mut results, "embedding", 0.5, 2);

        assert_eq!(removed, 1);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].id, "doc1");
        assert_eq!(results[1].id, "doc3");
    }

    #[test]
    fn test_mmr_pure_relevance() {
        let mut results = vec![
            make_fused_with_embedding("doc1", "Content A", vec![1.0, 0.0, 0.0], 0.03),
            make_fused_with_embedding("doc2", "Content A copy", vec![1.0, 0.0, 0.0], 0.02),
            make_fused_with_embedding("doc3", "Content B", vec![0.0, 1.0, 0.0], 0.01),
        ];

        mmr_reorder(&mut results, "embedding", 1.0, 3);

        assert_eq!(results[0].id, "doc1");
        assert_eq!(results[1].id, "doc2");
        assert_eq!(results[2].id, "doc3");
    }

    #[test]
    fn test_mmr_without_embeddings() {
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

        assert_eq!(removed, 0);
        assert_eq!(results.len(), 2);
    }
}
