//! Aggregation pipeline operations for CollectionCore

use serde_json::Value;

use crate::error::Result;
use crate::log_debug;
use crate::storage::{RawStorage, Storage};

use super::CollectionCore;

impl<S: Storage + RawStorage> CollectionCore<S> {
    // ========== AGGREGATION ==========

    /// Execute aggregation pipeline
    ///
    /// # Arguments
    /// * `pipeline_json` - JSON array of pipeline stages
    ///
    /// # Example
    /// ```no_run
    /// use ironbase_core::{DatabaseCore, Document};
    /// use serde_json::json;
    ///
    /// let db = DatabaseCore::open("test.db").unwrap();
    /// let collection = db.collection("users").unwrap();
    ///
    /// let results = collection.aggregate(&json!([
    ///     {"$match": {"age": {"$gte": 18}}},
    ///     {"$group": {"_id": "$city", "count": {"$sum": 1}}},
    ///     {"$sort": {"count": -1}}
    /// ])).unwrap();
    /// ```
    pub fn aggregate(&self, pipeline_json: &Value) -> Result<Vec<Value>> {
        self.aggregate_with_limits(
            pipeline_json,
            crate::aggregation::AggregationLimits::default(),
        )
    }

    /// Execute aggregation pipeline with custom memory limits
    ///
    /// This version allows specifying custom limits to prevent OOM on large collections.
    ///
    /// # Arguments
    /// * `pipeline_json` - JSON array of pipeline stages
    /// * `limits` - Memory safety limits
    ///
    /// # Errors
    /// Returns error if:
    /// - Document count exceeds `max_docs_without_match` (when no $match)
    /// - Group count exceeds `max_group_count`
    ///
    /// # Example
    /// ```rust,ignore
    /// use ironbase_core::aggregation::AggregationLimits;
    ///
    /// let limits = AggregationLimits {
    ///     max_docs_without_match: 50_000,
    ///     max_group_count: 10_000,
    ///     max_memory_mb: 256,
    /// };
    /// let results = collection.aggregate_with_limits(&pipeline, limits)?;
    /// ```
    pub fn aggregate_with_limits(
        &self,
        pipeline_json: &Value,
        limits: crate::aggregation::AggregationLimits,
    ) -> Result<Vec<Value>> {
        let ctx = crate::aggregation::AggregationLimitContext::new(limits);
        self.aggregate_with_context_internal(pipeline_json, &ctx)
    }

    /// Run aggregation with automatic memory-based limits
    ///
    /// Automatically detects system RAM and scales limits accordingly.
    /// This is the **recommended** method for production use where memory
    /// constraints may vary across deployment environments.
    ///
    /// # Memory scaling
    /// - Uses max 25% of available RAM for aggregation
    /// - Limits scale proportionally with available memory
    /// - Falls back to conservative defaults if detection fails
    ///
    /// # Example
    /// ```rust,ignore
    /// // Automatically scales limits based on system RAM
    /// let results = collection.aggregate_auto(&pipeline)?;
    ///
    /// // Equivalent to:
    /// let limits = AggregationLimits::from_system_memory();
    /// let results = collection.aggregate_with_limits(&pipeline, limits)?;
    /// ```
    pub fn aggregate_auto(&self, pipeline_json: &Value) -> Result<Vec<Value>> {
        let limits = crate::aggregation::AggregationLimits::from_system_memory();
        self.aggregate_with_limits(pipeline_json, limits)
    }

    /// Run aggregation with centralized context for limit tracking
    ///
    /// This method uses the new `AggregationLimitContext` system (2026-01 refactoring)
    /// for unified limit tracking across all pipeline stages.
    ///
    /// # Advantages over `aggregate_with_limits`
    /// - Centralized limit tracking (no scattered limit checks)
    /// - Per-group $push/$addToSet tracking
    /// - Consistent error messages across all stages
    /// - Uses streaming iterator adapters for $match/$project
    ///
    /// # Example
    /// ```rust,ignore
    /// use ironbase_core::aggregation::{AggregationLimits, AggregationLimitContext};
    ///
    /// let limits = AggregationLimits::from_system_memory();
    /// let ctx = AggregationLimitContext::new(limits);
    /// let results = collection.aggregate_with_context(&pipeline, &ctx)?;
    ///
    /// // After execution, you can inspect context state:
    /// println!("Documents processed: {}", ctx.docs_processed());
    /// println!("Groups created: {}", ctx.groups_created());
    /// ```
    #[allow(dead_code)] // New API - will be used by clients
    pub fn aggregate_with_context(
        &self,
        pipeline_json: &Value,
        ctx: &crate::aggregation::AggregationLimitContext,
    ) -> Result<Vec<Value>> {
        self.aggregate_with_context_internal(pipeline_json, ctx)
    }

