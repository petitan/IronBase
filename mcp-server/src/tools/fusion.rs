//! Shared fusion utilities for hybrid and RAG search
//!
//! Contains the reranking pipeline, MMR diversity reranking, and common helpers
//! shared between hybrid search tools.
//!
//! Extracted from duplicated code in hybrid.rs and rag.rs (2026-02-18).

use crate::adapter::{FindOptions, IronBaseAdapter, QualificationResult};
use crate::error::{McpError, Result};
use crate::tools::defaults::MAX_INTERNAL_LIMIT;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

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
// Adjacent Chunk Merge
// ============================================================================

/// Merge adjacent chunks from the same source document into single results.
///
/// RAG chunking with overlap (~100 chars) can cause the same text to appear in
/// consecutive chunks. When both match, they waste top-K slots with redundant
/// content. This post-processing step merges runs of consecutive chunk_index
/// values (same doc_id) into a single result with concatenated text (overlap removed).
///
/// Placement: after reranking, before MMR. This ensures merged results carry the
/// best score from the run, and MMR sees fewer near-duplicates.
///
/// Returns the number of results removed by merging.
pub(crate) fn merge_adjacent_chunks(results: &mut Vec<FusedResult>, text_field: &str) -> usize {
    if results.len() < 2 {
        return 0;
    }

    // Collect (doc_id, chunk_index, results_index) for RAG chunks only
    let mut chunk_info: Vec<(String, u64, usize)> = Vec::new();
    for (i, item) in results.iter().enumerate() {
        let doc_id = item.doc.get("doc_id").and_then(|v| v.as_str());
        let chunk_index = item.doc.get("chunk_index").and_then(|v| v.as_u64());
        if let (Some(did), Some(ci)) = (doc_id, chunk_index) {
            chunk_info.push((did.to_string(), ci, i));
        }
    }

    if chunk_info.is_empty() {
        return 0;
    }

    // Group by doc_id
    let mut groups: HashMap<String, Vec<(u64, usize)>> = HashMap::with_capacity(chunk_info.len());
    for (doc_id, ci, idx) in chunk_info {
        groups.entry(doc_id).or_default().push((ci, idx));
    }

    // For each group, sort by chunk_index and find consecutive runs
    let mut indices_to_remove: HashSet<usize> = HashSet::new();

    for (_doc_id, mut entries) in groups {
        if entries.len() < 2 {
            continue;
        }
        entries.sort_by_key(|(ci, _)| *ci);

        // Detect consecutive runs
        let mut run_start = 0;
        while run_start < entries.len() {
            let mut run_end = run_start;
            while run_end + 1 < entries.len() && entries[run_end + 1].0 == entries[run_end].0 + 1 {
                run_end += 1;
            }

            if run_end > run_start {
                // Found a consecutive run of length > 1 — merge
                let run = &entries[run_start..=run_end];

                // Find the entry with the best final_score in this run
                let best_idx_in_run = run
                    .iter()
                    .max_by(|a, b| {
                        results[a.1]
                            .final_score
                            .partial_cmp(&results[b.1].final_score)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .unwrap()
                    .1;

                // Build merged text from all chunks in the run (overlap removed)
                let mut merged_text = String::new();
                let mut min_start_char: Option<u64> = None;
                let mut max_end_char: Option<u64> = None;

                for (i, &(_, res_idx)) in run.iter().enumerate() {
                    let doc = &results[res_idx].doc;
                    let chunk_text = doc.get(text_field).and_then(|v| v.as_str()).unwrap_or("");

                    if i == 0 {
                        merged_text.push_str(chunk_text);
                    } else {
                        // Calculate overlap from start_char/end_char if available
                        let prev_end = results[run[i - 1].1]
                            .doc
                            .get("end_char")
                            .and_then(|v| v.as_u64());
                        let curr_start = doc.get("start_char").and_then(|v| v.as_u64());

                        let overlap_chars = match (prev_end, curr_start) {
                            (Some(pe), Some(cs)) if pe > cs => (pe - cs) as usize,
                            _ => 0,
                        };

                        // Skip overlap characters from the beginning of this chunk's text
                        let text_to_append =
                            if overlap_chars > 0 && overlap_chars < chunk_text.chars().count() {
                                // Find byte offset for char boundary
                                let byte_offset = chunk_text
                                    .char_indices()
                                    .nth(overlap_chars)
                                    .map(|(idx, _)| idx)
                                    .unwrap_or(chunk_text.len());
                                &chunk_text[byte_offset..]
                            } else {
                                chunk_text
                            };

                        merged_text.push_str(text_to_append);
                    }

                    // Track start_char/end_char range
                    if let Some(sc) = doc.get("start_char").and_then(|v| v.as_u64()) {
                        min_start_char = Some(min_start_char.map_or(sc, |m: u64| m.min(sc)));
                    }
                    if let Some(ec) = doc.get("end_char").and_then(|v| v.as_u64()) {
                        max_end_char = Some(max_end_char.map_or(ec, |m: u64| m.max(ec)));
                    }
                }

                // Update the best-scored result's doc with merged content
                if let Value::Object(ref mut obj) = results[best_idx_in_run].doc {
                    obj.insert(text_field.to_string(), Value::String(merged_text));
                    obj.insert("chunk_index".to_string(), json!(run[0].0));
                    if let Some(sc) = min_start_char {
                        obj.insert("start_char".to_string(), json!(sc));
                    }
                    if let Some(ec) = max_end_char {
                        obj.insert("end_char".to_string(), json!(ec));
                    }
                    obj.insert("chunk_merged".to_string(), json!(true));
                    obj.insert("chunks_in_merge".to_string(), json!(run.len()));
                }

                // Mark all other entries in this run for removal
                for &(_, res_idx) in run {
                    if res_idx != best_idx_in_run {
                        indices_to_remove.insert(res_idx);
                    }
                }
            }

            run_start = run_end + 1;
        }
    }

    let removed = indices_to_remove.len();
    if removed > 0 {
        let mut idx = 0;
        results.retain(|_| {
            let keep = !indices_to_remove.contains(&idx);
            idx += 1;
            keep
        });
    }

    removed
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
                    let sim =
                        ironbase_core::vector::simd::cosine_similarity(emb_i, emb_first) as f64;
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
                                    ironbase_core::vector::simd::cosine_similarity(emb_i, emb_new)
                                        as f64;
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

// ============================================================================
// Document-level AND qualification
// ============================================================================

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
pub(crate) fn qualify_documents(
    adapter: &Arc<IronBaseAdapter>,
    collection: &str,
    text_field: &str,
    query: &str,
) -> Result<Option<Value>> {
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
                limit: Some(MAX_INTERNAL_LIMIT),
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

// ============================================================================
// Document qualification orchestration (shared between hybrid & fulltext)
// ============================================================================

/// Validate `match_scope` parameter value.
/// Returns error for invalid values; None and valid values pass through.
pub(crate) fn validate_match_scope(match_scope: Option<&str>) -> Result<()> {
    if let Some(scope) = match_scope {
        match scope {
            "document" | "chunk" => Ok(()),
            other => Err(McpError::invalid_params(format!(
                "Invalid match_scope '{}'. Use: 'document' or 'chunk'",
                other
            ))),
        }
    } else {
        Ok(())
    }
}

/// Result of document qualification orchestration
#[derive(Debug)]
pub(crate) struct QualificationOutcome {
    /// Whether document-level scope is active
    pub is_doc_scope: bool,
    /// Effective AND mode (false if doc qualification switched to OR)
    pub effective_and_mode: bool,
    /// Effective filter (merged user filter + doc qualification filter)
    pub effective_filter: Option<Value>,
    /// Number of qualified documents (if doc scope active)
    pub qualified_doc_count: Option<usize>,
}

/// Apply document-level AND qualification if needed.
///
/// Shared orchestration logic used by both `hybrid_search` and `fulltext_search`.
/// Handles: match_scope validation, multi-field qualification (union), filter merge,
/// and OR mode switch.
///
/// `fields`: all text fields to qualify across (union of doc_ids).
/// For single-field, pass a slice with one element.
pub(crate) fn apply_document_qualification(
    adapter: &Arc<IronBaseAdapter>,
    collection: &str,
    fields: &[&str],
    query: &str,
    mode: Option<&str>,
    match_scope: Option<&str>,
    user_filter: Option<Value>,
) -> Result<QualificationOutcome> {
    validate_match_scope(match_scope)?;

    let and_mode = mode != Some("or");
    let is_doc_scope = and_mode && match_scope != Some("chunk");

    if !is_doc_scope || fields.is_empty() {
        return Ok(QualificationOutcome {
            is_doc_scope,
            effective_and_mode: and_mode,
            effective_filter: user_filter,
            qualified_doc_count: None,
        });
    }

    // Skip qualification if the collection has no fulltext indexes (non-RAG collection)
    let ft_fields = adapter
        .get_fulltext_field_names(collection)
        .unwrap_or_default();
    if ft_fields.is_empty() {
        return Ok(QualificationOutcome {
            is_doc_scope: false,
            effective_and_mode: and_mode,
            effective_filter: user_filter,
            qualified_doc_count: None,
        });
    }

    // Multi-field qualification: qualify on each field, union the doc_ids.
    // A document qualifies if ALL query tokens appear across its chunks in ANY of the fields.
    let doc_qualification_filter = if fields.len() == 1 {
        // Single field: direct qualification
        qualify_documents(adapter, collection, fields[0], query)?
    } else {
        // Multi-field: union of qualified doc_ids across all fields
        let mut union_doc_ids: HashSet<String> = HashSet::new();
        let mut any_field_had_restriction = false;

        for field in fields {
            match qualify_documents(adapter, collection, field, query)? {
                None => {
                    // NotRequired for this field (e.g. single token) — no restriction needed
                    return Ok(QualificationOutcome {
                        is_doc_scope,
                        effective_and_mode: and_mode,
                        effective_filter: user_filter,
                        qualified_doc_count: None,
                    });
                }
                Some(filter) => {
                    any_field_had_restriction = true;
                    if let Some(ids) = filter
                        .get("doc_id")
                        .and_then(|v| v.get("$in"))
                        .and_then(|v| v.as_array())
                    {
                        for id in ids {
                            if let Some(s) = id.as_str() {
                                union_doc_ids.insert(s.to_string());
                            }
                        }
                    }
                }
            }
        }

        if any_field_had_restriction {
            let qualified_vec: Vec<Value> = union_doc_ids.into_iter().map(|s| json!(s)).collect();
            Some(json!({"doc_id": {"$in": qualified_vec}}))
        } else {
            None
        }
    };

    let qualified_doc_count = doc_qualification_filter.as_ref().and_then(|f| {
        f.get("doc_id")
            .and_then(|v| v.get("$in"))
            .and_then(|v| v.as_array())
            .map(|a| a.len())
    });

    // If document-level AND is active, switch to OR mode (doc qualification
    // already ensures all tokens appear across the document's chunks) and merge filters.
    let (effective_and_mode, effective_filter) =
        if let Some(ref qual_filter) = doc_qualification_filter {
            let merged = match user_filter {
                Some(user_f) => Some(json!({"$and": [user_f, qual_filter]})),
                None => Some(qual_filter.clone()),
            };
            (false, merged) // OR mode — doc-level AND already satisfied
        } else {
            (and_mode, user_filter)
        };

    Ok(QualificationOutcome {
        is_doc_scope,
        effective_and_mode,
        effective_filter,
        qualified_doc_count,
    })
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

        rerank_results(&mut results, "the exact phrase test query", "content", None);

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
        let mut results = vec![FusedResult {
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
        }];

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
        let mut results = vec![FusedResult {
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
        }];

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

    // -------------------------------------------------------------------------
    // Adjacent Chunk Merge Tests
    // -------------------------------------------------------------------------

    fn make_chunk(
        id: &str,
        doc_id: &str,
        chunk_index: u64,
        text: &str,
        start_char: u64,
        end_char: u64,
        score: f64,
    ) -> FusedResult {
        FusedResult {
            id: id.to_string(),
            doc: json!({
                "_id": id,
                "doc_id": doc_id,
                "chunk_index": chunk_index,
                "content": text,
                "start_char": start_char,
                "end_char": end_char
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
    fn test_merge_adjacent_basic() {
        // Two adjacent chunks from same doc → merge into 1
        let mut results = vec![
            make_chunk("c6", "docA", 6, "Hello world overlap_", 600, 700, 0.03),
            make_chunk("c7", "docA", 7, "overlap_region end text", 680, 800, 0.02),
        ];

        let removed = merge_adjacent_chunks(&mut results, "content");

        assert_eq!(removed, 1);
        assert_eq!(results.len(), 1);
        // Best score kept
        assert!((results[0].final_score - 0.03).abs() < 1e-10);
        // Metadata
        assert_eq!(results[0].doc["chunk_merged"], true);
        assert_eq!(results[0].doc["chunks_in_merge"], 2);
        assert_eq!(results[0].doc["chunk_index"], 6);
        assert_eq!(results[0].doc["start_char"], 600);
        assert_eq!(results[0].doc["end_char"], 800);
    }

    #[test]
    fn test_merge_distant_chunks_preserved() {
        // Same doc_id but non-adjacent chunk_index → NOT merged
        let mut results = vec![
            make_chunk("c10", "docA", 10, "Chunk 10 text", 1000, 1100, 0.03),
            make_chunk("c15", "docA", 15, "Chunk 15 text", 1500, 1600, 0.02),
        ];

        let removed = merge_adjacent_chunks(&mut results, "content");

        assert_eq!(removed, 0);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_merge_three_consecutive() {
        // Three consecutive chunks → merge into 1
        // Realistic overlap: 4 chars overlap out of ~14 char chunks
        let mut results = vec![
            make_chunk("c5", "docA", 5, "AAAA BBBB CCCC", 0, 14, 0.01),
            make_chunk("c6", "docA", 6, "CCCC DDDD EEEE", 10, 24, 0.03),
            make_chunk("c7", "docA", 7, "EEEE FFFF GGGG", 20, 34, 0.02),
        ];

        let removed = merge_adjacent_chunks(&mut results, "content");

        assert_eq!(removed, 2);
        assert_eq!(results.len(), 1);
        // Best score from run (c6 = 0.03)
        assert!((results[0].final_score - 0.03).abs() < 1e-10);
        assert_eq!(results[0].doc["chunks_in_merge"], 3);
        assert_eq!(results[0].doc["start_char"], 0);
        assert_eq!(results[0].doc["end_char"], 34);
        // Text should be merged with overlap removed
        let merged_text = results[0].doc["content"].as_str().unwrap();
        assert_eq!(merged_text, "AAAA BBBB CCCC DDDD EEEE FFFF GGGG");
    }

    /// Regression for #64: merge must only touch the text field it is given;
    /// other fields (e.g. `title`, identical across chunks) must stay intact.
    /// The original bug merged a mis-resolved field (`title`), concatenating it
    /// N times while leaving content unmerged.
    #[test]
    fn test_merge_leaves_other_fields_intact() {
        let mut results = vec![
            FusedResult {
                id: "c0".into(),
                doc: json!({
                    "_id": "c0", "doc_id": "docA", "chunk_index": 0,
                    "content": "AAAA BBBB", "title": "BKV Zrt árajánlat",
                    "start_char": 0, "end_char": 9
                }),
                rrf_score: 0.02,
                final_score: 0.02,
                rerank_boost: 1.0,
                v_rank: 1,
                t_rank: 1,
                v_score: None,
                t_score: Some(1.0),
            },
            FusedResult {
                id: "c1".into(),
                doc: json!({
                    "_id": "c1", "doc_id": "docA", "chunk_index": 1,
                    "content": "BBBB CCCC", "title": "BKV Zrt árajánlat",
                    "start_char": 5, "end_char": 14
                }),
                rrf_score: 0.03,
                final_score: 0.03,
                rerank_boost: 1.0,
                v_rank: 1,
                t_rank: 1,
                v_score: None,
                t_score: Some(1.0),
            },
        ];

        let removed = merge_adjacent_chunks(&mut results, "content");
        assert_eq!(removed, 1);
        assert_eq!(results.len(), 1);
        // Content is merged across both chunks (overlap removed)...
        assert_eq!(
            results[0].doc["content"].as_str().unwrap(),
            "AAAA BBBB CCCC"
        );
        // ...but the title is the single stored value, NOT concatenated (#64).
        assert_eq!(
            results[0].doc["title"].as_str().unwrap(),
            "BKV Zrt árajánlat"
        );
    }

    #[test]
    fn test_merge_no_doc_id() {
        // Results without doc_id → untouched
        let mut results = vec![
            make_fused_result("doc1", "Content A", "Title", 0.03),
            make_fused_result("doc2", "Content B", "Title", 0.02),
        ];

        let removed = merge_adjacent_chunks(&mut results, "content");

        assert_eq!(removed, 0);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_merge_mixed_docs() {
        // doc_A chunk 6+7 (adjacent) + doc_B chunk 3 (alone) → doc_A merges, doc_B stays
        let mut results = vec![
            make_chunk("a6", "docA", 6, "A chunk six_", 600, 700, 0.03),
            make_chunk("a7", "docA", 7, "six_A chunk seven", 680, 800, 0.02),
            make_chunk("b3", "docB", 3, "B chunk three", 300, 400, 0.025),
        ];

        let removed = merge_adjacent_chunks(&mut results, "content");

        assert_eq!(removed, 1);
        assert_eq!(results.len(), 2);
        // docA merged, docB untouched
        let has_doc_a = results
            .iter()
            .any(|r| r.doc.get("doc_id").and_then(|v| v.as_str()) == Some("docA"));
        let has_doc_b = results
            .iter()
            .any(|r| r.doc.get("doc_id").and_then(|v| v.as_str()) == Some("docB"));
        assert!(has_doc_a);
        assert!(has_doc_b);
    }

    #[test]
    fn test_merge_empty() {
        let mut results: Vec<FusedResult> = vec![];
        let removed = merge_adjacent_chunks(&mut results, "content");
        assert_eq!(removed, 0);
        assert_eq!(results.len(), 0);
    }
}
