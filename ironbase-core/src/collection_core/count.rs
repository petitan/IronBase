//! Count operations for CollectionCore

use serde_json::Value;

use crate::document::{Document, DocumentId};
use crate::error::{IronBaseError, Result};
use crate::execution::ExecutionContext;
use crate::index::plan_ranges::PlanRanges;
use crate::index::{BPlusTree, IndexManager};
use crate::query::Query;
use crate::query_planner::{QueryPlan, QueryPlanner};
use crate::storage::{RawStorage, Storage};
use std::collections::HashMap;

use super::CollectionCore;

/// Operators that a plan provably encodes in its index range/keys.
///
/// The fully-covered count fast path (`count_range_validated`) does NOT
/// post-filter, so it is only safe when EVERY operator on the indexed field is
/// in this set. Any other operator (e.g. a `$ne` next to a range, a second
/// membership op, a `$type`) would be silently dropped, making count diverge
/// from `find` (audit 2026-06-06 finding B).
fn plan_covered_operators(plan: &QueryPlan) -> &'static [&'static str] {
    match plan {
        QueryPlan::IndexScan { .. } => &["$eq"],
        QueryPlan::MultiValueScan { .. } => &["$in"],
        QueryPlan::IndexRangeScan { .. } => &["$gt", "$gte", "$lt", "$lte"],
        QueryPlan::RegexPrefixScan { .. } | QueryPlan::MultiRegexPrefixScan { .. } => {
            &["$regex", "$options"]
        }
        QueryPlan::SparseIndexScan { .. } => &["$exists"],
    }
}

/// Check if the query is fully covered by the index plan's field.
///
/// Returns true when the query constrains ONLY the indexed field AND every
/// operator on that field is provably encoded by the chosen plan — meaning the
/// O(1) `count_range_validated` fast path (which does not post-filter) is
/// accurate. Returns false when additional conditions exist, so count routes
/// through the index-narrowed post-filter path instead.
///
/// This prevents two bug classes:
/// - cross-field residual, e.g. `{"doc_type": "X", "content": {"$regex": "..."}}`
///   returning the doc_type index count without applying the $regex; and
/// - same-field residual, e.g. `{"f": {"$gte": 5, "$ne": 7}}` or
///   `{"f": {"$in": [..], "$gte": 5}}`, where the plan encodes only one operator
///   and the others would be silently dropped (audit 2026-06-06 finding B).
fn query_fully_covered_by_plan(query_json: &Value, plan: &QueryPlan) -> bool {
    let map = match query_json.as_object() {
        Some(m) => m,
        None => return false,
    };

    let plan_field = match plan {
        QueryPlan::IndexScan { ref field, .. } => field,
        QueryPlan::IndexRangeScan { ref field, .. } => field,
        QueryPlan::RegexPrefixScan { ref field, .. } => field,
        QueryPlan::MultiRegexPrefixScan { ref field, .. } => field,
        QueryPlan::MultiValueScan { ref field, .. } => field,
        QueryPlan::SparseIndexScan { ref field, .. } => field,
    };

    // Must constrain ONLY the indexed field.
    if map.len() != 1 || !map.contains_key(plan_field) {
        return false;
    }

    // The single field's condition must be fully ENCODED by the plan, not just
    // present on the field. A bare scalar / plain-object equality is covered
    // only by an equality IndexScan; an operator object is covered only when
    // every $-operator key is in the plan's covered set.
    let cond = match map[plan_field].as_object() {
        None => return matches!(plan, QueryPlan::IndexScan { .. }),
        Some(c) => c,
    };
    if cond.keys().all(|k| !k.starts_with('$')) {
        // Plain nested-object value (no operators) — an object equality, which
        // is never index-covered (is_indexable_value blocks it); treat like a
        // scalar equality.
        return matches!(plan, QueryPlan::IndexScan { .. });
    }

    let covered = plan_covered_operators(plan);
    cond.keys().all(|k| covered.contains(&k.as_str()))
}

/// Whether the plan's index is multikey (array-valued). The fully-covered count
/// fast path (`count_range_validated` / `count_numeric_buckets`) counts index
/// ENTRIES, so a multikey document indexed under several matching keys would be
/// over-counted. Such plans must take the index-narrowed path instead, which
/// deduplicates doc_ids before counting — matching find()'s multikey dedup.
fn plan_index_is_multikey(indexes: &IndexManager, plan: &QueryPlan) -> bool {
    indexes
        .get_btree_index(plan.index_name())
        .map(|i| i.metadata.multikey)
        .unwrap_or(false)
}