    fn aggregate_with_context_internal(
        &self,
        pipeline_json: &Value,
        ctx: &crate::aggregation::AggregationLimitContext,
    ) -> Result<Vec<Value>> {
        self.check_not_closed()?;
        use crate::aggregation::optimizer::{analyze_pipeline, FastPath};
        use crate::aggregation::Pipeline;

        // Parse pipeline
        let mut pipeline = Pipeline::from_json(pipeline_json)?;

        // =========================================================================
        // FAST PATH OPTIMIZATION (Phase 1)
        // Check for simple patterns that can bypass full pipeline execution
        // =========================================================================
        let opt = analyze_pipeline(pipeline.stages());

        if let Some(fast_path) = opt.fast_path {
            match fast_path {
                FastPath::CountOnly {
                    filter,
                    output_field,
                    multiplier,
                    include_id,
                } => {
                    // Use count_documents() instead of full scan - O(1) for unfiltered!
                    let query = filter.unwrap_or_else(|| serde_json::json!({}));
                    let count = self.count_documents(&query)?;
                    // saturating_mul to match the streaming accumulator path, which
                    // saturates on overflow (accumulator.rs $sum: <constant>). Plain `*`
                    // would panic in debug builds and silently wrap in release.
                    let result_count = (count as i64).saturating_mul(multiplier);

                    log_debug!(
                        "aggregate FAST PATH: CountOnly ({} docs, multiplier {})",
                        count,
                        multiplier
                    );

                    // MongoDB semantics: a `{$group: {_id: null}}` or `$count` over an
                    // EMPTY input set produces NO document. The streaming $group path
                    // (group_stage::execute_streaming_with_context) and CountStage::execute
                    // both return [] for zero input rows, so the fast path must match them
                    // instead of emitting a spurious `{_id: null, <field>: 0}` / `{field: 0}`.
                    let mut docs = if count == 0 {
                        Vec::new()
                    } else {
                        let mut doc = serde_json::json!({ output_field: result_count });
                        if include_id {
                            if let Some(obj) = doc.as_object_mut() {
                                obj.insert("_id".to_string(), serde_json::Value::Null);
                            }
                        }
                        vec![doc]
                    };

                    let stages = pipeline.stages();
                    let group_idx =
                        if matches!(stages.first(), Some(crate::aggregation::Stage::Match(_))) {
                            1
                        } else {
                            0
                        };
                    let remaining = stages.get(group_idx + 1..).unwrap_or(&[]);
                    if !remaining.is_empty() {
                        for stage in remaining {
                            docs = stage.execute_with_context(docs, ctx)?;
                        }
                    }

                    return Ok(docs);
                }
                FastPath::CountByField { .. } => {
                    // CountByField optimization is handled by the existing index-based
                    // $group execution path below (try_index_based_execute_with_context).
                    // Fall through to regular execution which already handles this case.
                }
            }
        }

        // =========================================================================
        // REGULAR EXECUTION PATH
        // =========================================================================

        // Extract leading $match for index optimization
        let match_query = pipeline.extract_leading_match();
        let had_match = match_query.is_some();
        pipeline.set_has_leading_match(had_match);

        // Inform context so it can pick correct doc limit bucket
        ctx.set_leading_match(had_match);

        let query = match_query.unwrap_or_else(|| serde_json::json!({}));

        log_debug!(
            "aggregate_with_context: query {:?} (has_match: {})",
            query,
            had_match
        );

        // INDEX-BASED $GROUP OPTIMIZATION
        // Uses context for both entry counting and group cardinality checks
        if !had_match {
            if let Some(group_stage) = pipeline.peek_leading_group() {
                let indexes = self.indexes.read();
                ctx.enter_streaming_group();
                let indexed_opt = group_stage.try_index_based_execute_with_context(&indexes, ctx);
                if let Some(mut indexed_result) = indexed_opt {
                    ctx.exit_streaming_group(indexed_result.len());
                    log_debug!(
                        "aggregate_with_context: index-based $group ({} groups)",
                        indexed_result.len()
                    );
                    drop(indexes);

                    pipeline.remove_leading_group();

                    use crate::aggregation::optimizer::analyze_pipeline;
                    let opt = analyze_pipeline(&pipeline.stages);

                    for (i, stage) in pipeline.stages.iter().enumerate() {
                        use crate::aggregation::Stage;
                        if let Stage::Sort(sort_stage) = stage {
                            if opt.sort_stage_index == Some(i) && opt.sort_limit_hint.is_some() {
                                indexed_result = sort_stage
                                    .execute_with_limit_hint(indexed_result, opt.sort_limit_hint)?;
                                continue;
                            }
                        }
                        indexed_result = stage.execute_with_context(indexed_result, ctx)?;
                    }

                    return Ok(indexed_result);
                } else {
                    ctx.exit_streaming_group(0);
                }
            }
        }

        // STREAMING EXECUTION with context
        // Get streaming cursor - uses index if match_query has indexed field
        // FIX: Propagate deadline from AggregationLimitContext to find_streaming
        // This enables cooperative cancellation during document collection phase
        let deadline = ctx.deadline();
        let mut cursor = self.find_streaming_with_options(&query, None, deadline)?;
        let doc_iter = std::iter::from_fn(move || match cursor.next() {
            Ok(Some(doc)) => Some(Ok(doc)),
            Ok(None) => None,
            Err(e) => Some(Err(e)),
        });

        // Execute pipeline with streaming iterator and centralized limits
        pipeline.execute_with_context(doc_iter, ctx)
    }
}
