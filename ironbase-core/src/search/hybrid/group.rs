//! Grouping fused chunks by document (`group_by_document`).

use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

use crate::collection_core::CollectionCore;
use crate::error::{IronBaseError, Result};
use crate::execution::ExecutionContext;
use crate::find_options::apply_projection;
use crate::fulltext::FulltextSearchOptions;
use crate::storage::{RawStorage, Storage};

use super::fuse::{exclusion_projection, fused_doc_id};
use super::{id_to_string, merge_adjacent_values, FusedResult, HybridQuery, MAX_CANDIDATES};

/// One grouped document: its id, best fused score, doc-level fields lifted from
/// its chunks ([`lift_common_fields`]), and its (post-cap, merged) chunks.
#[derive(Debug, Clone)]
pub struct DocGroup {
    pub doc_id: String,
    pub best_score: f64,
    pub lifted: serde_json::Map<String, Value>,
    /// Merged chunks (full Values, carrying score fields)
    pub chunks: Vec<Value>,
}

/// Keys never lifted to the group root by [`lift_common_fields`] in
/// `group_by_document` mode (#69). Two categories:
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
    // Merge metadata (merge.rs)
    "chunk_merged",
    "chunks_in_merge",
    // Engine score metadata (score fields + the Phase 2 fulltext score)
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
pub fn lift_common_fields(chunks: &mut [Value]) -> serde_json::Map<String, Value> {
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
                obj.shift_remove(key);
            }
        }
    }
    lifted
}

impl<S: Storage + RawStorage> CollectionCore<S> {
    /// Two-phase document grouping over [`CollectionCore::hybrid_fuse`]'s
    /// result (run with `group_by_document`).
    ///
    /// Phase 1: top `q.limit` unique documents from `fused` (first occurrence =
    /// best score), keeping every fused chunk of those documents. Phase 2: one
    /// fulltext OR search restricted to those documents → further relevant
    /// chunks. A document's chunks are its Phase-1 chunks in fused order (full
    /// score shape) followed by its Phase-2-only chunks in BM25 order (just
    /// `_text_score`, #71). Phase 2 is lexical, so the Phase-1 chunks are kept
    /// as they are: a chunk or a whole document found only by the vector lane
    /// has no BM25 hit and was dropped (hybrid review H1). Adjacent chunks are
    /// merged and [`lift_common_fields`] runs before the `max_chunks_per_doc`
    /// cap, so doc-level fields are derived from the full set.
    ///
    /// Returns the groups in best-score order and the total (post-cap) chunk count.
    pub fn hybrid_group_documents(
        &self,
        fused: &[FusedResult],
        q: &HybridQuery,
        ctx: Option<&ExecutionContext>,
    ) -> Result<(Vec<DocGroup>, usize)> {
        let projection = &q.projection;

        // Phase 1: top N unique doc_ids (first occurrence = best score)
        let mut doc_best_scores: HashMap<String, f64> = HashMap::new();
        let mut doc_order: Vec<String> = Vec::new();
        for item in fused {
            let doc_id = fused_doc_id(item);
            if !doc_best_scores.contains_key(&doc_id) {
                doc_best_scores.insert(doc_id.clone(), item.final_score);
                doc_order.push(doc_id);
                if doc_order.len() >= q.limit {
                    break;
                }
            }
        }

        // Every fused chunk of those docs, in fused order, with its score shape
        let mut doc_groups: HashMap<String, Vec<Value>> = HashMap::with_capacity(doc_order.len());
        let mut phase1_chunk_ids: HashSet<String> = HashSet::new();
        for item in fused {
            let doc_id = fused_doc_id(item);
            if !doc_best_scores.contains_key(&doc_id) {
                continue;
            }
            let chunk_id = item.doc.get("_id").and_then(id_to_string);
            if let Some(ref id) = chunk_id {
                phase1_chunk_ids.insert(id.clone());
            }
            let mut chunk = if let Some(ref proj) = projection {
                apply_projection(&item.doc, proj)?
            } else {
                item.doc.clone()
            };
            if let Value::Object(ref mut obj) = chunk {
                item.write_score_fields(obj);
            }
            doc_groups.entry(doc_id).or_default().push(chunk);
        }

        // Phase 2: single fulltext OR search for the top doc_ids' chunks
        let target_doc_id_set: HashSet<String> = doc_order.iter().cloned().collect();

        let phase2_limit = q.limit.saturating_mul(100).min(MAX_CANDIDATES);
        // The fields `projection` drops are left unparsed: a Phase-2 chunk is
        // projected right away, after only `doc_id` and `_id` are read (T2)
        let phase2_exclude: Vec<String> = projection
            .as_ref()
            .filter(|proj| proj.values().all(|&v| v == 0))
            .map(|proj| {
                proj.keys()
                    .filter(|k| proj[k.as_str()] == 0 && *k != "doc_id" && *k != "_id")
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        let phase2_options = FulltextSearchOptions {
            limit: Some(phase2_limit),
            projection: exclusion_projection(&phase2_exclude),
            filter: q.filter.clone(),
            and_mode: false, // OR mode: any query word → relevant chunk
            cancel_flag: ctx.and_then(|c| c.cancel_flag().cloned()),
            deadline: ctx.and_then(|c| c.deadline()),
            target_doc_ids: Some(target_doc_id_set),
            ..Default::default()
        };

        let phase2_results = match q.text_fields {
            Some(ref fields) => {
                let field_refs: Vec<&str> = fields.iter().map(|s| s.as_str()).collect();
                self.fulltext_search_multi_ext(&field_refs, &q.query, phase2_options)?
            }
            None => self.fulltext_search_ext(&q.text_field, &q.query, phase2_options)?,
        };

        // Append the Phase-2-only chunks after each doc's Phase-1 chunks
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
            if chunk_id
                .as_ref()
                .is_some_and(|id| phase1_chunk_ids.contains(id))
            {
                continue; // already in the group from Phase 1
            }
            let mut chunk = if let Some(ref proj) = projection {
                apply_projection(&res.document, proj)?
            } else {
                res.document
            };
            if let Value::Object(ref mut obj) = chunk {
                obj.insert("_text_score".to_string(), json!(res.score));
            }

            doc_groups.entry(doc_id).or_default().push(chunk);
        }

        // Assemble groups in doc_order (best score first)
        let mut total_chunks: usize = 0;
        let mut groups: Vec<DocGroup> = Vec::new();
        groups.try_reserve(doc_order.len()).map_err(|e| {
            IronBaseError::OutOfMemory(format!(
                "cannot allocate {} grouped results: {}",
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

            // Merge overlapping neighbours (same as flat mode) before lift and cap,
            // so each passage carries its text once
            if q.merge_chunks {
                merge_adjacent_values(&mut chunks, &q.text_field);
            }

            // Lift BEFORE cap so doc-level fields are determined from the full set.
            let lifted = lift_common_fields(&mut chunks);

            if let Some(cap) = q.max_chunks_per_doc {
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
}
