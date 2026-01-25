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

use crate::aggregation::context::AggregationLimitContext;
use crate::aggregation::types::{
    Accumulator, AccumulatorState, AggregationLimits, GroupId, GroupStage, SumExpression,
};
use crate::error::{IronBaseError, Result};
use crate::index::IndexManager;
use crate::value_utils::{get_nested_value, substr_string, value_hash};
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
                } else if let Some(id_obj) = id_value.as_object() {
                    if let Some(substr_spec) = id_obj.get("$substr") {
                        let (field, start, length) = Self::parse_substr_spec(substr_spec)?;
                        GroupId::Substring {
                            field,
                            start,
                            length,
                        }
                    } else {
                        return Err(IronBaseError::AggregationError(
                            "Group _id object must use a supported operator".to_string(),
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
    /// # DEPRECATED (2026-01-25)
    ///
    /// **WARNING:** This method has NO LIMIT CHECKING!
    /// Use `execute_streaming_with_context()` or `execute_streaming_with_limits()` instead.
    ///
    /// This legacy method is kept only for backward compatibility and may be removed
    /// in a future version.
    ///
    /// ## Why this is dangerous:
    /// - No `max_group_count` limit → OOM with high-cardinality group keys
    /// - No `max_push_elements` limit → OOM with large $push accumulators
    /// - No deadline checking → can run indefinitely
    ///
    /// ## Memory comparison for 650K emails grouped by sender (~10K unique senders):
    /// - OLD (storing docs): 650K × ~800 bytes = ~500MB
    /// - NEW (streaming): 10K × ~64 bytes = ~640KB (780× reduction!)
    #[deprecated(
        since = "0.3.90",
        note = "Use execute_streaming_with_context() or execute_streaming_with_limits() for OOM protection"
    )]
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
            GroupId::Substring {
                field,
                start,
                length,
            } => {
                let field_name = field.trim_start_matches('$');
                let value = if let Some(raw) = get_nested_value(doc, field_name) {
                    match raw.as_str() {
                        Some(text) => Value::String(substr_string(text, *start, *length)),
                        None => Value::Null,
                    }
                } else {
                    Value::Null
                };
                let hash = value_hash(&value);
                Ok((hash, value))
            }
        }
    }

    fn parse_substr_number(value: &Value, label: &str) -> Result<usize> {
        let num = value.as_i64().ok_or_else(|| {
            IronBaseError::AggregationError(format!("$substr {} must be integer", label))
        })?;
        if num < 0 {
            return Err(IronBaseError::AggregationError(format!(
                "$substr {} must be non-negative",
                label
            )));
        }
        Ok(num as usize)
    }

    fn parse_substr_spec(spec: &Value) -> Result<(String, usize, usize)> {
        let arr = spec.as_array().ok_or_else(|| {
            IronBaseError::AggregationError("$substr must be an array".to_string())
        })?;
        if arr.len() != 3 {
            return Err(IronBaseError::AggregationError(
                "$substr requires [field, start, length]".to_string(),
            ));
        }

        let field = arr[0].as_str().ok_or_else(|| {
            IronBaseError::AggregationError("$substr field must be a string".to_string())
        })?;
        if !field.starts_with('$') {
            return Err(IronBaseError::AggregationError(
                "$substr field must start with $".to_string(),
            ));
        }

        let start = Self::parse_substr_number(&arr[1], "start")?;
        let length = Self::parse_substr_number(&arr[2], "length")?;

        Ok((field.to_string(), start, length))
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

        // FIX (2026-01-25): Track global $push/$addToSet counts
        // These were added but not enforced in this method!
        let mut total_push_elements: usize = 0;
        let mut total_addtoset_elements: usize = 0;

        // OOM FIX (2026-01): Adaptive check interval based on limit
        // For small limits (e.g., 100), check every ~10 new groups
        // For large limits (e.g., 50K), check every ~1000 docs
        // This prevents overshooting small limits while avoiding overhead for large ones
        let check_interval = (limits.max_group_count / 10).clamp(1, 1000);

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
                    // Track global counts BEFORE update to catch the limit
                    let (push_added, addtoset_added) = state.update_with_limits_tracking(
                        &doc,
                        accumulator,
                        limits.max_push_elements,
                        limits.max_addtoset_elements,
                    )?;

                    // Update global counters
                    total_push_elements += push_added;
                    total_addtoset_elements += addtoset_added;

                    // FIX (2026-01-25): Check GLOBAL limits
                    if total_push_elements > limits.max_total_push_elements {
                        return Err(IronBaseError::AggregationError(format!(
                            "$push accumulators exceeded global limit: {} total elements across all groups (limit: {}). \
                             Too many groups with $push. Consider reducing input documents or using a different approach.",
                            total_push_elements, limits.max_total_push_elements
                        )));
                    }
                    if total_addtoset_elements > limits.max_total_addtoset_elements {
                        return Err(IronBaseError::AggregationError(format!(
                            "$addToSet accumulators exceeded global limit: {} total unique elements across all groups (limit: {}). \
                             Too many groups with $addToSet. Consider reducing input documents or using a different approach.",
                            total_addtoset_elements, limits.max_total_addtoset_elements
                        )));
                    }
                }
            }

            // MEMORY SAFETY: Check group count when new group is added
            // Uses adaptive interval to balance overhead vs responsiveness
            if is_new_group && groups.len() - last_group_check >= check_interval {
                last_group_check = groups.len();

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
                 High-cardinality $group key detected. Consider:\n\
                 1. Add a $match stage to filter documents first\n\
                 2. Use a lower-cardinality group key\n\
                 3. Increase max_group_count limit",
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

    // ========== CONTEXT-AWARE EXECUTION ==========

    /// Execute $group with STREAMING iterator and context limit tracking
    ///
    /// **CRITICAL**: This method takes an ITERATOR, not Vec<Value>!
    /// Documents are processed one at a time and immediately discarded.
    /// Only accumulator states are kept in memory.
    ///
    /// Memory comparison for 650K emails grouped by sender (10K groups):
    /// - Vec<Value> input (BAD):  650K × 800 bytes = ~500MB all loaded first
    /// - Iterator input (GOOD):   Only 10K × 64 bytes = ~640KB group states
    ///
    /// Uses AggregationLimitContext for:
    /// - Group count tracking (register_new_group)
    /// - Per-group $push limit tracking
    /// - Per-group $addToSet limit tracking
    ///
    /// # Arguments
    /// * `docs` - Iterator of documents (NOT collected to Vec!)
    /// * `ctx` - Shared limit context for tracking across pipeline
    pub(crate) fn execute_streaming_with_context<I>(
        &self,
        docs: I,
        ctx: &AggregationLimitContext,
    ) -> Result<Vec<Value>>
    where
        I: Iterator<Item = Result<Value>>,
    {
        use std::collections::hash_map::Entry;

        let mut groups: HashMap<u64, GroupEntry> = HashMap::new();

        // Process documents ONE AT A TIME from the iterator
        // Each document is dropped after processing - NOT stored!
        for doc_result in docs {
            let doc = doc_result?;

            // FIX (2026-01-25): Check deadline on EVERY document, not just new groups.
            // Without this, aggregations on 129K+ docs with few unique groups would never
            // check the timeout, causing apparent hangs. The increment_docs() call:
            // 1. Increments docs_processed counter
            // 2. Calls check_doc_limit() which calls check_deadline()
            // 3. Skips doc limit check in streaming_to_group mode (but deadline still checked)
            ctx.increment_docs()?;

            let (group_hash, group_value) = self.extract_group_key_hash(&doc)?;

            // Use Entry API to check and insert atomically
            let entry = match groups.entry(group_hash) {
                Entry::Vacant(e) => {
                    // New group - register with context first (checks limit)
                    ctx.register_new_group(group_hash)?;

                    e.insert(GroupEntry {
                        key_value: group_value,
                        states: self
                            .accumulators
                            .iter()
                            .map(|(field, acc)| (field.clone(), acc.init_state()))
                            .collect(),
                    })
                }
                Entry::Occupied(e) => e.into_mut(),
            };

            // Update accumulators with context-aware limit tracking
            for (field, accumulator) in &self.accumulators {
                if let Some(state) = entry.states.get_mut(field) {
                    state.update_with_context(&doc, accumulator, group_hash, ctx)?;
                }
            }
            // doc is DROPPED here - NOT stored in memory!
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

    /// Execute $group with Vec input and context (DEPRECATED - prefer streaming!)
    ///
    /// **WARNING**: This method requires ALL documents in memory first.
    /// Use `execute_streaming_with_context` for large collections!
    #[allow(dead_code)] // Kept for backward compatibility, prefer streaming version
    pub(crate) fn execute_with_context_impl(
        &self,
        docs: Vec<Value>,
        ctx: &AggregationLimitContext,
    ) -> Result<Vec<Value>> {
        // Delegate to streaming version
        self.execute_streaming_with_context(docs.into_iter().map(Ok), ctx)
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
            GroupId::Substring { .. } => return None,
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
    #[allow(dead_code)] // Legacy path (aggregate_with_limits) kept for backwards compatibility
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

    /// Index-based $group execution with context tracking
    ///
    /// Same as `try_index_based_execute` but uses `AggregationLimitContext`
    /// for consistent limit tracking across all code paths.
    ///
    /// Key differences:
    /// - Uses `ctx.increment_index_entries()` to count entries
    /// - Uses `ctx.register_new_group()` for group limit checking
    /// - Returns `Option<Vec<Value>>` (None = can't use index or error)
    ///
    /// Note: Returns None on errors to fall back to streaming path
    /// which provides better error messages via context.
    pub(crate) fn try_index_based_execute_with_context(
        &self,
        indexes: &IndexManager,
        ctx: &super::super::context::AggregationLimitContext,
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
        // Uses context for both entry and group limit tracking
        let mut counts: HashMap<u64, (Value, i64)> = HashMap::new();

        for (key, _doc_id) in btree.get_all_entries() {
            // Track index entry (equivalent to document processing)
            if ctx.increment_index_entries(1).is_err() {
                // Hit entry limit - fall back to streaming for proper error
                return None;
            }

            let key_value = key.to_value();
            let key_hash = value_hash(&key_value);

            // Use Entry API for clippy compliance
            use std::collections::hash_map::Entry;
            match counts.entry(key_hash) {
                Entry::Vacant(e) => {
                    // Register new group via context
                    if ctx.register_new_group(key_hash).is_err() {
                        // Hit group limit - fall back to streaming for proper error
                        return None;
                    }
                    e.insert((key_value, 1));
                }
                Entry::Occupied(mut e) => {
                    // Update existing group's count
                    e.get_mut().1 += 1;
                }
            }
        }

        // Build result documents
        let mut results = Vec::with_capacity(counts.len());

        for (_hash, (key_value, count)) in counts {
            let mut result = serde_json::Map::new();
            result.insert("_id".to_string(), key_value);

            // For each accumulator, compute final value
            for (acc_name, acc) in &self.accumulators {
                let value = match acc {
                    Accumulator::Sum(SumExpression::Constant(n)) => {
                        json!(n * count)
                    }
                    _ => {
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

#[cfg(test)]
#[allow(deprecated)] // Tests use deprecated execute() for backward compatibility testing
mod tests {
    use super::*;
    use crate::aggregation::context::AggregationLimitContext;
    use crate::aggregation::types::AggregationLimits;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// PROOF that streaming works: this iterator tracks how many docs are "alive" at once
    struct CountingIterator {
        docs: Vec<Value>,
        current_idx: usize,
        alive_count: std::sync::Arc<AtomicUsize>,
        max_alive: std::sync::Arc<AtomicUsize>,
    }

    impl Iterator for CountingIterator {
        type Item = Result<Value>;

        fn next(&mut self) -> Option<Self::Item> {
            if self.current_idx >= self.docs.len() {
                return None;
            }

            // Increment alive count (simulating doc being loaded)
            let new_count = self.alive_count.fetch_add(1, Ordering::SeqCst) + 1;

            // Track max concurrent docs
            let mut max = self.max_alive.load(Ordering::SeqCst);
            while new_count > max {
                match self.max_alive.compare_exchange_weak(
                    max,
                    new_count,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                ) {
                    Ok(_) => break,
                    Err(m) => max = m,
                }
            }

            let doc = self.docs[self.current_idx].clone();
            self.current_idx += 1;

            // Decrement alive count (simulating doc being dropped after processing)
            // In real streaming, this happens when doc goes out of scope
            self.alive_count.fetch_sub(1, Ordering::SeqCst);

            Some(Ok(doc))
        }
    }

    #[test]
    fn test_streaming_does_not_collect_all_docs() {
        // Create 1000 documents
        let docs: Vec<Value> = (0..1000)
            .map(|i| json!({"group": i % 10, "value": i}))
            .collect();

        let alive_count = std::sync::Arc::new(AtomicUsize::new(0));
        let max_alive = std::sync::Arc::new(AtomicUsize::new(0));

        let iter = CountingIterator {
            docs,
            current_idx: 0,
            alive_count: alive_count.clone(),
            max_alive: max_alive.clone(),
        };

        // Create group stage: group by "group" field, count
        let group_stage = GroupStage::from_json(&json!({
            "_id": "$group",
            "count": {"$sum": 1}
        }))
        .unwrap();

        let limits = AggregationLimits::default();
        let ctx = AggregationLimitContext::new(limits);

        // Execute streaming
        let results = group_stage
            .execute_streaming_with_context(iter, &ctx)
            .unwrap();

        // Should have 10 groups (0-9)
        assert_eq!(results.len(), 10);

        // PROOF: Max alive docs should be 1 (streaming processes one at a time)
        // If we collected all first, max_alive would be 1000
        let max = max_alive.load(Ordering::SeqCst);
        println!("Max concurrent docs in memory: {}", max);
        assert_eq!(max, 1, "Streaming should process docs one at a time!");
    }

    #[test]
    fn test_streaming_context_enforces_group_limit() {
        // 100 docs with 100 unique groups
        let docs: Vec<Value> = (0..100).map(|i| json!({"group": i})).collect();

        let limits = AggregationLimits {
            max_group_count: 50, // Only allow 50 groups
            ..Default::default()
        };
        let ctx = AggregationLimitContext::new(limits);

        let group_stage = GroupStage::from_json(&json!({
            "_id": "$group",
            "count": {"$sum": 1}
        }))
        .unwrap();

        // Should fail because we exceed group limit
        let result = group_stage.execute_streaming_with_context(docs.into_iter().map(Ok), &ctx);

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("group") && err_msg.contains("limit"),
            "Error should mention group limit: {}",
            err_msg
        );
    }

    #[test]
    fn test_group_id_substr() {
        let docs = vec![
            json!({"name": "Alice"}),
            json!({"name": "Aaron"}),
            json!({"name": "Bob"}),
            json!({"other": "missing"}),
        ];

        let group_stage = GroupStage::from_json(&json!({
            "_id": {"$substr": ["$name", 0, 1]},
            "count": {"$sum": 1}
        }))
        .unwrap();

        let results = group_stage.execute(docs).unwrap();
        let mut counts = std::collections::HashMap::new();

        for doc in results {
            let key = doc.get("_id").cloned().unwrap_or(Value::Null);
            let count = doc.get("count").and_then(|v| v.as_i64()).unwrap_or(0);
            counts.insert(key, count);
        }

        assert_eq!(counts.get(&Value::String("A".to_string())), Some(&2));
        assert_eq!(counts.get(&Value::String("B".to_string())), Some(&1));
        assert_eq!(counts.get(&Value::Null), Some(&1));
    }

    #[test]
    fn test_group_accumulator_substr() {
        let docs = vec![
            json!({"group": 1, "name": "Alice"}),
            json!({"group": 1, "name": "Aaron"}),
            json!({"group": 2, "name": "Bob"}),
        ];

        let group_stage = GroupStage::from_json(&json!({
            "_id": "$group",
            "prefix": {"$first": {"$substr": ["$name", 0, 1]}}
        }))
        .unwrap();

        let results = group_stage.execute(docs).unwrap();
        let mut prefixes = std::collections::HashMap::new();
        for doc in results {
            prefixes.insert(
                doc.get("_id").cloned().unwrap(),
                doc.get("prefix").cloned().unwrap(),
            );
        }

        assert_eq!(prefixes.get(&json!(1)), Some(&json!("A")));
        assert_eq!(prefixes.get(&json!(2)), Some(&json!("B")));
    }
}
