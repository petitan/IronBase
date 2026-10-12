//! Distinct operations for CollectionCore
//!
//! # Known Limitations
//!
//! ## No Point-in-Time Snapshot Guarantee
//!
//! The streaming distinct implementation does NOT guarantee a consistent snapshot:
//!
//! 1. Document IDs are collected first (lock released after)
//! 2. Documents are then loaded one-by-one (new lock per read)
//! 3. Concurrent writes between steps can cause:
//!    - Updated documents returning new values instead of original
//!    - Deleted documents being skipped (returning None)
//!    - New documents not being included
//!
//! **Why this trade-off exists:**
//! - Holding lock for entire operation → OOM risk on large collections + lock contention
//! - Snapshot isolation (MVCC) → Complex implementation, more memory
//! - Current streaming approach → OOM-safe, eventually consistent results
//!
//! **Practical impact:** Rare in practice. For strict consistency, wrap in a transaction
//! (note: distinct inside transactions is not yet supported).

use std::collections::HashSet;

use serde_json::Value;

use crate::document::Document;
use crate::error::{IronBaseError, Result};
use crate::execution::ExecutionContext;
use crate::index::IndexKey;
use crate::log_debug;
use crate::storage::{RawStorage, Storage};

use super::CollectionCore;

/// Maximum estimated unique values to pre-allocate (OOM protection)
const MAX_ESTIMATED_UNIQUE: usize = 100_000;

/// Memory check interval for dynamic allocation during distinct collection
const MEMORY_CHECK_INTERVAL: usize = 1000;

impl<S: Storage + RawStorage> CollectionCore<S> {
    /// Distinct values for a field
    /// FIX #19: Now supports nested fields via get_nested_value (e.g., "address.city")
    ///
    /// MEMORY FIX: Uses streaming document loading instead of bulk load.
    /// Previously used scan_documents_via_catalog() which loaded ALL documents
    /// into memory at once - causing OOM on large collections (e.g., 21GB emails).
    /// Now uses collect_doc_ids + streaming read - only IDs in memory, docs loaded one by one.
    pub fn distinct(&self, field: &str, query_json: &Value) -> Result<Vec<Value>> {
        // Batch mode: see the inserts acknowledged before this call
        self.flush_pending_writes()?;
        // Delegate to ctx version with no context
        self.distinct_with_ctx(field, query_json, None)
    }

    /// Get distinct values for a field with execution context for cancellation support.
    ///
    /// This is the main implementation. Pass an ExecutionContext to enable
    /// timeout/cancellation checking, or None for simple usage.
    pub fn distinct_with_ctx(
        &self,
        field: &str,
        query_json: &Value,
        ctx: Option<&ExecutionContext>,
    ) -> Result<Vec<Value>> {
        self.distinct_with_byte_limit(
            field,
            query_json,
            ctx,
            crate::find_options::calculate_safe_response_limit(),
        )
    }

