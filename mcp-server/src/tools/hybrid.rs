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

/// Keys reserved against accidental lift to the group root in `group_by_document`
/// mode (#69). Two categories:
///
/// 1. **Engine group-root keys** (`doc_id`, `best_score`, `chunk_count`, `chunks`) —
///    the grouped builder writes them itself; a coincidentally-identical user
///    field must not overwrite them.
/// 2. **Chunk-level engine/structural fields** — semantically belong to the
///    chunk even when their values happen to match across all chunks of a group
///    (e.g. all chunks tied on `_text_score`, or every chunk has the same
///    `chunk_merged=true`). Lifting these would falsely promote chunk semantics
///    to document semantics (e.g. `group.chunk_merged=true` reads as "the doc
///    was merged" when it's a chunk-level fact) or contradict `best_score`.
///
/// User-defined doc-level metadata (`title`, `customer`, `year`, `date`, etc.)
/// is naturally NOT in this list and lifts as designed.
const GROUP_RESERVED_KEYS: &[&str] = &[
    // Engine group-root keys
    "doc_id",
    "best_score",
    "chunk_count",
    "chunks",
    // Chunk-tracking (RAG schema)
    "chunk_index",
    "chunk_total",
    "start_char",
    "end_char",
    "table_header",
    // Chunk payload (default RAG convention: `text_field=content`,
    // `embedding_field=embedding`). Defense against duplicate ingest accidentally
    // lifting a per-chunk array/string to the group root (embedding bloats the
    // response; content is semantically wrong as a doc-level field). Custom field
    // names (e.g. `text_field="body"`) are not covered here — those rely on
    // natural value-variance to stay in chunks.
    "content",
    "embedding",
    // Merge metadata (fusion.rs)
    "chunk_merged",
    "chunks_in_merge",
    // Engine score metadata (enrich_result + fulltext_search Phase 2)
    "_text_score",
    "_vector_score",
    "_rrf_score",
    "_final_score",
    "_rerank_boost",
    "_vector_rank",
    "_text_rank",
];

/// Lift document-level fields to the group root in `group_by_document` mode (#69).
///
/// A key whose value is identical across ALL chunks in the group is **moved**
/// (removed from each chunk) and returned in the lifted map. Chunk-specific keys
/// (whose values vary) stay in `chunks[i]`. Generic, no hardcoded field list:
/// `title`/`customer`/`year`/`date`/`doc_type` naturally lift; `content`/`_id`/
/// `chunk_index`/`embedding`/`_text_score`/... naturally stay.
///
/// Skipped:
/// - Single-chunk groups (`chunks.len() < 2`) — lifting would empty `chunks[0]`
///   (vacuous "all match"); without duplication to remove there is no benefit
///   and clients iterating `chunks` would lose their data.
/// - Engine-reserved group-root keys (`GROUP_RESERVED_KEYS`) — never overwritten
///   by a coincidentally-identical user field.
pub(crate) fn lift_common_fields(chunks: &mut [Value]) -> serde_json::Map<String, Value> {
    let mut lifted: serde_json::Map<String, Value> = serde_json::Map::new();
    if chunks.len() < 2 {
        return lifted;
    }
    if let Some(Value::Object(first)) = chunks.first() {
        for (key, val) in first {
            if GROUP_RESERVED_KEYS.contains(&key.as_str()) {
                continue;
            }
            let all_match = chunks.iter().all(|c| {
                c.as_object()
                    .and_then(|o| o.get(key))
                    .map(|v| v == val)
                    .unwrap_or(false)
            });
            if all_match {
                lifted.insert(key.clone(), val.clone());
            }
        }
    }
    for chunk in chunks.iter_mut() {
        if let Value::Object(obj) = chunk {
            for key in lifted.keys() {
                obj.remove(key);
            }
        }
    }
    lifted
}

/// Score-only snapshot of a `FusedResult`. Used to enrich grouped-mode Phase 2
/// chunks with the same score shape as flat mode without cloning `doc: Value`.
#[derive(Debug, Clone, Copy)]
struct FusedScoreInfo {
    rrf_score: f64,
    final_score: f64,
    rerank_boost: f64,
    v_rank: usize,
    t_rank: usize,
    v_score: Option<f32>,
    t_score: Option<f64>,
}