/// The two ways a fully-planned count resolves. Returned by [`count_for_plan`]
/// so the **caller** owns the lock-drop discipline: `Exact` is computed while
/// `storage`+`indexes` are held (it reads the document catalog), whereas
/// `Narrow` hands the index-matched candidate doc_ids back so the caller can
/// drop BOTH guards (issue #75 storage→indexes order) before delegating to the
/// re-entrant `count_index_narrowed`.
enum CountOutcome {
    /// O(1)-memory catalog-validated count — the plan's ranges are exact, no
    /// post-filter needed.
    Exact(u64),
    /// Index-narrowed candidate doc_ids — the caller must post-filter the full
    /// query against them.
    Narrow(Vec<DocumentId>),
}

/// Resolve a plan to a [`CountOutcome`] via the shared range chokepoint (audit
/// finding #8 — the single `PlanRanges::from_plan` derivation that count, find
/// and the cursor all share). Pure index math plus a catalog read; acquires NO
/// locks itself.
///
/// The exact O(1) fast path is taken only when the caller's coverage + multikey
/// gates pass (`fully_covered`) AND the plan's raw ranges are exact
/// ([`PlanRanges::exact`], which folds in the regex `exact && !case_insensitive`
/// rule). Otherwise the ranges are materialized into candidate doc_ids for the
/// caller to post-filter.
fn count_for_plan(
    index: &BPlusTree,
    plan: &QueryPlan,
    catalog: &HashMap<DocumentId, u64>,
    fully_covered: bool,
) -> Result<CountOutcome> {
    let ranges = PlanRanges::from_plan(plan, index);
    if fully_covered && ranges.exact {
        Ok(CountOutcome::Exact(
            index.count_exact(&ranges, catalog) as u64
        ))
    } else {
        Ok(CountOutcome::Narrow(index.scan_ranges(&ranges)?))
    }
}

impl<S: Storage + RawStorage> CollectionCore<S> {
    /// Count documents matching query
    ///
    /// Uses QueryPlanner for index optimization when available.
    /// Performance optimized: Uses streaming count without Vec allocation for scans.
    pub fn count_documents(&self, query_json: &Value) -> Result<u64> {
        self.check_not_closed()?;

        // Fast path: empty query = count all (O(1))
        if Self::query_matches_all(query_json) {
            let storage = self.storage.read();
            return Ok(storage.get_live_count(&self.name).unwrap_or(0));
        }

        // Fast path: _id query = O(1) lookup
        // FIX #7: Uses normalize_document_id to handle string/int conversion
        if let Some(doc_id) = Self::extract_id_query(query_json) {
            // Try original ID first
            if self.read_document_by_id(&doc_id)?.is_some() {
                return Ok(1);
            }
            // Try normalized version (string "123" → int 123)
            if let Some(normalized) = Self::normalize_document_id(&doc_id) {
                if self.read_document_by_id(&normalized)?.is_some() {
                    return Ok(1);
                }
            }
            return Ok(0);
        }

        // Fast path: _id $in query = O(k) lookups (k = number of IDs)
        // Note: Uses normalize_document_id to handle string/int conversion
        if let Some(doc_ids) = Self::extract_id_in_query(query_json) {
            let mut count = 0u64;
            for doc_id in doc_ids {
                // Try both the original ID and normalized version
                // This handles {"$in": ["123"]} matching DocumentId::Int(123)
                if self.read_document_by_id(&doc_id)?.is_some() {
                    count += 1;
                } else if let Some(normalized) = Self::normalize_document_id(&doc_id) {
                    if self.read_document_by_id(&normalized)?.is_some() {
                        count += 1;
                    }
                }
            }
            return Ok(count);
        }

        // Index-aware count with Vec-less fallback
        self.count_with_plan(query_json, None)
    }