    /// `distinct_with_ctx` with the byte limit of the collected values given:
    /// past `max_bytes` (`estimate_json_size`, as the `find` response limit)
    /// it fails instead of growing without bound (audit 2026-10-07 M7).
    fn distinct_with_byte_limit(
        &self,
        field: &str,
        query_json: &Value,
        ctx: Option<&ExecutionContext>,
        max_bytes: usize,
    ) -> Result<Vec<Value>> {
        // Batch mode: see the inserts acknowledged before this call
        self.flush_pending_writes()?;
        self.check_not_closed()?;

        // Handle _id query optimization
        // FIX #7: Uses normalize_document_id to handle string/int conversion
        // e.g., {"_id": "123"} should match DocumentId::Int(123)
        if let Some(doc_id) = Self::extract_id_query(query_json) {
            // Try original ID first
            if let Some(doc) = self.read_document_by_id(&doc_id)? {
                // PERF: Direct Value→Document conversion (no serialization)
                let document = Document::from_value_owned(doc)?;
                let values: Vec<Value> = document.get_all(field).into_iter().cloned().collect();
                return Ok(values);
            }
            // Try normalized version (string "123" → int 123)
            if let Some(normalized) = Self::normalize_document_id(&doc_id) {
                if let Some(doc) = self.read_document_by_id(&normalized)? {
                    let document = Document::from_value_owned(doc)?;
                    let values: Vec<Value> = document.get_all(field).into_iter().cloned().collect();
                    return Ok(values);
                }
            }
            return Ok(Vec::new());
        }

        // INDEX-BASED OPTIMIZATION: use index to get distinct values without loading documents
        if query_json == &serde_json::json!({}) {
            if let Some(distinct_values) = self.try_index_based_distinct(field, max_bytes)? {
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
        let mut total_bytes: usize = 0;

        let estimated_unique = doc_ids.len().min(MAX_ESTIMATED_UNIQUE);
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
                        total_bytes = total_bytes
                            .saturating_add(crate::find_options::estimate_json_size(field_value));
                        if total_bytes > max_bytes {
                            return Err(distinct_limit_error(distinct_values.len(), max_bytes));
                        }
                        // Dynamic memory check every N new distinct values
                        if distinct_values.len() % MEMORY_CHECK_INTERVAL == 0
                            && !distinct_values.is_empty()
                        {
                            distinct_values
                                .try_reserve(MEMORY_CHECK_INTERVAL)
                                .map_err(|e| {
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
    fn try_index_based_distinct(
        &self,
        field: &str,
        max_bytes: usize,
    ) -> Result<Option<Vec<Value>>> {
        // The index has no null / missing / object values and holds arrays
        // element by element: it gives the scan's answer only if it holds
        // every document once under a scalar key (audit 2026-10-07 Q13)
        if !self.field_index_covers_every_doc(field) {
            return Ok(None);
        }
        let indexes = self.indexes.read();

        // Find an index that covers this field (single-field index, not compound)
        // For compound indexes, we'd need more complex logic
        let index_info = indexes.list_indexes_with_compound_info();

        for info in &index_info {
            // Only use single-field indexes for now
            // The field in index name format is: "{collection}_{field}" or just the field directly
            // A case-insensitive index stores lowercased keys, so it cannot
            // return the original values (audit 2026-10-06 #37).
            if !info.is_compound && !info.case_insensitive && info.prefix_field == field {
                // Found a matching index!
                if let Some(btree) = indexes.get_btree_index(&info.index_name) {
                    // Streaming traversal — O(distinct_keys) memory instead of O(all_entries).
                    // Uses for_each_entry to avoid materializing the entire index.
                    let mut seen: HashSet<IndexKey> = HashSet::new();
                    let mut distinct_values = Vec::new();
                    let mut total_bytes: usize = 0;
                    let mut over_limit = false;

                    btree.for_each_entry(&mut |key, _doc_id| {
                        // Skip Null keys (documents without this field)
                        if over_limit || matches!(key, IndexKey::Null) {
                            return;
                        }
                        if seen.insert(key.clone()) {
                            let value = key.to_value();
                            total_bytes = total_bytes
                                .saturating_add(crate::find_options::estimate_json_size(&value));
                            if total_bytes > max_bytes {
                                over_limit = true;
                                return;
                            }
                            distinct_values.push(value);
                        }
                    });

                    if over_limit {
                        return Err(distinct_limit_error(distinct_values.len(), max_bytes));
                    }
                    return Ok(Some(distinct_values));
                }
            }
        }

        // No suitable index found
        Ok(None)
    }
}

fn distinct_limit_error(collected: usize, max_bytes: usize) -> IronBaseError {
    IronBaseError::InvalidQuery(format!(
        "Response size limit exceeded: distinct collected {} values, \
        the next one would exceed the {} byte limit. \
        Solutions: 1) Add a query filter, 2) Use an aggregation with $group and $limit.",
        collected, max_bytes
    ))
}

#[cfg(test)]
mod tests {
    use crate::storage::MemoryStorage;
    use crate::DatabaseCore;
    use serde_json::{json, Value};
    use std::collections::HashMap;

    /// 100 documents with distinct ~1 KB `s` values.
    fn create_test_db() -> DatabaseCore<MemoryStorage> {
        let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
        for n in 0..100 {
            let s = format!("{:04}{}", n, "y".repeat(1_000));
            let fields: HashMap<String, Value> =
                HashMap::from([("s".to_string(), json!(s)), ("n".to_string(), json!(n))]);
            db.insert_one("c", fields).unwrap();
        }
        db
    }

    /// M7: the scan path fails past the byte limit (~100 KB of values here).
    #[test]
    fn distinct_scan_respects_byte_limit() {
        let db = create_test_db();
        let coll = db.collection("c").unwrap();
        let query = json!({"n": {"$gte": 0}});
        let err = coll
            .distinct_with_byte_limit("s", &query, None, 50_000)
            .unwrap_err();
        assert!(err.to_string().contains("byte limit"), "{err}");
        let all = coll
            .distinct_with_byte_limit("s", &query, None, 1_000_000)
            .unwrap();
        assert_eq!(all.len(), 100);
    }

    /// M7: the index path has the same limit.
    #[test]
    fn distinct_index_respects_byte_limit() {
        let db = create_test_db();
        let coll = db.collection("c").unwrap();
        coll.create_index("s".to_string(), false, false).unwrap();
        let err = coll
            .distinct_with_byte_limit("s", &json!({}), None, 50_000)
            .unwrap_err();
        assert!(err.to_string().contains("byte limit"), "{err}");
        let all = coll
            .distinct_with_byte_limit("s", &json!({}), None, 1_000_000)
            .unwrap();
        assert_eq!(all.len(), 100);
    }
}
