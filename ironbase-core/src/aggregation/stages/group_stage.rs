// src/aggregation/stages/group_stage.rs
// $group stage implementation
//
// STREAMING OPTIMIZATION (2024-12):
// Changed from storing full documents per group to streaming accumulator states.
// Memory usage: O(N * doc_size) → O(G * state_size) where G = number of groups
// This fixes OOM issues with large collections (e.g., 650K emails → 15 groups)
//
// MEMORY SAFETY (2026-01):
// Added max_group_count limit to prevent OOM with high-cardinality group keys.
// See execute_streaming_with_limits() for implementation.
//
// INDEX-BASED OPTIMIZATION (2026-01):
// When group key has a single-field index and all accumulators are $sum:1 (count),
// we can compute the result directly from the index without loading any documents.
// This reduces I/O from O(N * doc_size) to O(index_size), typically 100-1000x faster.

use crate::aggregation::types::{
    Accumulator, AccumulatorState, AggregationLimits, GroupId, GroupStage, SumExpression,
};
use crate::error::{IronBaseError, Result};
use crate::index::IndexManager;
use crate::value_utils::{get_nested_value, value_hash};
use serde_json::{json, Value};
use std::collections::HashMap;

/// Group entry: stores the original key value and accumulator states
/// Using u64 hash as HashMap key avoids expensive JSON serialization
struct GroupEntry {
    key_value: Value, // Original _id value for output
    states: HashMap<String, AccumulatorState>,
}

impl GroupStage {
    pub(crate) fn from_json(spec: &Value) -> Result<Self> {
        if let Value::Object(obj) = spec {
            let id = if let Some(id_value) = obj.get("_id") {
                if id_value.is_null() {
                    GroupId::Null
                } else if let Some(s) = id_value.as_str() {
                    if s.starts_with('$') {
                        GroupId::Field(s.to_string())
                    } else {
                        return Err(IronBaseError::AggregationError(
                            "Group _id field reference must start with $".to_string(),
                        ));
                    }
                } else {
                    return Err(IronBaseError::AggregationError(
                        "Group _id must be null or field reference".to_string(),
                    ));
                }
            } else {
                return Err(IronBaseError::AggregationError(
                    "Group stage must have _id field".to_string(),
                ));
            };

            let mut accumulators = HashMap::new();
            for (field, value) in obj {
                if field == "_id" {
                    continue;
                }

                let accumulator = Accumulator::from_json(value)?;
                accumulators.insert(field.clone(), accumulator);
            }

            Ok(GroupStage { id, accumulators })
        } else {
            Err(IronBaseError::AggregationError(
                "$group must be an object".to_string(),
            ))
        }
    }

    /// Execute $group stage with streaming accumulators (memory optimized)
    ///
    /// OLD APPROACH (BAD - OOM prone):
    /// ```ignore
    /// groups: HashMap<String, Vec<Value>>  // Stored ALL documents per group
    /// ```
    ///
    /// NEW APPROACH (GOOD - constant memory per group):
    /// ```ignore
    /// groups: HashMap<u64, GroupEntry>  // Hash-based lookup, stores original key
    /// ```
    ///
    /// PERF OPTIMIZATION (2024-12):
    /// Changed from String keys (serde_json::to_string) to u64 hash keys (value_hash).
    /// This eliminates JSON serialization overhead: ~30µs/doc → ~0.3µs/doc
    ///
    /// Memory comparison for 650K emails grouped by sender (~10K unique senders):
    /// - OLD: 650K × ~800 bytes = ~500MB
    /// - NEW: 10K × ~64 bytes = ~640KB (780× reduction!)
    pub(crate) fn execute(&self, docs: Vec<Value>) -> Result<Vec<Value>> {
        // HashMap<hash, GroupEntry> - hash-based lookup avoids JSON serialization
        let mut groups: HashMap<u64, GroupEntry> = HashMap::new();

        for doc in docs {
            let (group_hash, group_value) = self.extract_group_key_hash(&doc)?;

            // Get or initialize accumulator states for this group
            let entry = groups.entry(group_hash).or_insert_with(|| GroupEntry {
                key_value: group_value,
                states: self
                    .accumulators
                    .iter()
                    .map(|(field, acc)| (field.clone(), acc.init_state()))
                    .collect(),
            });

            // Update each accumulator state with this document (streaming)
            // Note: execute_streaming() has no limits - use execute_streaming_with_limits() for OOM protection
            for (field, accumulator) in &self.accumulators {
                if let Some(state) = entry.states.get_mut(field) {
                    state.update(&doc, accumulator);
                }
            }
            // doc is DROPPED here - not stored in memory!
        }

        // Finalize: convert accumulated states to output values
        let mut results = Vec::new();

        for (_hash, mut entry) in groups {
            let mut result = serde_json::Map::new();

            result.insert("_id".to_string(), entry.key_value);

            for field in self.accumulators.keys() {
                if let Some(state) = entry.states.remove(field) {
                    let value = state.finalize();
                    result.insert(field.clone(), value);
                }
            }

            results.push(Value::Object(result));
        }

        Ok(results)
    }

