// src/aggregation/context.rs
// Centralized limit manager for aggregation pipelines
//
// DESIGN (2026-01):
// Uses Rc<RefCell> pattern for shared mutable state across iterator chain.
// Clone creates a new reference to the same inner state, allowing multiple
// iterators to share counters (e.g., MatchIterator and ProjectIterator).

use crate::aggregation::types::AggregationLimits;
use crate::error::{IronBaseError, Result};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Instant;

/// Centralized limit tracker for aggregation pipelines
///
/// Tracks document count, group count, per-group $push/$addToSet counts,
/// and $unwind output counts. All limits are checked incrementally during
/// pipeline execution to fail fast on OOM-prone operations.
///
/// # Example
/// ```rust,ignore
/// let limits = AggregationLimits::default();
/// let ctx = AggregationLimitContext::new(limits);
///
/// // Track documents
/// ctx.increment_docs()?;
///
/// // Track groups
/// ctx.register_new_group(group_hash)?;
///
/// // Track per-group $push
/// ctx.increment_push(group_hash)?;
/// ```
#[allow(dead_code)] // Phase 2: will be used in pipeline.rs
#[derive(Clone)]
pub struct AggregationLimitContext {
    inner: Rc<RefCell<ContextInner>>,
}

#[allow(dead_code)] // Phase 2: fields will be used in pipeline.rs
struct ContextInner {
    limits: AggregationLimits,

    // Document counter
    docs_processed: usize,

    // Index entry counter (for index-based $group paths)
    index_entries_scanned: usize,

    // Group counter
    groups_created: usize,

    // $unwind output counter
    unwind_outputs: usize,

    // Per-group counters: HashMap<group_hash, (push_count, addtoset_count)>
    group_counters: HashMap<u64, (usize, usize)>,

    // Flag: pipeline has leading $match (allows higher doc limit)
    has_leading_match: bool,

    // Early termination limit (for $limit stages)
    early_limit: Option<usize>,

    // Flag: streaming to $group (disables doc limit - docs not collected)
    streaming_to_group: bool,

    // Optional deadline for cooperative cancellation
    deadline: Option<Instant>,
}

#[allow(dead_code)] // Phase 2: methods will be used in pipeline.rs
impl AggregationLimitContext {
    /// Create new context with specified limits
    pub fn new(limits: AggregationLimits) -> Self {
        Self {
            inner: Rc::new(RefCell::new(ContextInner {
                limits,
                docs_processed: 0,
                index_entries_scanned: 0,
                groups_created: 0,
                unwind_outputs: 0,
                group_counters: HashMap::new(),
                has_leading_match: false,
                early_limit: None,
                streaming_to_group: false,
                deadline: None,
            })),
        }
    }

    /// Set whether pipeline has a leading $match stage
    ///
    /// When true, uses max_docs_with_match instead of max_docs_without_match.
    /// This allows higher limits for filtered aggregations.
    pub fn with_leading_match(self, has_match: bool) -> Self {
        self.inner.borrow_mut().has_leading_match = has_match;
        self
    }

    /// Set has_leading_match flag (mutable version)
    pub fn set_leading_match(&self, has_match: bool) {
        self.inner.borrow_mut().has_leading_match = has_match;
    }

    /// Set a deadline for cooperative cancellation
    pub fn with_deadline(self, deadline: Instant) -> Self {
        self.inner.borrow_mut().deadline = Some(deadline);
        self
    }

    /// Set or clear deadline (mutable version)
    pub fn set_deadline(&self, deadline: Option<Instant>) {
        self.inner.borrow_mut().deadline = deadline;
    }

    /// Check if the deadline has been exceeded
    fn check_deadline(&self) -> Result<()> {
        if let Some(deadline) = self.inner.borrow().deadline {
            if Instant::now() >= deadline {
                return Err(IronBaseError::AggregationError(
                    "Aggregation timed out (deadline exceeded).".to_string(),
                ));
            }
        }
        Ok(())
    }

    /// Enter streaming-to-group mode (doc limit temporarily disabled)
    pub fn enter_streaming_group(&self) {
        self.inner.borrow_mut().streaming_to_group = true;
    }

