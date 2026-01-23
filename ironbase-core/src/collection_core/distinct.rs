//! Distinct operations for CollectionCore

use std::collections::HashSet;

use serde_json::Value;

use crate::document::Document;
use crate::error::{IronBaseError, Result};
use crate::execution::ExecutionContext;
use crate::index::IndexKey;
use crate::log_debug;
use crate::storage::{RawStorage, Storage};

use super::CollectionCore;

impl<S: Storage + RawStorage> CollectionCore<S> {
    /// Distinct values for a field
    /// FIX #19: Now supports nested fields via get_nested_value (e.g., "address.city")
    ///
    /// MEMORY FIX: Uses streaming document loading instead of bulk load.
    /// Previously used scan_documents_via_catalog() which loaded ALL documents
    /// into memory at once - causing OOM on large collections (e.g., 21GB emails).
    /// Now uses collect_doc_ids + streaming read - only IDs in memory, docs loaded one by one.
    pub fn distinct(&self, field: &str, query_json: &Value) -> Result<Vec<Value>> {
        self.check_not_closed()?;

        // Handle _id query optimization
        if let Some(doc_id) = Self::extract_id_query(query_json) {
            if let Some(doc) = self.read_document_by_id(&doc_id)? {
                // PERF: Direct Value→Document conversion (no serialization)
                let document = Document::from_value_owned(doc)?;
                // Use Document::get_all for MongoDB-style array traversal
                let values: Vec<Value> = document.get_all(field).into_iter().cloned().collect();
                return Ok(values);
            }
            return Ok(Vec::new());
        }

        // INDEX-BASED OPTIMIZATION: If query is empty {} and there's an index on the field,
        // use the index to get distinct values without loading any documents.
        // This is O(index_size) instead of O(all_documents) - huge speedup for large collections.
        if query_json == &serde_json::json!({}) {
            if let Some(distinct_values) = self.try_index_based_distinct(field)? {
                log_debug!(
                    "distinct: used index for field '{}', found {} unique values",
                    field,
                    distinct_values.len()
                );
                return Ok(distinct_values);
            }
        }

        // STREAMING FALLBACK: When no index is available or query has filters
        // Collect only document IDs first (small: ~8-32 bytes each)
        // Then stream-load documents one by one - never bulk load all docs into memory
        let (doc_ids, _) = self.collect_doc_ids_with_options(
            query_json, None, None, false, 0, None, true, 0, None, None, None,
        )?;

        // Collect distinct values - stream documents one by one
        // OOM PROTECTION: Use try_reserve for dynamic memory checking
        // PERF: Use value_hash (u64) instead of canonical_json_string (String) for 10-50x faster dedup
        let mut seen_hashes: HashSet<u64> = HashSet::new();
        let mut distinct_values = Vec::new();

        // Pre-check: try to reserve for estimated unique values (capped at 100K)
        let estimated_unique = doc_ids.len().min(100_000);
        distinct_values.try_reserve(estimated_unique).map_err(|e| {
            IronBaseError::OutOfMemory(format!(
                "Cannot allocate for ~{} distinct values ({}). \
                Solutions: 1) Use indexed distinct, 2) Add a query filter, 3) Increase system memory.",
                estimated_unique, e
            ))
        })?;

        for doc_id in doc_ids {
            // Load document one at a time - O(1) memory per iteration
            if let Some(doc) = self.read_document_by_id(&doc_id)? {
                // PERF: Direct Value→Document conversion (no serialization)
                // Old: serde_json::to_string(&doc) + Document::from_json() = 2x serialization
                // New: from_value_owned() = 0 serialization, just moves ownership
                let document = Document::from_value_owned(doc)?;

                // Use Document::get_all() for MongoDB-style implicit array traversal
                // This properly handles dot notation with arrays like "items.name"
                for field_value in document.get_all(field) {
                    // PERF: Use value_hash instead of canonical_json_string (10-50x faster)
                    // Both provide deterministic deduplication for objects with sorted keys
                    let value_hash = crate::value_utils::value_hash(field_value);

                    if seen_hashes.insert(value_hash) {
                        // Dynamic memory check every 1000 new distinct values
                        if distinct_values.len() % 1000 == 0 && !distinct_values.is_empty() {
                            distinct_values.try_reserve(1000).map_err(|e| {
                                IronBaseError::OutOfMemory(format!(
                                    "Out of memory at {} distinct values ({}). \
                                    Consider using a more restrictive query filter.",
                                    distinct_values.len(),
                                    e
                                ))
                            })?;
                        }
                        distinct_values.push(field_value.clone());
                    }
                }
            }
        }

        Ok(distinct_values)
    }