    /// Extract group key as (hash, value) pair - avoids JSON serialization
    /// Returns (u64 hash for HashMap lookup, original Value for output _id)
    fn extract_group_key_hash(&self, doc: &Value) -> Result<(u64, Value)> {
        match &self.id {
            GroupId::Null => {
                // All documents go to same group
                Ok((0, Value::Null))
            }
            GroupId::Field(field) => {
                let field_name = field.trim_start_matches('$');
                if let Some(value) = get_nested_value(doc, field_name) {
                    // PERF: Use value_hash instead of serde_json::to_string (10-50x faster)
                    let hash = value_hash(value);
                    Ok((hash, value.clone()))
                } else {
                    // null value - use hash of null
                    Ok((value_hash(&Value::Null), Value::Null))
                }
            }
        }
    }

    /// Execute $group stage with streaming document input (iterator-based)
    ///
    /// This is the most memory-efficient approach for large collections:
    /// - Documents are processed one at a time from the iterator
    /// - Only accumulator states are kept in memory (not full documents)
    /// - Memory usage: O(G × state_size) where G = number of groups
    ///
    /// PERF OPTIMIZATION (2024-12):
    /// Uses hash-based group keys (value_hash) instead of JSON serialization.
    ///
    /// # Example memory savings for 650K emails grouped by sender (10K groups):
    /// - Vec<Value> input: 650K × 800 bytes = ~500MB loaded upfront
    /// - Iterator input: ~0MB (documents processed and discarded)
    /// - Group states: 10K × 64 bytes = ~640KB
    /// - **Total: 640KB instead of 500MB+**
    #[allow(dead_code)]
    pub(crate) fn execute_streaming<I>(&self, docs: I) -> Result<Vec<Value>>
    where
        I: Iterator<Item = Result<Value>>,
    {
        self.execute_streaming_with_limits(docs, AggregationLimits::default())
    }

    /// Execute $group with explicit memory limits
    ///
    /// Adds group count checking to prevent OOM with high-cardinality group keys.
    ///
    /// # Arguments
    /// * `docs` - Iterator of documents to process
    /// * `limits` - Memory safety limits including max_group_count
    ///
    /// # Errors
    /// Returns error if group count exceeds `limits.max_group_count`
    ///
    /// # Example
    /// ```rust,ignore
    /// let limits = AggregationLimits {
    ///     max_group_count: 10_000,
    ///     ..Default::default()
    /// };
    /// let results = group_stage.execute_streaming_with_limits(docs, limits)?;
    /// ```
    pub(crate) fn execute_streaming_with_limits<I>(
        &self,
        docs: I,
        limits: AggregationLimits,
    ) -> Result<Vec<Value>>
    where
        I: Iterator<Item = Result<Value>>,
    {
        // HashMap<hash, GroupEntry> - hash-based lookup avoids JSON serialization
        let mut groups: HashMap<u64, GroupEntry> = HashMap::new();
        let mut doc_count: usize = 0;
        let mut last_group_check: usize = 0;

        for doc_result in docs {
            let doc = doc_result?;
            doc_count += 1;

            let (group_hash, group_value) = self.extract_group_key_hash(&doc)?;

            let is_new_group = !groups.contains_key(&group_hash);

            let entry = groups.entry(group_hash).or_insert_with(|| GroupEntry {
                key_value: group_value,
                states: self
                    .accumulators
                    .iter()
                    .map(|(field, acc)| (field.clone(), acc.init_state()))
                    .collect(),
            });

            // Update accumulators with OOM-safe limits for $push/$addToSet
            for (field, accumulator) in &self.accumulators {
                if let Some(state) = entry.states.get_mut(field) {
                    state.update_with_limits(
                        &doc,
                        accumulator,
                        limits.max_push_elements,
                        limits.max_addtoset_elements,
                    )?;
                }
            }

            // MEMORY SAFETY: Check group count periodically
            // Only check when a new group is added and we've processed at least 1000 more docs
            if is_new_group && doc_count - last_group_check >= 1000 {
                last_group_check = doc_count;

                if groups.len() > limits.max_group_count {
                    return Err(IronBaseError::AggregationError(format!(
                        "Aggregation exceeded group limit: {} unique groups after {} documents. \
                         High-cardinality $group key detected. Consider:\n\
                         1. Add a $match stage to filter documents first\n\
                         2. Use a lower-cardinality group key\n\
                         3. Increase max_group_count limit (current: {})\n\
                         Group key: {:?}",
                        groups.len(),
                        doc_count,
                        limits.max_group_count,
                        self.id
                    )));
                }
            }

            // doc is dropped here - NOT kept in memory!
        }

        // Final group count check
        if groups.len() > limits.max_group_count {
            return Err(IronBaseError::AggregationError(format!(
                "Aggregation exceeded group limit: {} unique groups (limit: {}). \
                 Add a $match stage to filter documents or use a lower-cardinality group key.",
                groups.len(),
                limits.max_group_count
            )));
        }

        // Finalize
        let mut results = Vec::new();
        for (_hash, mut entry) in groups {
            let mut result = serde_json::Map::new();
            result.insert("_id".to_string(), entry.key_value);

            for field in self.accumulators.keys() {
                if let Some(state) = entry.states.remove(field) {
                    let value = state.finalize();
                    result.insert(field.clone(), value);
                }
            }
            results.push(Value::Object(result));
        }

        Ok(results)
    }

