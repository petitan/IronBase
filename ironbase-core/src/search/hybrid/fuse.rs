//! Retrieval and Reciprocal Rank Fusion: the vector and fulltext lanes,
//! RRF, rerank, the per-document cap, adjacent-chunk merge and MMR.

use serde_json::Value;
use std::collections::HashMap;

use crate::collection_core::{CollectionCore, QualificationOutcome};
use crate::error::Result;
use crate::execution::ExecutionContext;
use crate::find_options::validate_projection;
use crate::fulltext::FulltextSearchOptions;
use crate::storage::{RawStorage, Storage};
use crate::vector::VectorSearchOptions;

use super::{
    id_to_string, merge_adjacent_chunks, mmr_reorder, rerank_results, FusedResult, HybridQuery,
    MAX_CANDIDATES,
};

/// Output of [`CollectionCore::hybrid_fuse`]
#[derive(Debug)]
pub struct HybridFusion {
    /// Fused, reranked results, best first. Flat mode: merged, capped per
    /// document, MMR-reordered and cut to `limit`. Grouped mode: every
    /// candidate, for [`CollectionCore::hybrid_group_documents`]. The
    /// documents are not projected (only a projected-out embedding may be
    /// missing): flat callers apply `q.projection` to their output.
    pub fused: Vec<FusedResult>,
    /// How document-level AND restricted the fulltext lane
    pub qualification: QualificationOutcome,
}