    /// Count documents matching a query with execution context for cancellation support.
    ///
    /// This is the cancellation-aware version of `count_documents`.
    /// Pass an ExecutionContext to enable timeout/cancellation checking.
    pub fn count_documents_with_ctx(
        &self,
        query_json: &Value,
        ctx: Option<&ExecutionContext>,
    ) -> Result<u64> {
        self.check_not_closed()?;

        // Fast path: empty query = count all (O(1))
        if Self::query_matches_all(query_json) {
            let storage = self.storage.read();
            return Ok(storage.get_live_count(&self.name).unwrap_or(0));
        }

        // Fast path: _id query = O(1) lookup
        // FIX #7: Uses normalize_document_id to handle string/int conversion
        if let Some(doc_id) = Self::extract_id_query(query_json) {
            // Try original ID first
            if self.read_document_by_id(&doc_id)?.is_some() {
                return Ok(1);
            }
            // Try normalized version (string "123" → int 123)
            if let Some(normalized) = Self::normalize_document_id(&doc_id) {
                if self.read_document_by_id(&normalized)?.is_some() {
                    return Ok(1);
                }
            }
            return Ok(0);
        }

        // Fast path: _id $in query = O(k) lookups (k = number of IDs)
        // Note: Uses normalize_document_id to handle string/int conversion
        if let Some(doc_ids) = Self::extract_id_in_query(query_json) {
            let mut count = 0u64;
            for doc_id in doc_ids {
                // Try both the original ID and normalized version
                // This handles {"$in": ["123"]} matching DocumentId::Int(123)
                if self.read_document_by_id(&doc_id)?.is_some() {
                    count += 1;
                } else if let Some(normalized) = Self::normalize_document_id(&doc_id) {
                    if self.read_document_by_id(&normalized)?.is_some() {
                        count += 1;
                    }
                }
            }
            return Ok(count);
        }

        // Index-aware count with Vec-less fallback
        self.count_with_plan(query_json, ctx)
    }

    /// Count using QueryPlanner for index optimization
    ///
    /// Tries to use available indexes for faster counting.
    /// When the query is fully covered by the index (single field match),
    /// trusts the index count directly for O(1) memory.
    /// When the query has additional conditions beyond the indexed field,
    /// uses index-narrowed post-filter: fetches doc_ids from index, then
    /// streams through them applying the full query match.
    /// Falls back to streaming scan if no suitable index exists.
    ///
    /// FIX #36: Previously returned unfiltered index counts when query had
    /// conditions on multiple fields (e.g., $regex on a non-indexed field).
    fn count_with_plan(&self, query_json: &Value, ctx: Option<&ExecutionContext>) -> Result<u64> {
        // Try index-based counting
        //
        // LOCK ORDER (issue #75): acquire `storage` BEFORE `indexes`. The write
        // paths (insert/update *_raw) hold `storage.write()` across their index
        // updates for read-modify-write atomicity ($inc), i.e. they lock in
        // storage→indexes order. Co-holding here in the opposite order
        // (indexes→storage) produced a classic ABBA deadlock: a concurrent
        // insert_many holding storage.write waited for indexes.write while this
        // count held indexes.read and waited for storage.read. We adopt the same
        // global storage→indexes order. Branches that delegate (count_index_narrowed
        // / count_with_scan, which re-acquire storage themselves) drop BOTH guards
        // first to avoid a re-entrant read deadlock.
        let storage = self.storage.read();
        let indexes = self.indexes.read();
        let index_fields = indexes.list_indexes_with_compound_info();

        if let Some((logical_op, clauses)) = QueryPlanner::extract_logical_clauses(query_json) {
            // Try clause merge optimization first: rewrite $and/$or to implicit form
            // so the standard planner can use a single index scan instead of HashSet merge.
            if let Some(merged_query) =
                QueryPlanner::try_merge_logical_clauses(logical_op, &clauses, &index_fields)
            {
                if let Some((_, plan)) =
                    QueryPlanner::analyze_query_with_fields(&merged_query, &index_fields)
                {
                    let fully_covered = query_fully_covered_by_plan(&merged_query, &plan)
                        && !plan_index_is_multikey(&indexes, &plan);

                    // Single shared derivation (audit finding #8). A merged
                    // $and/$or plan references an index that exists in this same
                    // locked `indexes`, so get_btree_index always succeeds here;
                    // count_for_plan resolves every variant uniformly.
                    if let Some(index) = indexes.get_btree_index(plan.index_name()) {
                        let outcome = {
                            let meta =
                                storage.get_collection_meta(&self.name).ok_or_else(|| {
                                    IronBaseError::CollectionNotFound(self.name.clone())
                                })?;
                            count_for_plan(index, &plan, &meta.document_catalog, fully_covered)?
                        };
                        match outcome {
                            CountOutcome::Exact(n) => return Ok(n),
                            CountOutcome::Narrow(doc_ids) => {
                                drop(indexes);
                                drop(storage);
                                return self.count_index_narrowed(doc_ids, query_json, ctx);
                            }
                        }
                    }
                }
            }

            // Fallback: original per-clause logical operator handling
            drop(indexes);
            drop(storage);
            let parsed_query = Query::from_json(query_json)?;
            let cancel_flag = ctx.and_then(|c| c.cancel_flag());
            let deadline = ctx.and_then(|c| c.deadline());
            if let Some((doc_ids, _)) = self.collect_doc_ids_for_logical_operator(
                &parsed_query,
                logical_op,
                &clauses,
                None,
                false,
                0,
                None,
                cancel_flag,
                deadline,
            )? {
                return Ok(doc_ids.len() as u64);
            }
            return self.count_with_scan(query_json, ctx);
        }

        if let Some((_, plan)) = QueryPlanner::analyze_query_with_fields(query_json, &index_fields)
        {
            let fully_covered = query_fully_covered_by_plan(query_json, &plan)
                && !plan_index_is_multikey(&indexes, &plan);

            // Single shared derivation (audit finding #8). The plan references an
            // index present in this same locked `indexes`, so get_btree_index
            // always succeeds; count_for_plan resolves every variant uniformly
            // (the regex `exact && !case_insensitive` rule lives in PlanRanges).
            if let Some(index) = indexes.get_btree_index(plan.index_name()) {
                let outcome = {
                    let meta = storage
                        .get_collection_meta(&self.name)
                        .ok_or_else(|| IronBaseError::CollectionNotFound(self.name.clone()))?;
                    count_for_plan(index, &plan, &meta.document_catalog, fully_covered)?
                };
                match outcome {
                    CountOutcome::Exact(n) => return Ok(n),
                    CountOutcome::Narrow(doc_ids) => {
                        drop(indexes);
                        drop(storage);
                        return self.count_index_narrowed(doc_ids, query_json, ctx);
                    }
                }
            }

            drop(indexes);
            drop(storage);
        } else {
            drop(indexes);
            drop(storage);
        }

        // Fallback: streaming count without Vec allocation
        self.count_with_scan(query_json, ctx)
    }