    /// Exit streaming-to-group mode and set processed doc count
    pub fn exit_streaming_group(&self, output_docs: usize) {
        let mut inner = self.inner.borrow_mut();
        inner.streaming_to_group = false;
        inner.docs_processed = output_docs;
        inner.index_entries_scanned = 0;
    }

    // ========== Document counting ==========

    /// Check if document count is within limit
    ///
    /// IMPORTANT: Returns Ok immediately if streaming_to_group is true,
    /// because docs are not being collected into memory.
    pub fn check_doc_limit(&self) -> Result<()> {
        self.check_deadline()?;
        let inner = self.inner.borrow();

        // Skip limit check in streaming-to-group mode
        // Docs are processed one at a time and discarded, not collected
        if inner.streaming_to_group {
            return Ok(());
        }

        let limit = if inner.has_leading_match {
            inner.limits.max_docs_with_match
        } else {
            inner.limits.max_docs_without_match
        };

        if inner.docs_processed > limit {
            let mode = if inner.has_leading_match {
                "with $match"
            } else {
                "without $match"
            };
            return Err(IronBaseError::AggregationError(format!(
                "Aggregation exceeded document limit: {} documents processed {} (limit: {}). \
                 Consider adding a more selective $match stage or reducing the collection size.",
                inner.docs_processed, mode, limit
            )));
        }
        Ok(())
    }

    /// Increment document counter and check limit
    pub fn increment_docs(&self) -> Result<()> {
        {
            let mut inner = self.inner.borrow_mut();
            inner.docs_processed += 1;
        }
        self.check_doc_limit()
    }

    /// Get current document count
    pub fn docs_processed(&self) -> usize {
        self.inner.borrow().docs_processed
    }

    // ========== Group counting ==========

    /// Check if group count is within limit
    pub fn check_group_limit(&self) -> Result<()> {
        self.check_deadline()?;
        let inner = self.inner.borrow();
        if inner.groups_created > inner.limits.max_group_count {
            return Err(IronBaseError::AggregationError(format!(
                "Aggregation exceeded group limit: {} unique groups (limit: {}). \
                 High-cardinality $group key detected. Consider:\n\
                 1. Add a $match stage to filter documents first\n\
                 2. Use a lower-cardinality group key\n\
                 3. Increase max_group_count limit",
                inner.groups_created, inner.limits.max_group_count
            )));
        }
        Ok(())
    }

    /// Register a new group and check limit
    ///
    /// Returns Ok if group was already registered or limit not exceeded.
    /// Returns Err if new group would exceed limit.
    pub fn register_new_group(&self, group_hash: u64) -> Result<()> {
        self.check_deadline()?;
        let mut inner = self.inner.borrow_mut();

        // Check if group already exists
        if inner.group_counters.contains_key(&group_hash) {
            return Ok(());
        }

        // Check limit BEFORE adding new group
        if inner.groups_created >= inner.limits.max_group_count {
            return Err(IronBaseError::AggregationError(format!(
                "Aggregation exceeded group limit: {} unique groups (limit: {}). \
                 High-cardinality $group key detected. Consider:\n\
                 1. Add a $match stage to filter documents first\n\
                 2. Use a lower-cardinality group key\n\
                 3. Increase max_group_count limit",
                inner.groups_created + 1,
                inner.limits.max_group_count
            )));
        }

        // Register new group with zero counters
        inner.groups_created += 1;
        inner.group_counters.insert(group_hash, (0, 0));
        Ok(())
    }

    /// Get current group count
    pub fn groups_created(&self) -> usize {
        self.inner.borrow().groups_created
    }

    // ========== Per-group $push counting ==========

    /// Check if $push count for a group is within limit
    pub fn check_push_limit(&self, group_hash: u64) -> Result<()> {
        self.check_deadline()?;
        let inner = self.inner.borrow();
        if let Some((push_count, _)) = inner.group_counters.get(&group_hash) {
            if *push_count >= inner.limits.max_push_elements {
                return Err(IronBaseError::AggregationError(format!(
                    "$push accumulator exceeded element limit for group: {} elements (limit: {}). \
                     Consider using $slice after $push or limiting input documents.",
                    push_count, inner.limits.max_push_elements
                )));
            }
        }
        Ok(())
    }