    /// Get distinct values for a field with execution context for cancellation support.
    ///
    /// This is the cancellation-aware version of `distinct`.
    /// Pass an ExecutionContext to enable timeout/cancellation checking.
    pub fn distinct_with_ctx(
        &self,
        field: &str,
        query_json: &Value,
        ctx: Option<&ExecutionContext>,
    ) -> Result<Vec<Value>> {
        self.check_not_closed()?;

        // Handle _id query optimization
        if let Some(doc_id) = Self::extract_id_query(query_json) {
            if let Some(doc) = self.read_document_by_id(&doc_id)? {
                // PERF: Direct Value→Document conversion (no serialization)
                let document = Document::from_value_owned(doc)?;
                let values: Vec<Value> = document.get_all(field).into_iter().cloned().collect();
                return Ok(values);
            }
            return Ok(Vec::new());
        }

        // INDEX-BASED OPTIMIZATION: use index to get distinct values without loading documents
        if query_json == &serde_json::json!({}) {
            if let Some(distinct_values) = self.try_index_based_distinct(field)? {
                log_debug!(
                    "distinct: used index for field '{}', found {} unique values",
                    field,
                    distinct_values.len()
                );
                return Ok(distinct_values);
            }
        }

        // STREAMING FALLBACK: collect document IDs first, then stream-load one by one
        // Extract cancel_flag and deadline from ExecutionContext
        let cancel_flag = ctx.and_then(|c| c.cancel_flag().cloned());
        let deadline = ctx.and_then(|c| c.deadline());
        let (doc_ids, _) = self.collect_doc_ids_with_options(
            query_json,
            None,
            None,
            false,
            0,
            None,
            true,
            0,
            None,
            cancel_flag.as_ref(),
            deadline,
        )?;

        // Collect distinct values with cancellation checks
        let mut seen_hashes: HashSet<u64> = HashSet::new();
        let mut distinct_values = Vec::new();

        let estimated_unique = doc_ids.len().min(100_000);
        distinct_values.try_reserve(estimated_unique).map_err(|e| {
            IronBaseError::OutOfMemory(format!(
                "Cannot allocate for ~{} distinct values ({}). \
                Solutions: 1) Use indexed distinct, 2) Add a query filter, 3) Increase system memory.",
                estimated_unique, e
            ))
        })?;

        for (iteration, doc_id) in doc_ids.iter().enumerate() {
            // Check for cancellation every N iterations
            if let Some(exec_ctx) = ctx {
                exec_ctx.maybe_check(iteration)?;
            }

            if let Some(doc) = self.read_document_by_id(doc_id)? {
                let document = Document::from_value_owned(doc)?;

                for field_value in document.get_all(field) {
                    let value_hash = crate::value_utils::value_hash(field_value);

                    if seen_hashes.insert(value_hash) {
                        if distinct_values.len() % 1000 == 0 && !distinct_values.is_empty() {
                            distinct_values.try_reserve(1000).map_err(|e| {
                                IronBaseError::OutOfMemory(format!(
                                    "Out of memory at {} distinct values ({}). \
                                    Consider using a more restrictive query filter.",
                                    distinct_values.len(),
                                    e
                                ))
                            })?;
                        }
                        distinct_values.push(field_value.clone());
                    }
                }
            }
        }

        Ok(distinct_values)
    }

    /// Try to get distinct values using an index on the field
    ///
    /// Returns Some(values) if an index was found and used, None otherwise.
    /// This is O(index_entries) which is much faster than loading all documents.
    fn try_index_based_distinct(&self, field: &str) -> Result<Option<Vec<Value>>> {
        let indexes = self.indexes.read();

        // Find an index that covers this field (single-field index, not compound)
        // For compound indexes, we'd need more complex logic
        let index_info = indexes.list_indexes_with_compound_info();

        for info in &index_info {
            // Only use single-field indexes for now
            // The field in index name format is: "{collection}_{field}" or just the field directly
            if !info.is_compound && info.prefix_field == field {
                // Found a matching index!
                if let Some(btree) = indexes.get_btree_index(&info.index_name) {
                    // Get all entries from the index
                    let entries = btree.get_all_entries();

                    // Extract unique keys (the B+ tree already has them in order)
                    // But we need to deduplicate because non-unique indexes have duplicates
                    let mut seen: HashSet<IndexKey> = HashSet::new();
                    let mut distinct_values = Vec::new();

                    for (key, _doc_id) in entries {
                        // Skip Null keys (documents without this field)
                        if matches!(key, IndexKey::Null) {
                            continue;
                        }
                        if seen.insert(key.clone()) {
                            distinct_values.push(key.to_value());
                        }
                    }

                    return Ok(Some(distinct_values));
                }
            }
        }

        // No suitable index found
        Ok(None)
    }
}