impl From<&FusedResult> for FusedScoreInfo {
    fn from(item: &FusedResult) -> Self {
        Self {
            rrf_score: item.rrf_score,
            final_score: item.final_score,
            rerank_boost: item.rerank_boost,
            v_rank: item.v_rank,
            t_rank: item.t_rank,
            v_score: item.v_score,
            t_score: item.t_score,
        }
    }
}

/// Single source of truth for the chunk-level score shape, shared by flat
/// `enrich_result` and grouped-mode Phase 2 chunk enrichment.
fn apply_score_fields(obj: &mut serde_json::Map<String, Value>, info: &FusedScoreInfo) {
    obj.insert("_rrf_score".to_string(), json!(info.rrf_score));
    obj.insert("_final_score".to_string(), json!(info.final_score));
    obj.insert("_rerank_boost".to_string(), json!(info.rerank_boost));
    obj.insert("_vector_rank".to_string(), json!(info.v_rank));
    obj.insert("_text_rank".to_string(), json!(info.t_rank));
    if let Some(vs) = info.v_score {
        obj.insert("_vector_score".to_string(), json!(vs));
    }
    if let Some(ts) = info.t_score {
        obj.insert("_text_score".to_string(), json!(ts));
    }
}

/// Output of the shared retrieve+fuse pipeline (STEP 1–6): the fused, reranked,
/// merged result set plus the resolution/qualification context the response
/// builders need. Shared by `hybrid_search` (flat + grouped) and the `search`
/// tool, so both run identical retrieval with no duplication or JSON round-trip.
pub(crate) struct FusionOutcome {
    pub fused: Vec<FusedResult>,
    pub effective_text_field: String,
    pub effective_text_fields: Option<Vec<String>>,
    pub qual: super::fusion::QualificationOutcome,
}