    /// Increment $push counter for a group
    pub fn increment_push(&self, group_hash: u64) -> Result<()> {
        self.check_deadline()?;
        let mut inner = self.inner.borrow_mut();
        let limit = inner.limits.max_push_elements;

        if let Some((push_count, _)) = inner.group_counters.get_mut(&group_hash) {
            *push_count += 1;
            if *push_count > limit {
                let count = *push_count;
                return Err(IronBaseError::AggregationError(format!(
                    "$push accumulator exceeded element limit for group: {} elements (limit: {}). \
                     Consider using $slice after $push or limiting input documents.",
                    count, limit
                )));
            }
        }
        Ok(())
    }

    /// Get current $push count for a group
    pub fn push_count(&self, group_hash: u64) -> usize {
        self.inner
            .borrow()
            .group_counters
            .get(&group_hash)
            .map(|(p, _)| *p)
            .unwrap_or(0)
    }

    // ========== Per-group $addToSet counting ==========

    /// Check if $addToSet count for a group is within limit
    pub fn check_addtoset_limit(&self, group_hash: u64) -> Result<()> {
        self.check_deadline()?;
        let inner = self.inner.borrow();
        if let Some((_, addtoset_count)) = inner.group_counters.get(&group_hash) {
            if *addtoset_count >= inner.limits.max_addtoset_elements {
                return Err(IronBaseError::AggregationError(format!(
                    "$addToSet accumulator exceeded element limit for group: {} elements (limit: {}). \
                     Consider limiting input documents or using a different approach.",
                    addtoset_count, inner.limits.max_addtoset_elements
                )));
            }
        }
        Ok(())
    }

    /// Increment $addToSet counter for a group
    pub fn increment_addtoset(&self, group_hash: u64) -> Result<()> {
        self.check_deadline()?;
        let mut inner = self.inner.borrow_mut();
        let limit = inner.limits.max_addtoset_elements;

        if let Some((_, addtoset_count)) = inner.group_counters.get_mut(&group_hash) {
            *addtoset_count += 1;
            if *addtoset_count > limit {
                let count = *addtoset_count;
                return Err(IronBaseError::AggregationError(format!(
                    "$addToSet accumulator exceeded element limit for group: {} elements (limit: {}). \
                     Consider limiting input documents or using a different approach.",
                    count, limit
                )));
            }
        }
        Ok(())
    }

    /// Get current $addToSet count for a group
    pub fn addtoset_count(&self, group_hash: u64) -> usize {
        self.inner
            .borrow()
            .group_counters
            .get(&group_hash)
            .map(|(_, a)| *a)
            .unwrap_or(0)
    }

    // ========== $unwind counting ==========

    /// Check if $unwind output count is within limit
    pub fn check_unwind_limit(&self) -> Result<()> {
        self.check_deadline()?;
        let inner = self.inner.borrow();
        if inner.unwind_outputs > inner.limits.max_unwind_output {
            return Err(IronBaseError::AggregationError(format!(
                "$unwind stage exceeded output limit: {} documents (limit: {}). \
                 Consider adding a $match stage before $unwind or using $slice.",
                inner.unwind_outputs, inner.limits.max_unwind_output
            )));
        }
        Ok(())
    }

    /// Check if adding `count` unwind outputs would exceed limit
    ///
    /// Call this BEFORE allocating memory for the unwind results.
    /// This prevents OOM by failing before the allocation, not after.
    pub fn check_unwind_would_exceed(&self, count: usize) -> Result<()> {
        self.check_deadline()?;
        let inner = self.inner.borrow();
        let new_total = inner.unwind_outputs.saturating_add(count);
        if new_total > inner.limits.max_unwind_output {
            return Err(IronBaseError::AggregationError(format!(
                "$unwind would exceed output limit: {} + {} = {} (limit: {}). \
                 Consider adding a $match stage before $unwind or using $slice.",
                inner.unwind_outputs, count, new_total, inner.limits.max_unwind_output
            )));
        }
        Ok(())
    }

