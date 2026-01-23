//! Count operations for CollectionCore

use serde_json::Value;

use crate::document::{Document, DocumentId};
use crate::error::{IronBaseError, Result};
use crate::execution::ExecutionContext;
use crate::index::{IndexKey, RangeQueryMode};
use crate::query::Query;
use crate::query_planner::{QueryPlan, QueryPlanner};
use crate::storage::{RawStorage, Storage};

use super::CollectionCore;

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
        if let Some(doc_id) = Self::extract_id_query(query_json) {
            return Ok(if self.read_document_by_id(&doc_id)?.is_some() {
                1
            } else {
                0
            });
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
        if let Some(doc_id) = Self::extract_id_query(query_json) {
            return Ok(if self.read_document_by_id(&doc_id)?.is_some() {
                1
            } else {
                0
            });
        }

        // Index-aware count with Vec-less fallback
        self.count_with_plan(query_json, ctx)
    }

    /// Count using QueryPlanner for index optimization
    ///
    /// Tries to use available indexes for faster counting.
    /// For simple equality queries, trusts the index count directly.
    /// Falls back to streaming scan if no suitable index exists.
    ///
    /// 🔧 REFACTORED: Uses unified range_query(Count) for O(1) memory index counting.
    fn count_with_plan(&self, query_json: &Value, ctx: Option<&ExecutionContext>) -> Result<u64> {
        use crate::index::RangeQueryResult;

        // Try index-based counting
        let indexes = self.indexes.read();
        let index_fields = indexes.list_indexes_with_compound_info();

        if let Some((logical_op, clauses)) = QueryPlanner::extract_logical_clauses(query_json) {
            drop(indexes);
            let parsed_query = Query::from_json(query_json)?;
            // Extract cancel_flag and deadline from ExecutionContext
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
            match &plan {
                QueryPlan::IndexScan {
                    ref index_name,
                    ref key,
                    is_compound,
                    ..
                } => {
                    if let Some(index) = indexes.get_btree_index(index_name) {
                        // Use range_query with Count mode - O(1) memory!
                        let (start, end) = if *is_compound {
                            index.build_prefix_range(key.clone())
                        } else {
                            (key.clone(), key.clone())
                        };

                        let result =
                            index.range_query(&start, &end, true, true, RangeQueryMode::Count);

                        let raw_count = match result {
                            RangeQueryResult::Count(c) => c,
                            _ => unreachable!("Count mode always returns Count"),
                        };
                        drop(indexes);

                        // Need to verify tombstones - use sampling or trust if no tombstones
                        return self.adjust_count_for_tombstones(raw_count);
                    }
                }
                QueryPlan::IndexRangeScan {
                    ref index_name,
                    ref start,
                    ref end,
                    inclusive_start,
                    inclusive_end,
                    ..
                } => {
                    if let Some(index) = indexes.get_btree_index(index_name) {
                        let default_start = IndexKey::Null;
                        let default_end = IndexKey::String("\u{10ffff}".repeat(100));
                        let start_key = start.as_ref().unwrap_or(&default_start);
                        let end_key = end.as_ref().unwrap_or(&default_end);

                        // Use range_query with Count mode - O(1) memory!
                        let result = index.range_query(
                            start_key,
                            end_key,
                            *inclusive_start,
                            *inclusive_end,
                            RangeQueryMode::Count,
                        );

                        let raw_count = match result {
                            RangeQueryResult::Count(c) => c,
                            _ => unreachable!("Count mode always returns Count"),
                        };
                        drop(indexes);

                        return self.adjust_count_for_tombstones(raw_count);
                    }
                }
                QueryPlan::RegexPrefixScan {
                    ref index_name,
                    ref prefix,
                    exact,
                    ..
                } => {
                    // Only use index count when exact=true (pure prefix, no regex verification needed)
                    if *exact {
                        if let Some(index) = indexes.get_btree_index(index_name) {
                            let start = IndexKey::String(prefix.clone());
                            let end = IndexKey::String(format!("{}\u{10ffff}", prefix));

                            // Use range_query with Count mode - O(1) memory!
                            let result =
                                index.range_query(&start, &end, true, true, RangeQueryMode::Count);

                            let raw_count = match result {
                                RangeQueryResult::Count(c) => c,
                                _ => unreachable!("Count mode always returns Count"),
                            };
                            drop(indexes);

                            return self.adjust_count_for_tombstones(raw_count);
                        }
                    }
                }
                QueryPlan::MultiRegexPrefixScan {
                    ref index_name,
                    ref prefixes,
                    ..
                } => {
                    // Count for multi-regex prefix: sum of all prefix counts
                    // Note: This may over-count if prefixes overlap, but exact counting
                    // would require deduplication which defeats the purpose
                    if let Some(index) = indexes.get_btree_index(index_name) {
                        let mut total_count = 0usize;
                        for prefix in prefixes {
                            let start = IndexKey::String(prefix.clone());
                            let end = IndexKey::String(format!("{}\u{10ffff}", prefix));
                            let result =
                                index.range_query(&start, &end, true, true, RangeQueryMode::Count);
                            if let RangeQueryResult::Count(c) = result {
                                total_count += c;
                            }
                        }
                        drop(indexes);
                        return self.adjust_count_for_tombstones(total_count);
                    }
                }
                QueryPlan::SparseIndexScan { ref index_name, .. } => {
                    // Sparse index: all entries represent docs where field exists
                    // Simply count all entries in the index - O(1) operation
                    if let Some(index) = indexes.get_btree_index(index_name) {
                        let raw_count = index.metadata.num_keys as usize;
                        drop(indexes);
                        return self.adjust_count_for_tombstones(raw_count);
                    }
                }
            }

            drop(indexes);
        } else {
            drop(indexes);
        }

        // Fallback: streaming count without Vec allocation
        self.count_with_scan(query_json, ctx)
    }

    /// Adjust index count for tombstones.
    ///
    /// If no tombstones exist, returns the raw count directly.
    /// Otherwise, applies an estimated adjustment ratio.
    fn adjust_count_for_tombstones(&self, raw_count: usize) -> Result<u64> {
        let storage = self.storage.read();
        let live_count = storage.get_live_count(&self.name).unwrap_or(0) as usize;

        let meta = storage
            .get_collection_meta(&self.name)
            .ok_or_else(|| IronBaseError::CollectionNotFound(self.name.clone()))?;
        let catalog_len = meta.document_catalog.len();

        if live_count == catalog_len || catalog_len == 0 {
            // No tombstones - trust index count
            Ok(raw_count as u64)
        } else {
            // Apply tombstone ratio estimation
            // This is approximate but avoids loading all documents
            let live_ratio = live_count as f64 / catalog_len as f64;
            Ok((raw_count as f64 * live_ratio).round() as u64)
        }
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
            #[cfg(feature = "parallel")]
            {
                use rayon::prelude::*;

                total_count += raw_docs
                    .par_iter()
                    .filter(|doc_bytes| {
                        // Parse document
                        let doc: Value = match serde_json::from_slice(doc_bytes) {
                            Ok(d) => d,
                            Err(_) => return false,
                        };

                        // Skip tombstones
                        if doc
                            .get("_tombstone")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false)
                        {
                            return false;
                        }

                        // Apply query filter
                        match Document::from_value_owned(doc) {
                            Ok(document) => parsed_query.matches(&document).unwrap_or(false),
                            Err(_) => false,
                        }
                    })
                    .count() as u64;
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

                    if parsed_query.matches(&document).unwrap_or(false) {
                        total_count += 1;
                    }
                }
            }
            // raw_docs freed here - memory reclaimed before next chunk
        }

        Ok(total_count)
    }
}