    /// Count documents from an index-narrowed set of doc_ids by applying the full query filter.
    ///
    /// FIX #36: When a query has conditions beyond the indexed field (e.g.,
    /// `{doc_type: "X", content: {$regex: "..."}}` with index on doc_type),
    /// the index narrows candidates but the remaining conditions must be verified.
    ///
    /// Uses chunked processing identical to count_with_scan() for OOM safety.
    /// Performance: Only loads index-matched documents instead of full collection.
    fn count_index_narrowed(
        &self,
        doc_ids: Vec<DocumentId>,
        query_json: &Value,
        ctx: Option<&ExecutionContext>,
    ) -> Result<u64> {
        /// Max documents per chunk — matches count_with_scan() for consistency
        const CHUNK_SIZE: usize = 1000;

        // Dedup doc_ids up front: a multikey index can yield the same doc_id
        // multiple times (one per matching array element, or across the Int and
        // Float numeric buckets). This counts one match per occurrence, so without
        // dedup a multikey doc would be over-counted. Single chokepoint covering
        // every narrowed plan path (IndexRangeScan/MultiValue/MultiRegex/Regex).
        let mut doc_ids = doc_ids;
        doc_ids.sort_unstable();
        doc_ids.dedup();

        let parsed_query = Query::from_json(query_json)?;
        let mut total_count = 0u64;

        for (chunks_processed, chunk) in doc_ids.chunks(CHUNK_SIZE).enumerate() {
            // Check for cancellation at the start of each chunk
            if let Some(exec_ctx) = ctx {
                exec_ctx.maybe_check(chunks_processed)?;
            }

            // Phase 1: Read documents by ID (sequential)
            let raw_docs: Vec<Value> = chunk
                .iter()
                .filter_map(|doc_id| self.read_document_by_id(doc_id).ok().flatten())
                .collect();

            // Phase 2: Parallel parse + match (CPU bound)
            #[cfg(feature = "parallel")]
            {
                use rayon::prelude::*;

                total_count += raw_docs
                    .par_iter()
                    .filter(|doc| {
                        // Skip tombstones
                        if doc
                            .get("_tombstone")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false)
                        {
                            return false;
                        }

                        // Apply full query filter
                        match Document::from_value_owned((*doc).clone()) {
                            Ok(document) => parsed_query.matches(&document).unwrap_or(false),
                            Err(_) => false,
                        }
                    })
                    .count() as u64;
            }

            #[cfg(not(feature = "parallel"))]
            {
                for doc in raw_docs {
                    if doc
                        .get("_tombstone")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                    {
                        continue;
                    }

                    let document = match Document::from_value_owned(doc) {
                        Ok(d) => d,
                        Err(_) => continue,
                    };

                    if parsed_query.matches(&document).unwrap_or(false) {
                        total_count += 1;
                    }
                }
            }
            // raw_docs freed here — memory reclaimed before next chunk
        }