    /// Increment $unwind output counter
    pub fn increment_unwind(&self, count: usize) -> Result<()> {
        self.check_deadline()?;
        {
            let mut inner = self.inner.borrow_mut();
            inner.unwind_outputs += count;
        }
        self.check_unwind_limit()
    }

    /// Get current $unwind output count
    pub fn unwind_outputs(&self) -> usize {
        self.inner.borrow().unwind_outputs
    }

    // ========== Index entry counting (for index-based paths) ==========

    /// Increment index entry counter and check limit
    ///
    /// Used by index-based $group optimization. The limit is the same
    /// as document limit since each index entry corresponds to a document.
    pub fn increment_index_entries(&self, count: usize) -> Result<()> {
        self.check_deadline()?;
        let mut inner = self.inner.borrow_mut();
        inner.index_entries_scanned += count;

        if inner.streaming_to_group {
            return Ok(());
        }

        // Use same logic as doc limit
        let limit = if inner.has_leading_match {
            inner.limits.max_docs_with_match
        } else {
            inner.limits.max_docs_without_match
        };

        if inner.index_entries_scanned > limit {
            return Err(IronBaseError::AggregationError(format!(
                "Index-based aggregation exceeded entry limit: {} entries (limit: {}). \
                 Add a more selective $match or reduce index size.",
                inner.index_entries_scanned, limit
            )));
        }
        Ok(())
    }

    /// Get current index entry count
    pub fn index_entries_scanned(&self) -> usize {
        self.inner.borrow().index_entries_scanned
    }

    // ========== Early termination for $limit ==========

    /// Set early termination limit
    ///
    /// When set, documents beyond this limit can be skipped.
    /// The limit is capped at the current doc_limit to prevent bypass.
    pub fn set_early_limit(&self, limit: usize) {
        let mut inner = self.inner.borrow_mut();
        let doc_limit = if inner.has_leading_match {
            inner.limits.max_docs_with_match
        } else {
            inner.limits.max_docs_without_match
        };
        // Cap at doc_limit to prevent $limit: 1_000_000 bypass
        inner.early_limit = Some(limit.min(doc_limit));
    }

    /// Get effective limit considering both early_limit and doc_limit
    pub fn effective_limit(&self) -> usize {
        let inner = self.inner.borrow();
        let doc_limit = if inner.has_leading_match {
            inner.limits.max_docs_with_match
        } else {
            inner.limits.max_docs_without_match
        };
        inner
            .early_limit
            .map(|l| l.min(doc_limit))
            .unwrap_or(doc_limit)
    }

    /// Check if we've reached the early termination limit
    pub fn should_terminate_early(&self) -> bool {
        let inner = self.inner.borrow();
        if let Some(limit) = inner.early_limit {
            inner.docs_processed >= limit
        } else {
            false
        }
    }

    // ========== Accessors ==========

    /// Get the configured limits
    pub fn limits(&self) -> AggregationLimits {
        self.inner.borrow().limits
    }

    /// Check if pipeline has leading $match
    pub fn has_leading_match(&self) -> bool {
        self.inner.borrow().has_leading_match
    }

    /// Reset all counters (for reuse)
    pub fn reset(&self) {
        let mut inner = self.inner.borrow_mut();
        inner.docs_processed = 0;
        inner.index_entries_scanned = 0;
        inner.groups_created = 0;
        inner.unwind_outputs = 0;
        inner.group_counters.clear();
        inner.early_limit = None;
    }
}