    // ========== INDEX-BASED OPTIMIZATION ==========

    /// Check if this $group can be optimized using index
    ///
    /// Returns the group field name (without $) if:
    /// - Group key is a single field: {"_id": "$field"}
    /// - All accumulators are $sum with constant (counting), not field references
    ///
    /// Returns None if the group cannot be index-optimized.
    pub(crate) fn can_use_index(&self) -> Option<&str> {
        // Check 1: Group key must be a single field
        let field = match &self.id {
            GroupId::Field(f) => f.trim_start_matches('$'),
            GroupId::Null => return None, // Null means all docs in one group - no index benefit
        };

        // Check 2: All accumulators must be count-only ($sum: 1 or $sum: <constant>)
        // We don't support $sum: "$field" because that requires reading document data
        for acc in self.accumulators.values() {
            match acc {
                Accumulator::Sum(SumExpression::Constant(_)) => {
                    // OK - counting, no document data needed
                }
                _ => {
                    // Any other accumulator needs document data
                    return None;
                }
            }
        }

        Some(field)
    }

    /// Try to execute $group using index instead of document scan
    ///
    /// This optimization works when:
    /// 1. Group key is a single field with a single-field index
    /// 2. All accumulators are $sum with constant (e.g., $sum: 1 for counting)
    ///
    /// Performance improvement:
    /// - Before: Load all 78K documents (39GB I/O), extract field, group
    /// - After: Scan index entries only (~50MB), count per key
    /// - Speedup: 100-1000x depending on document size
    ///
    /// Returns None if index optimization is not possible or would exceed limits (falls back to streaming).
    pub(crate) fn try_index_based_execute(
        &self,
        indexes: &IndexManager,
        limits: AggregationLimits,
    ) -> Option<Vec<Value>> {
        // Check if this group can use index
        let field = self.can_use_index()?;

        // Find a single-field index on this field
        let index_infos = indexes.list_indexes_with_compound_info();
        let matching_index = index_infos
            .iter()
            .find(|info| !info.is_compound && info.prefix_field == field)?;

        // Get the B+ tree index
        let btree = indexes.get_btree_index(&matching_index.index_name)?;

        // Count entries per key directly from the index
        // This avoids loading any documents!
        // OOM FIX (2026-01): Apply max_group_count limit
        let mut counts: HashMap<u64, (Value, i64)> = HashMap::new();

        for (key, _doc_id) in btree.get_all_entries() {
            let key_value = key.to_value();
            let key_hash = value_hash(&key_value);

            // Check if this is a NEW group (not updating existing count)
            if !counts.contains_key(&key_hash) {
                // Check group count limit BEFORE inserting new group
                if counts.len() >= limits.max_group_count {
                    // Too many groups - fall back to streaming which has proper error handling
                    return None;
                }
                counts.insert(key_hash, (key_value, 1));
            } else {
                // Update existing group's count
                counts.get_mut(&key_hash).unwrap().1 += 1;
            }
        }

        // Build result documents
        // Each accumulator that was $sum: N gets value N * count
        let mut results = Vec::with_capacity(counts.len());

        for (_hash, (key_value, count)) in counts {
            let mut result = serde_json::Map::new();
            result.insert("_id".to_string(), key_value);

            // For each accumulator, compute final value
            for (acc_name, acc) in &self.accumulators {
                let value = match acc {
                    Accumulator::Sum(SumExpression::Constant(n)) => {
                        // $sum: N means multiply by count
                        json!(n * count)
                    }
                    _ => {
                        // Shouldn't happen if can_use_index() returned Some
                        json!(null)
                    }
                };
                result.insert(acc_name.clone(), value);
            }

            results.push(Value::Object(result));
        }

        Some(results)
    }
}
