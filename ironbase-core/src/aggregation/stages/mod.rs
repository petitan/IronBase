// src/aggregation/stages/mod.rs
// Individual stage implementations with context-aware execution

mod accumulator;
mod count_stage;
mod group_stage;
mod limit_stage;
mod match_stage;
mod project_stage;
mod skip_stage;
mod sort_stage;
mod unwind_stage;

use crate::aggregation::context::AggregationLimitContext;
use crate::aggregation::types::{
    CountStage, GroupStage, LimitStage, MatchStage, ProjectStage, SkipStage, SortStage, Stage,
    UnwindStage,
};
use crate::error::Result;
use serde_json::Value;

// ========== StageExecutor Trait ==========

/// Trait for context-aware stage execution
///
/// This trait provides a unified interface for executing aggregation stages
/// with centralized limit tracking via AggregationLimitContext.
///
/// # Design (2026-01)
/// The context-aware approach allows:
/// - Unified limit checking across all stages
/// - Per-group tracking for $push/$addToSet
/// - Early termination when limits are exceeded
/// - Consistent error messages with helpful suggestions
///
/// # Example
/// ```rust,ignore
/// let limits = AggregationLimits::from_system_memory();
/// let ctx = AggregationLimitContext::new(limits);
///
/// let result = stage.execute_with_context(docs, &ctx)?;
/// ```
#[allow(dead_code)] // Will be used in pipeline.rs integration
pub trait StageExecutor {
    /// Execute the stage with context-aware limit tracking
    ///
    /// # Arguments
    /// * `docs` - Input documents
    /// * `ctx` - Shared context for limit tracking
    ///
    /// # Returns
    /// Result containing output documents, or error if limits exceeded
    fn execute_with_context(
        &self,
        docs: Vec<Value>,
        ctx: &AggregationLimitContext,
    ) -> Result<Vec<Value>>;
}

// ========== Stage Enum Implementation ==========

#[allow(dead_code)] // Will be used in pipeline.rs integration
impl Stage {
    /// Execute stage with context-aware limits
    ///
    /// This is the main entry point for context-aware execution.
    /// Routes to the appropriate stage implementation.
    pub fn execute_with_context(
        &self,
        docs: Vec<Value>,
        ctx: &AggregationLimitContext,
    ) -> Result<Vec<Value>> {
        match self {
            Stage::Match(s) => s.execute_with_context(docs, ctx),
            Stage::Project(s) => s.execute_with_context(docs, ctx),
            Stage::Group(s) => s.execute_with_context(docs, ctx),
            Stage::Count(s) => s.execute_with_context(docs, ctx),
            Stage::Sort(s) => s.execute_with_context(docs, ctx),
            Stage::Limit(s) => s.execute_with_context(docs, ctx),
            Stage::Skip(s) => s.execute_with_context(docs, ctx),
            Stage::Unwind(s) => s.execute_with_context(docs, ctx),
        }
    }
}

// ========== Individual Stage Implementations ==========

impl StageExecutor for MatchStage {
    fn execute_with_context(
        &self,
        docs: Vec<Value>,
        _ctx: &AggregationLimitContext,
    ) -> Result<Vec<Value>> {
        // $match uses streaming iterator in chain_streamable_stages
        // When executed standalone, just filter without counting
        // (counting happens at document loading level)
        self.execute(docs)
    }
}

impl StageExecutor for ProjectStage {
    fn execute_with_context(
        &self,
        docs: Vec<Value>,
        _ctx: &AggregationLimitContext,
    ) -> Result<Vec<Value>> {
        // $project is memory-neutral (1:1 document ratio)
        // No limit checking needed
        self.execute(docs)
    }
}

impl StageExecutor for GroupStage {
    fn execute_with_context(
        &self,
        docs: Vec<Value>,
        ctx: &AggregationLimitContext,
    ) -> Result<Vec<Value>> {
        // Use context for group/push/addtoset limit tracking
        self.execute_with_context_impl(docs, ctx)
    }
}

impl StageExecutor for CountStage {
    fn execute_with_context(
        &self,
        docs: Vec<Value>,
        _ctx: &AggregationLimitContext,
    ) -> Result<Vec<Value>> {
        // Count stage only needs the number of documents
        self.execute(docs)
    }
}

impl StageExecutor for SortStage {
    fn execute_with_context(
        &self,
        docs: Vec<Value>,
        _ctx: &AggregationLimitContext,
    ) -> Result<Vec<Value>> {
        // $sort is in-place, doesn't change document count
        // Memory is bounded by input size
        self.execute(docs)
    }
}

impl StageExecutor for LimitStage {
    fn execute_with_context(
        &self,
        docs: Vec<Value>,
        _ctx: &AggregationLimitContext,
    ) -> Result<Vec<Value>> {
        // $limit only reduces documents, no memory concerns
        self.execute(docs)
    }
}

impl StageExecutor for SkipStage {
    fn execute_with_context(
        &self,
        docs: Vec<Value>,
        _ctx: &AggregationLimitContext,
    ) -> Result<Vec<Value>> {
        // $skip only reduces documents, no memory concerns
        self.execute(docs)
    }
}

impl StageExecutor for UnwindStage {
    fn execute_with_context(
        &self,
        docs: Vec<Value>,
        ctx: &AggregationLimitContext,
    ) -> Result<Vec<Value>> {
        // Use context for unwind output limit tracking
        self.execute_with_context_impl(docs, ctx)
    }
}