impl Default for AggregationLimitContext {
    fn default() -> Self {
        Self::new(AggregationLimits::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_creation() {
        let ctx = AggregationLimitContext::default();
        assert_eq!(ctx.docs_processed(), 0);
        assert_eq!(ctx.groups_created(), 0);
        assert!(!ctx.has_leading_match());
    }

    #[test]
    fn test_context_clone_shares_state() {
        let ctx1 = AggregationLimitContext::default();
        let ctx2 = ctx1.clone();

        ctx1.increment_docs().unwrap();
        assert_eq!(ctx2.docs_processed(), 1); // Same state!
    }

    #[test]
    fn test_doc_limit_without_match() {
        let limits = AggregationLimits {
            max_docs_without_match: 5,
            ..Default::default()
        };
        let ctx = AggregationLimitContext::new(limits);

        for _ in 0..5 {
            ctx.increment_docs().unwrap();
        }
        assert!(ctx.increment_docs().is_err()); // 6th fails
    }

    #[test]
    fn test_doc_limit_with_match() {
        let limits = AggregationLimits {
            max_docs_without_match: 5,
            max_docs_with_match: 100,
            ..Default::default()
        };
        let ctx = AggregationLimitContext::new(limits).with_leading_match(true);

        for _ in 0..100 {
            ctx.increment_docs().unwrap();
        }
        assert!(ctx.increment_docs().is_err()); // 101st fails
    }

    #[test]
    fn test_group_limit() {
        let limits = AggregationLimits {
            max_group_count: 3,
            ..Default::default()
        };
        let ctx = AggregationLimitContext::new(limits);

        ctx.register_new_group(1).unwrap();
        ctx.register_new_group(2).unwrap();
        ctx.register_new_group(3).unwrap();
        assert!(ctx.register_new_group(4).is_err()); // 4th fails
    }

    #[test]
    fn test_group_reregistration_no_error() {
        let limits = AggregationLimits {
            max_group_count: 2,
            ..Default::default()
        };
        let ctx = AggregationLimitContext::new(limits);

        ctx.register_new_group(1).unwrap();
        ctx.register_new_group(2).unwrap();
        // Re-registering existing groups should not error
        ctx.register_new_group(1).unwrap();
        ctx.register_new_group(2).unwrap();
        assert_eq!(ctx.groups_created(), 2);
    }

    #[test]
    fn test_push_limit_per_group() {
        let limits = AggregationLimits {
            max_push_elements: 2,
            max_group_count: 10,
            ..Default::default()
        };
        let ctx = AggregationLimitContext::new(limits);

        ctx.register_new_group(1).unwrap();
        ctx.register_new_group(2).unwrap();

        // Group 1: 2 pushes OK
        ctx.increment_push(1).unwrap();
        ctx.increment_push(1).unwrap();
        assert!(ctx.increment_push(1).is_err()); // 3rd fails

        // Group 2: Still has 2 slots
        ctx.increment_push(2).unwrap();
        ctx.increment_push(2).unwrap();
        assert!(ctx.increment_push(2).is_err()); // 3rd fails
    }

    #[test]
    fn test_addtoset_limit_per_group() {
        let limits = AggregationLimits {
            max_addtoset_elements: 3,
            max_group_count: 10,
            ..Default::default()
        };
        let ctx = AggregationLimitContext::new(limits);

        ctx.register_new_group(1).unwrap();

        ctx.increment_addtoset(1).unwrap();
        ctx.increment_addtoset(1).unwrap();
        ctx.increment_addtoset(1).unwrap();
        assert!(ctx.increment_addtoset(1).is_err()); // 4th fails
    }

    #[test]
    fn test_unwind_limit() {
        let limits = AggregationLimits {
            max_unwind_output: 10,
            ..Default::default()
        };
        let ctx = AggregationLimitContext::new(limits);

        ctx.increment_unwind(5).unwrap();
        ctx.increment_unwind(5).unwrap();
        assert!(ctx.increment_unwind(1).is_err()); // 11th fails
    }

    #[test]
    fn test_reset() {
        let ctx = AggregationLimitContext::default();

        ctx.increment_docs().unwrap();
        ctx.register_new_group(1).unwrap();
        ctx.increment_unwind(5).unwrap();

        assert_eq!(ctx.docs_processed(), 1);
        assert_eq!(ctx.groups_created(), 1);
        assert_eq!(ctx.unwind_outputs(), 5);

        ctx.reset();

        assert_eq!(ctx.docs_processed(), 0);
        assert_eq!(ctx.groups_created(), 0);
        assert_eq!(ctx.unwind_outputs(), 0);
    }
}