/// Shared retrieve+fuse pipeline (STEP 1–6): resolve fields/provider, document
/// qualification, vector + fulltext retrieval, RRF fusion, rerank, per-doc cap
/// (flat), adjacent-chunk merge, and MMR (flat). Behaviour-preserving extraction
/// of the former inline prefix of `handle_hybrid_search`.
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

    // Sort by RRF score initially. Tie-break on the stable chunk id so equal
    // scores resolve deterministically — otherwise the HashMap-drain input order
    // leaks through the (stable) sort and the same query can return different
    // tied docs run-to-run (reproducibility + eval-gate correctness).
    fused.sort_by(|a, b| {
        b.rrf_score
            .partial_cmp(&a.rrf_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.id.cmp(&b.id))
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
    // STEP 5.4: Per-doc chunk cap in flat mode (#72)
    //
    // Applied AFTER rerank, BEFORE merge_chunks — so the cap is enforced on
    // the raw chunk-top-K and the subsequent adjacent-merge counts a merge-run
    // as a single result (issue #72 acceptance criterion 3). Grouped mode caps
    // the chunks[] array later, in the grouped builder (already implemented
    // for #71).
    // ========================================================================
    if let Some(cap) = p.max_chunks_per_doc {
        if !p.group_by_document {
            let mut counts: HashMap<String, usize> = HashMap::new();
            fused.retain(|item| {
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
                let c = counts.entry(doc_id).or_insert(0);
                *c += 1;
                *c <= cap
            });
        }
    }

    // ========================================================================
    // STEP 5.5: Merge adjacent chunks from same document (overlap dedup)
    // ========================================================================
    // Flat mode only: in group_by_document mode `build_doc_groups` re-fetches
    // every chunk via its own Phase-2 search and never reads these merged
    // bodies, so merging here would be wasted O(n) work. (Doc ordering is
    // unaffected: merge preserves each doc's max score.) Mutates `fused` in
    // place; the merge count is no longer surfaced.
    if p.merge_chunks && !p.group_by_document {
        merge_adjacent_chunks(&mut fused, &effective_text_field);
    }

    // ========================================================================
    // STEP 6: MMR diversity reranking (skipped when grouping — limit applies
    //         at document level in STEP 7)
    // ========================================================================
    if p.deduplicate && !p.group_by_document {
        mmr_reorder(&mut fused, &effective_vector_field, p.mmr_lambda, p.limit);
    } else if !p.group_by_document {
        // Flat mode: truncate to limit here
        fused.truncate(p.limit);
    }
    // group_by_document: no truncation, limit applied at doc level in STEP 7

    Ok(FusionOutcome {
        fused,
        effective_text_field,
        effective_text_fields,
        qual,
    })
}

/// One grouped document: its id, best fused score, doc-level fields lifted from
/// its chunks (generic, via `lift_common_fields`), and its (post-cap, merged)
/// chunks. Internal structured form consumed by the `hybrid_search` grouped JSON
/// builder and (Stage A) by the `search` compact contract — so the two-phase
/// grouping logic lives in exactly one place.
pub(crate) struct DocGroup {
    pub doc_id: String,
    pub best_score: f64,
    pub lifted: serde_json::Map<String, Value>,
    /// Merged chunks (full Values, carrying score fields). The old grouped
    /// response emits these as `chunks`; the new contract maps them to passages.
    pub chunks: Vec<Value>,
}

/// Two-phase document grouping over an already-fused, sorted result set.
///
/// Phase 1: top-N unique doc_ids from `fused` (first occurrence = best score),
/// recording per-chunk Phase-1 score info. Phase 2: one fulltext OR search
/// filtered to those doc_ids → all relevant chunks; Phase-1 chunks are re-enriched
/// with the full score shape, Phase-2-only chunks carry just `_text_score` (#71).
/// Adjacent-merge `lift_common_fields` runs before `max_chunks_per_doc` truncation
/// so doc-level fields are derived from the full set.
///
/// Returns the groups in best-score order and the total (post-cap) chunk count.
/// Behaviour-preserving extraction of the former inline grouped builder.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_doc_groups(
    fused: &[FusedResult],
    adapter: &Arc<IronBaseAdapter>,
    collection: &str,
    query: &str,
    limit: usize,
    filter: &Option<Value>,
    max_chunks_per_doc: Option<usize>,
    projection: &Option<HashMap<String, i32>>,
    effective_text_field: &str,
    effective_text_fields: &Option<Vec<String>>,
) -> Result<(Vec<DocGroup>, usize)> {
    // Phase 1: top N unique doc_ids + per-chunk score info
    let mut doc_best_scores: HashMap<String, f64> = HashMap::new();
    let mut doc_order: Vec<String> = Vec::new();
    let mut phase1_chunk_info: HashMap<String, FusedScoreInfo> = HashMap::new();

    for item in fused {
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

        if let Some(chunk_id) = item.doc.get("_id").and_then(id_to_string) {
            phase1_chunk_info.insert(chunk_id, FusedScoreInfo::from(item));
        }

        if !doc_best_scores.contains_key(&doc_id) {
            doc_best_scores.insert(doc_id.clone(), item.final_score);
            doc_order.push(doc_id);
            if doc_order.len() >= limit {
                break;
            }
        }
    }

    // Phase 2: single fulltext OR search for the top doc_ids' chunks
    let target_doc_id_set: HashSet<String> = doc_order.iter().cloned().collect();

    let phase2_limit = (limit * 100).min(MAX_INTERNAL_LIMIT);
    let phase2_options = FulltextSearchOptions {
        limit: Some(phase2_limit),
        skip: None,
        min_score: None,
        projection: None,
        filter: filter.clone(),
        and_mode: false, // OR mode: any query word → relevant chunk
        highlight: false,
        highlight_context: None,
        highlight_max_snippets: None,
        target_doc_ids: Some(target_doc_id_set),
    };

    let phase2_results = if let Some(ref fields) = effective_text_fields {
        let field_refs: Vec<&str> = fields.iter().map(|s| s.as_str()).collect();
        adapter.fulltext_search_multi(collection, &field_refs, query, phase2_options)?
    } else {
        adapter.fulltext_search(collection, effective_text_field, query, phase2_options)?
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

        let chunk_id = res.document.get("_id").and_then(id_to_string);
        let mut chunk = if let Some(ref proj) = projection {
            apply_projection(&res.document, proj)
        } else {
            res.document
        };
        if let Value::Object(ref mut obj) = chunk {
            if let Some(ref id) = chunk_id {
                if let Some(info) = phase1_chunk_info.get(id) {
                    apply_score_fields(obj, info);
                } else {
                    obj.insert("_text_score".to_string(), json!(res.score));
                }
            } else {
                obj.insert("_text_score".to_string(), json!(res.score));
            }
        }

        doc_groups.entry(doc_id).or_default().push(chunk);
    }

    // Assemble groups in doc_order (best score first)
    let mut total_chunks: usize = 0;
    let mut groups: Vec<DocGroup> = Vec::new();
    groups.try_reserve(doc_order.len()).map_err(|e| {
        McpError::internal(format!(
            "OOM: cannot allocate {} grouped results: {}",
            doc_order.len(),
            e
        ))
    })?;
    for doc_id in doc_order {
        let best_score = doc_best_scores.get(&doc_id).copied().unwrap_or(0.0);
        let mut chunks = match doc_groups.remove(&doc_id) {
            Some(c) if !c.is_empty() => c,
            _ => continue,
        };

        // Lift BEFORE cap so doc-level fields are determined from the full set.
        let lifted = lift_common_fields(&mut chunks);

        if let Some(cap) = max_chunks_per_doc {
            chunks.truncate(cap);
        }
        total_chunks += chunks.len();

        groups.push(DocGroup {
            doc_id,
            best_score,
            lifted,
            chunks,
        });
    }

    Ok((groups, total_chunks))
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

    // -------------------------------------------------------------------------
    // lift_common_fields (#69 — group_by_document doc-level field promotion)
    // -------------------------------------------------------------------------

    #[test]
    fn test_lift_common_fields_promotes_identical_and_drops_from_chunks() {
        let mut chunks = vec![
            json!({"doc_id":"a","title":"T","year":2026,"content":"c1","chunk_index":0,"_text_score":6.2}),
            json!({"doc_id":"a","title":"T","year":2026,"content":"c2","chunk_index":1,"_text_score":5.1}),
            json!({"doc_id":"a","title":"T","year":2026,"content":"c3","chunk_index":2,"_text_score":4.0}),
        ];
        let lifted = lift_common_fields(&mut chunks);

        // doc_id is explicitly skipped (group root carries it).
        assert!(!lifted.contains_key("doc_id"));
        // Identical-everywhere keys lifted.
        assert_eq!(lifted["title"], json!("T"));
        assert_eq!(lifted["year"], json!(2026));
        // Varying keys stay in each chunk.
        assert!(!lifted.contains_key("content"));
        assert!(!lifted.contains_key("chunk_index"));
        assert!(!lifted.contains_key("_text_score"));
        // Lifted keys removed from each chunk → no duplication.
        for c in &chunks {
            assert!(c.get("title").is_none(), "title still in chunk: {c}");
            assert!(c.get("year").is_none(), "year still in chunk: {c}");
            assert!(
                c.get("content").is_some(),
                "content missing from chunk: {c}"
            );
        }
    }

    #[test]
    fn test_lift_common_fields_skips_single_chunk_group() {
        // With one chunk, "all match" is vacuously true on every key — lifting
        // would strand chunks[0] empty. Gated: no lift, chunk stays self-contained.
        let mut chunks = vec![json!({"doc_id":"x","title":"T","content":"only"})];
        let lifted = lift_common_fields(&mut chunks);
        assert!(lifted.is_empty(), "single-chunk groups must not lift");
        // chunk[0] retains all its fields (title, content) — clients iterating
        // group.chunks for snippet text get something to render.
        assert_eq!(chunks[0]["title"], json!("T"));
        assert_eq!(chunks[0]["content"], json!("only"));
    }

    #[test]
    fn test_lift_common_fields_skips_chunk_level_engine_metadata() {
        // Chunk-level engine metadata (`_text_score`, `chunk_merged`, etc.) MUST NOT
        // lift even if it happens to be identical across all chunks (BM25 tie, all
        // merged, etc.). Lifting would falsely promote chunk semantics to the doc.
        let mut chunks = vec![
            json!({
                "doc_id":"a", "title":"T",
                "chunk_index":3, "chunk_merged":true, "chunks_in_merge":2,
                "_text_score":5.0, "_final_score":0.03, "_text_rank":1,
                "table_header":"| H |\n|---|",
                "start_char":0, "end_char":100,
                "content":"first"
            }),
            json!({
                "doc_id":"a", "title":"T",
                "chunk_index":3, "chunk_merged":true, "chunks_in_merge":2,
                "_text_score":5.0, "_final_score":0.03, "_text_rank":1,
                "table_header":"| H |\n|---|",
                "start_char":0, "end_char":100,
                "content":"second"
            }),
        ];
        let lifted = lift_common_fields(&mut chunks);
        // Only the genuinely doc-level field lifts.
        assert_eq!(lifted["title"], json!("T"));
        // Chunk-level engine/structural keys stay in chunks, no matter how identical.
        for k in [
            "chunk_index",
            "chunk_merged",
            "chunks_in_merge",
            "_text_score",
            "_final_score",
            "_text_rank",
            "table_header",
            "start_char",
            "end_char",
        ] {
            assert!(
                !lifted.contains_key(k),
                "chunk-level key '{k}' must not lift"
            );
            for c in &chunks {
                assert!(
                    c.get(k).is_some(),
                    "chunk-level '{k}' missing from chunk: {c}"
                );
            }
        }
    }

    #[test]
    fn test_lift_common_fields_skips_chunk_payload_defaults() {
        // Defensive: duplicate ingest can leave every chunk with identical
        // `embedding` (vector) and `content` (text) — these must NOT lift, even
        // when value-equal across the whole group. Lifting `embedding` would
        // bloat the response; lifting `content` is semantically wrong.
        let mut chunks = vec![
            json!({
                "doc_id":"a", "title":"T",
                "content":"identical text",
                "embedding":[0.1,0.2,0.3]
            }),
            json!({
                "doc_id":"a", "title":"T",
                "content":"identical text",
                "embedding":[0.1,0.2,0.3]
            }),
        ];
        let lifted = lift_common_fields(&mut chunks);
        assert_eq!(lifted["title"], json!("T"));
        for k in ["content", "embedding"] {
            assert!(
                !lifted.contains_key(k),
                "chunk-payload key '{k}' must not lift"
            );
            for c in &chunks {
                assert!(c.get(k).is_some(), "'{k}' should remain in chunk: {c}");
            }
        }
    }

    #[test]
    fn test_lift_common_fields_skips_engine_reserved_keys() {
        // A user field happening to be named `best_score`/`chunk_count`/`chunks`
        // with identical values across chunks must NOT be lifted — those names
        // are reserved for the engine at the group root and would overwrite it.
        let mut chunks = vec![
            json!({"doc_id":"a","title":"T","best_score":0.99,"chunk_count":7,"chunks":["nested"],"content":"c1"}),
            json!({"doc_id":"a","title":"T","best_score":0.99,"chunk_count":7,"chunks":["nested"],"content":"c2"}),
        ];
        let lifted = lift_common_fields(&mut chunks);
        assert_eq!(lifted["title"], json!("T"));
        for k in ["doc_id", "best_score", "chunk_count", "chunks"] {
            assert!(
                !lifted.contains_key(k),
                "engine-reserved key '{k}' must not lift"
            );
            // ...and stays untouched in each chunk.
            for c in &chunks {
                assert!(
                    c.get(k).is_some(),
                    "engine-reserved '{k}' missing from chunk: {c}"
                );
            }
        }
    }

    #[test]
    fn test_lift_common_fields_empty_input() {
        let mut chunks: Vec<Value> = Vec::new();
        let lifted = lift_common_fields(&mut chunks);
        assert!(lifted.is_empty());
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
        build_doc_groups(
            &fo.fused,
            a,
            &p.collection,
            &p.query,
            p.limit,
            &p.filter,
            p.max_chunks_per_doc,
            &proj,
            &fo.effective_text_field,
            &fo.effective_text_fields,
        )
        .expect("build_doc_groups")
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
            "vector":[0.9,0.1,0.0,0.0],"group_by_document":true,"limit":5,"max_chunks_per_doc":2}),
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
    /// `qualify_documents` keys the filter on `_id`; the multi-field union must
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