impl<S: Storage + RawStorage> CollectionCore<S> {
    /// Hybrid retrieval: vector and fulltext lanes fused with weighted
    /// Reciprocal Rank Fusion, then reranked, capped per document, merged and
    /// MMR-reordered as `q` asks.
    ///
    /// The vector lane runs only when `q.vector` is set. Each lane returns
    /// `limit × 3` candidates (`× 20` when grouping), at most
    /// [`MAX_CANDIDATES`]. A document a lane did not return gets nothing from
    /// it; ties are broken on the chunk id, so the order is deterministic.
    pub fn hybrid_fuse(
        &self,
        q: &HybridQuery,
        ctx: Option<&ExecutionContext>,
    ) -> Result<HybridFusion> {
        if let Some(ref projection) = q.projection {
            validate_projection(projection)?;
        }

        // Internal limit: higher when grouping to capture enough chunks per document
        let internal_multiplier = if q.group_by_document { 20 } else { 3 };
        let internal_limit = q
            .limit
            .saturating_mul(internal_multiplier)
            .min(MAX_CANDIDATES);

        // Document-level AND qualification (match_scope="document")
        let qual_fields: Vec<&str> = match q.text_fields {
            Some(ref fields) => fields.iter().map(|s| s.as_str()).collect(),
            None => vec![q.text_field.as_str()],
        };
        let qualification = self.fulltext_document_scope(
            &qual_fields,
            &q.query,
            q.and_mode,
            q.document_scope,
            q.filter.clone(),
        )?;

        // The candidates' vectors are left unparsed when the output drops them
        // anyway and MMR does not read them: an embedding is most of a chunk's
        // bytes, and parsing it cost ~50 µs per candidate (hybrid review T2)
        let mmr = q.deduplicate && !q.group_by_document;
        let lane_exclude: Vec<String> = match q.projection {
            Some(ref proj)
                if !mmr
                    && !q.vector_field.contains('.')
                    && projection_drops(proj, &q.vector_field) =>
            {
                vec![q.vector_field.clone()]
            }
            _ => Vec::new(),
        };

        // Vector lane → ranks + docs (single pass)
        let mut vector_ranks: HashMap<String, usize> = HashMap::new();
        let mut vector_docs: HashMap<String, (Value, f32)> = HashMap::new();
        if let Some(ref query_vector) = q.vector {
            let searched = q.searched_vector_field.as_ref().unwrap_or(&q.vector_field);
            let mut exclude_fields = lane_exclude.clone();
            if *searched != q.vector_field && exclude_fields.contains(&q.vector_field) {
                exclude_fields.push(searched.clone());
            }
            let options = VectorSearchOptions {
                filter: q.filter.clone(),
                exclude_fields,
            };
            let vector_results = self.vector_search_with_options(
                searched,
                query_vector,
                internal_limit,
                &options,
                ctx,
            )?;

            vector_ranks.reserve(vector_results.len());
            vector_docs.reserve(vector_results.len());
            for (rank, (doc, score)) in vector_results.into_iter().enumerate() {
                if let Some(id) = doc.get("_id").and_then(id_to_string) {
                    vector_ranks.insert(id.clone(), rank + 1);
                    vector_docs.insert(id, (doc, score));
                }
            }
        }

        // Fulltext lane → ranks + docs (single pass), restricted by the qualification
        let text_options = FulltextSearchOptions {
            limit: Some(internal_limit),
            // Full docs for merging, but for the dropped vector field (above)
            projection: exclusion_projection(&lane_exclude),
            filter: qualification.effective_filter.clone(),
            and_mode: qualification.effective_and_mode,
            cancel_flag: ctx.and_then(|c| c.cancel_flag().cloned()),
            deadline: ctx.and_then(|c| c.deadline()),
            ..Default::default()
        };
        let text_results = match q.text_fields {
            Some(ref fields) => {
                let field_refs: Vec<&str> = fields.iter().map(|s| s.as_str()).collect();
                self.fulltext_search_multi_ext(&field_refs, &q.query, text_options)?
            }
            None => self.fulltext_search_ext(&q.text_field, &q.query, text_options)?,
        };

        let mut text_ranks: HashMap<String, usize> = HashMap::with_capacity(text_results.len());
        let mut text_docs: HashMap<String, (Value, f64)> =
            HashMap::with_capacity(text_results.len());
        for (rank, res) in text_results.into_iter().enumerate() {
            if let Some(id) = res.document.get("_id").and_then(id_to_string) {
                text_ranks.insert(id.clone(), rank + 1);
                text_docs.insert(id, (res.document, res.score));
            }
        }

        // RRF fusion (zero-copy drain, no intermediate HashSet).
        //
        // Standard RRF: a lane that did not return the document adds nothing
        // (rank 0 = absent). It used to count as rank `internal_limit + 1`, a
        // penalty that shrinks as `limit` grows, so the same documents were
        // ordered differently depending on the requested limit (hybrid review M3).
        let lane = |weight: f64, rank: usize| {
            if rank == 0 {
                0.0
            } else {
                weight / (q.rrf_k + rank as f64)
            }
        };
        let mut fused: Vec<FusedResult> = Vec::with_capacity(vector_docs.len() + text_docs.len());

        // Vector docs first — consume matching text docs via remove (no clone)
        for (id, (doc, v_score)) in vector_docs.drain() {
            let v_rank = vector_ranks.get(&id).copied().unwrap_or(0);
            let t_rank = text_ranks.get(&id).copied().unwrap_or(0);
            let t_score = text_docs.remove(&id).map(|(_, s)| s);

            let rrf_score = lane(q.vector_weight, v_rank) + lane(q.fulltext_weight, t_rank);

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
            let t_rank = text_ranks.get(&id).copied().unwrap_or(0);

            let rrf_score = lane(q.fulltext_weight, t_rank);

            fused.push(FusedResult {
                id,
                doc,
                rrf_score,
                final_score: rrf_score,
                rerank_boost: 1.0,
                v_rank: 0,
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

        if q.rerank {
            rerank_results(
                &mut fused,
                &q.query,
                &q.text_field,
                q.title_field.as_deref(),
            );
        }

        // Per-doc chunk cap in flat mode (#72): AFTER rerank, BEFORE the merge,
        // so the cap is enforced on the raw chunk top-K and a merged run counts
        // as one result. Grouped mode caps each document's chunks when grouping.
        if let Some(cap) = q.max_chunks_per_doc {
            if !q.group_by_document {
                let mut counts: HashMap<String, usize> = HashMap::new();
                fused.retain(|item| {
                    let c = counts.entry(fused_doc_id(item)).or_insert(0);
                    *c += 1;
                    *c <= cap
                });
            }
        }

        // Flat mode only: in grouped mode `hybrid_group_documents` merges each
        // document's assembled chunks itself. (Doc ordering is unaffected:
        // merge preserves each doc's max score.)
        if q.merge_chunks && !q.group_by_document {
            merge_adjacent_chunks(&mut fused, &q.text_field);
        }

        // MMR diversity reordering; grouped mode applies `limit` per document
        if mmr {
            mmr_reorder(&mut fused, &q.vector_field, q.mmr_lambda, q.limit);
        } else if !q.group_by_document {
            fused.truncate(q.limit);
        }

        Ok(HybridFusion {
            fused,
            qualification,
        })
    }
}

/// The document a fused chunk belongs to: its `doc_id`, else its own `_id`
pub(super) fn fused_doc_id(item: &FusedResult) -> String {
    item.doc
        .get("doc_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            item.doc
                .get("_id")
                .and_then(id_to_string)
                .unwrap_or_else(|| item.id.clone())
        })
}

/// Whether `projection` (as `find_options::apply_projection` applies it)
/// drops the top-level `field` from a document
fn projection_drops(projection: &HashMap<String, i32>, field: &str) -> bool {
    if projection.values().any(|&v| v == 1) {
        field != "_id" && projection.get(field) != Some(&1)
    } else {
        projection.get(field) == Some(&0)
    }
}

/// `{field: 0, ...}`, or None for no fields
pub(super) fn exclusion_projection(fields: &[String]) -> Option<HashMap<String, i32>> {
    (!fields.is_empty()).then(|| fields.iter().map(|f| (f.clone(), 0)).collect())
}