        Ok(total_count)
    }

    /// Count by chunked parallel scan (fast AND memory-safe)
    ///
    /// This is the fallback when no index is available.
    /// Uses chunked parallel processing:
    /// - Chunks of 1000 docs max (~500MB memory per chunk)
    /// - Parallel JSON parse + query matching within each chunk
    /// - Memory freed after each chunk
    ///
    /// ⚠️ OOM PREVENTION: Chunks prevent loading all docs at once!
    /// Lásd: CLAUDE.md "OOM Prevention" szekció
    /// Performance: ~30-50s instead of 274s for 78K emails
    fn count_with_scan(&self, query_json: &Value, ctx: Option<&ExecutionContext>) -> Result<u64> {
        /// Max documents per chunk - limits memory to ~500MB
        const CHUNK_SIZE: usize = 1000;

        let parsed_query = Query::from_json(query_json)?;
        let storage = self.storage.read();

        // Get catalog entries (small: only IDs + offsets, ~32 bytes each)
        let catalog_entries: Vec<(DocumentId, u64)> = {
            let meta = storage
                .get_collection_meta(&self.name)
                .ok_or_else(|| IronBaseError::CollectionNotFound(self.name.clone()))?;
            meta.document_catalog
                .iter()
                .map(|(id, &offset)| (id.clone(), offset))
                .collect()
        };

        let mut total_count = 0u64;

        // Process in chunks to limit memory usage
        for (chunks_processed, chunk) in catalog_entries.chunks(CHUNK_SIZE).enumerate() {
            // Check for cancellation at the start of each chunk
            if let Some(exec_ctx) = ctx {
                exec_ctx.maybe_check(chunks_processed)?;
            }

            // Phase 1: Read chunk bytes (sequential, holds lock)
            let raw_docs: Vec<Vec<u8>> = chunk
                .iter()
                .filter_map(|(_, offset)| storage.read_data_at(*offset).ok())
                .collect();

            // Phase 2: Parallel parse + match (CPU bound)
            //
            // Parser/document errors (malformed bytes, bad UTF-8) are SKIPPED
            // — corrupt-data robustness. Matcher errors (e.g. an invalid
            // query like an empty `$and: []` array) are PROPAGATED so the
            // caller sees the malformed query instead of a silent zero
            // (audit #28 follow-up: previously `.unwrap_or(false)` swallowed
            // every matcher Err and the count silently undershot).
            #[cfg(feature = "parallel")]
            {
                use rayon::prelude::*;

                let chunk_count: u64 = raw_docs
                    .par_iter()
                    .map(|doc_bytes| -> Result<u64> {
                        let doc: Value = match serde_json::from_slice(doc_bytes) {
                            Ok(d) => d,
                            Err(_) => return Ok(0), // malformed JSON, skip
                        };
                        if doc
                            .get("_tombstone")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false)
                        {
                            return Ok(0);
                        }
                        let document = match Document::from_value_owned(doc) {
                            Ok(d) => d,
                            Err(_) => return Ok(0), // malformed document, skip
                        };
                        if parsed_query.matches(&document)? {
                            Ok(1)
                        } else {
                            Ok(0)
                        }
                    })
                    .try_reduce(|| 0u64, |a, b| Ok(a + b))?;
                total_count += chunk_count;
            }

            #[cfg(not(feature = "parallel"))]
            {
                for doc_bytes in raw_docs {
                    let doc: Value = match serde_json::from_slice(&doc_bytes) {
                        Ok(d) => d,
                        Err(_) => continue,
                    };

                    if doc
                        .get("_tombstone")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                    {
                        continue;
                    }

                    let document = match Document::from_value_owned(doc) {
                        Ok(d) => d,
                        Err(_) => continue,
                    };

                    // Matcher errors propagate (audit #28 follow-up — no more
                    // `.unwrap_or(false)` swallowing of malformed-query errors).
                    if parsed_query.matches(&document)? {
                        total_count += 1;
                    }
                }
            }
            // raw_docs freed here - memory reclaimed before next chunk
        }

        Ok(total_count)
    }
}
