// src/aggregation/pipeline.rs
// Pipeline and Stage implementation
//
// MEMORY SAFETY (2026-01):
// Added AggregationLimits to prevent OOM on large collections.
// - max_docs_without_match: limits full collection scans
// - max_group_count: limits $group cardinality
// See execute_streaming_with_limits() for implementation.
//
// TOP-K OPTIMIZATION (2026-01):
// When $sort is followed by $limit K, we pass the limit hint to SortStage
// for O(k) memory instead of O(n). See optimizer module.

use crate::aggregation::context::AggregationLimitContext;
use crate::aggregation::optimizer::analyze_pipeline;
use crate::aggregation::stream::{chain_streamable_stages, with_counting};
use crate::aggregation::types::{
    AggregationLimits, CountStage, GroupStage, LimitStage, MatchStage, Pipeline, ProjectStage,
    SkipStage, SortStage, Stage, UnwindStage,
};
use crate::error::{IronBaseError, Result};
use crate::find_options::estimate_json_size;
use serde_json::Value;
use std::cell::Cell;
use std::rc::Rc;

/// Maximum check interval for document limit enforcement.
/// Balances overhead (too frequent) vs responsiveness (too infrequent).
const MAX_LIMIT_CHECK_INTERVAL: usize = 1000;

impl Pipeline {
    /// Create pipeline from JSON array
    pub fn from_json(pipeline_json: &Value) -> Result<Self> {
        if let Value::Array(stages_array) = pipeline_json {
            if stages_array.is_empty() {
                return Err(IronBaseError::AggregationError(
                    "Pipeline cannot be empty".to_string(),
                ));
            }

            let mut stages = Vec::new();
            let mut has_leading_match = false;

            for (i, stage_json) in stages_array.iter().enumerate() {
                let stage = Stage::from_json(stage_json)?;
                // Check if first stage is $match
                if i == 0 {
                    if let Stage::Match(_) = &stage {
                        has_leading_match = true;
                    }
                }
                stages.push(stage);
            }

            Ok(Pipeline {
                stages,
                has_leading_match,
            })
        } else {
            Err(IronBaseError::AggregationError(
                "Pipeline must be an array".to_string(),
            ))
        }
    }

    /// Mark that the leading $match was extracted (for limit checking)
    pub fn set_has_leading_match(&mut self, value: bool) {
        self.has_leading_match = value;
    }

    /// Execute pipeline on documents
    ///
    /// SAFETY: This method now uses `execute_streaming_with_limits` internally
    /// with default limits to prevent OOM. For custom limits, use
    /// `execute_streaming_with_limits` directly.
    #[allow(dead_code)]
    pub fn execute(&self, docs: Vec<Value>) -> Result<Vec<Value>> {
        // FIX (2026-01-10): Redirect to safe version with default limits
        // Previously this method had NO OOM protection!
        let limits = AggregationLimits::default();
        self.execute_streaming_with_limits(docs.into_iter().map(Ok), limits)
    }

    /// Execute pipeline with streaming document input
    ///
    /// Memory-efficient execution that processes documents one at a time
    /// for the initial streamable stages, then materializes when needed.
    ///
    /// # Streaming stages (processed one doc at a time):
    /// - `$match` - filter on the fly
    /// - `$project` - transform on the fly (reduces doc size before $group!)
    /// - `$group` - accumulate without storing full docs
    ///
    /// # Materializing stages (need all data):
    /// - `$sort` - must see all documents to sort
    /// - `$limit`, `$skip`, `$unwind` - operate on materialized results
    ///
    /// # Memory comparison for 650K emails (800B each) → 10K groups:
    /// - `execute()`: 650K × 800 bytes = ~500MB
    /// - `execute_streaming()`: ~640KB (only group states)
    /// - With `$project` first: even less (smaller projected docs)
    #[allow(dead_code)]
    pub fn execute_streaming<I>(&self, docs: I) -> Result<Vec<Value>>
    where
        I: Iterator<Item = Result<Value>>,
    {
        // Use default limits
        self.execute_streaming_with_limits(docs, AggregationLimits::default())
    }

    /// Execute pipeline with streaming and explicit memory limits
    ///
    /// This version allows specifying custom limits to prevent OOM on large collections.
    ///
    /// # Arguments
    /// * `docs` - Iterator of documents to process
    /// * `limits` - Memory safety limits
    ///
    /// # Errors
    /// Returns error if:
    /// - Document count exceeds `max_docs_without_match` (when no $match)
    /// - Group count exceeds `max_group_count`
    ///
    /// # Example
    /// ```rust,ignore
    /// let limits = AggregationLimits {
    ///     max_docs_without_match: 10_000,
    ///     max_group_count: 1_000,
    ///     max_memory_mb: 256,
    /// };
    /// let results = pipeline.execute_streaming_with_limits(docs, limits)?;
    /// ```
    pub fn execute_streaming_with_limits<I>(
        &self,
        docs: I,
        limits: AggregationLimits,
    ) -> Result<Vec<Value>>
    where
        I: Iterator<Item = Result<Value>>,
    {
        // Memory budget (max_memory_mb) for the documents this path holds;
        // also the context of the stages after materialization (M3)
        let ctx = AggregationLimitContext::new(limits);

        if self.stages.is_empty() {
            // OOM FIX (2026-01): Apply document limit even for empty pipeline
            // Previously collected ALL documents without limit!
            // Respect has_leading_match: if $match was extracted, use higher limit
            let doc_limit = if self.has_leading_match {
                limits.max_docs_with_match
            } else {
                limits.max_docs_without_match
            };
            // FIX (2026-01-10): Pre-allocate with try_reserve to fail fast on OOM
            // Previously Vec grew without bounds until limit check triggered
            let mut results = Vec::new();
            results.try_reserve(doc_limit.min(10_000)).map_err(|e| {
                IronBaseError::OutOfMemory(format!(
                    "Failed to allocate memory for empty pipeline results: {}",
                    e
                ))
            })?;
            for doc_result in docs {
                let doc = doc_result?;
                if results.len() >= doc_limit {
                    return Err(IronBaseError::AggregationError(format!(
                        "Empty pipeline exceeded document limit: {} documents. \
                         Add pipeline stages or use find() instead. Limit: {}",
                        results.len(),
                        doc_limit
                    )));
                }
                ctx.charge_bytes(estimate_json_size(&doc))?;
                results.push(doc);
            }
            return Ok(results);
        }

        let mut stage_iter = self.stages.iter().peekable();

        // Phase 1: Process streamable prefix stages ($match, $project)
        // These are applied inline without materializing
        let streaming_iter = self.apply_streamable_stages(docs, &mut stage_iter);

        // MEMORY SAFETY: Wrap iterator with document counter
        // Use different limits based on whether there's a leading $match
        // Note: Even with $match, we still need a limit to prevent OOM if $match returns many docs!
        let doc_limit = if self.has_leading_match {
            limits.max_docs_with_match // Higher limit when $match filters documents
        } else {
            limits.max_docs_without_match // Stricter limit for full collection scans
        };

        // Use Rc<Cell> to share counter between iterator and error checking
        let doc_count = Rc::new(Cell::new(0usize));
        let doc_count_clone = Rc::clone(&doc_count);

        // Calculate check interval: every 1000 docs or 10% of limit, whichever is smaller
        // This ensures we catch limit violations promptly even for small limits
        let check_interval = std::cmp::min(
            MAX_LIMIT_CHECK_INTERVAL,
            doc_limit.saturating_div(10).max(1),
        );

        let counted_iter = streaming_iter.map(move |doc_result| {
            if doc_result.is_ok() {
                let count = doc_count_clone.get() + 1;
                doc_count_clone.set(count);

                // Check limit periodically to balance overhead vs responsiveness
                if count % check_interval == 0 && count > doc_limit {
                    return Err(IronBaseError::AggregationError(format!(
                        "Aggregation exceeded document limit: {} documents processed. \
                         Add a $match stage to filter documents, or increase the limit. \
                         Current limit: {} (set via AggregationLimits)",
                        count, doc_limit
                    )));
                }
            }
            doc_result
        });

        // Phase 2: Check for $group stage (main optimization target)
        // Collect remaining stages first to enable early limit detection
        let remaining_stages: Vec<&Stage> = stage_iter.collect();

        let (materialized, stages_consumed) =
            if let Some(Stage::Group(group_stage)) = remaining_stages.first() {
                // Pass limits to group stage for cardinality checking
                let result = group_stage.execute_streaming_with_limits(counted_iter, limits)?;
                (result, 1) // consumed 1 stage ($group)
            } else if let Some(Stage::Count(count_stage)) = remaining_stages.first() {
                let mut count: i64 = 0;
                for doc_result in counted_iter {
                    doc_result?;
                    count += 1;
                }
                // MongoDB semantics: empty input → no $count document (matches the
                // count-only $group path and CountStage::execute).
                let result = if count == 0 {
                    Vec::new()
                } else {
                    vec![serde_json::json!({ count_stage.field.clone(): count })]
                };
                (result, 1) // consumed 1 stage ($count)
            } else {
                // No $group: materialize through the streamable prefix
                // ($skip/$limit/$match/$project), stopping at a used-up $limit
                materialize_streamable_prefix(
                    counted_iter,
                    remaining_stages.iter().copied(),
                    doc_limit,
                    &ctx,
                )?
            };

        // Phase 3: Execute remaining stages on materialized results
        // Skip stages that were already applied during Phase 2
        let remaining_stages: Vec<&Stage> =
            remaining_stages.into_iter().skip(stages_consumed).collect();
        let opt = analyze_pipeline(
            &remaining_stages
                .iter()
                .map(|s| (*s).clone())
                .collect::<Vec<_>>(),
        );

        // CUMULATIVE UNWIND FIX (2026-01): Create context for remaining stages
        // This ensures that multiple $unwind stages share a cumulative counter,
        // preventing OOM when: [$unwind: "$a", $unwind: "$b"] each produce near-limit output.
        // Before: Each $unwind had independent 1M limit → 2 stages could produce 2M docs
        // After: All $unwind stages share 1M cumulative limit

        let mut docs = materialized;
        for (i, stage) in remaining_stages.iter().enumerate() {
            // Check if this is a $sort stage that can use Top-K optimization
            if let Stage::Sort(sort_stage) = stage {
                if opt.sort_stage_index == Some(i) && opt.sort_limit_hint.is_some() {
                    // Use Top-K optimization
                    docs = sort_stage.execute_with_limit_hint(docs, opt.sort_limit_hint)?;
                    continue;
                }
            }
            // Use execute_with_context for cumulative limit tracking (especially $unwind)
            docs = stage.execute_with_context(docs, &ctx)?;
        }

        Ok(docs)
    }

    /// Execute pipeline with centralized context for limit tracking
    ///
    /// This is the preferred method for executing pipelines with the unified
    /// limit tracking system (2026-01 refactoring).
    ///
    /// # Design
    /// Uses `AggregationLimitContext` for:
    /// - Document count tracking (with/without $match distinction)
    /// - Group count tracking in $group stage
    /// - Per-group $push/$addToSet limits
    /// - $unwind output limits
    /// - Early termination (prevents $limit bypass)
    ///
    /// # Memory Efficiency
    /// - Streamable stages ($match, $project) are chained as iterators
    /// - Documents are processed one at a time until materialization is needed
    /// - Top-K optimization for $sort + $limit patterns
    /// - Early $limit prevents loading unnecessary documents
    ///
    /// # Example
    /// ```rust,ignore
    /// let limits = AggregationLimits::from_system_memory();
    /// let ctx = AggregationLimitContext::new(limits);
    /// ctx.set_leading_match(pipeline.has_leading_match);
    ///
    /// let results = pipeline.execute_with_context(docs, &ctx)?;
    /// ```
    #[allow(dead_code)] // Will be used in collection_core integration
    pub fn execute_with_context<'a, I>(
        &self,
        docs: I,
        ctx: &AggregationLimitContext,
    ) -> Result<Vec<Value>>
    where
        I: Iterator<Item = Result<Value>> + 'a,
    {
        // Set leading match flag in context
        ctx.set_leading_match(self.has_leading_match);

        if self.stages.is_empty() {
            // Empty pipeline - just collect with limit checking
            let iter = with_counting(docs, ctx.clone());
            return collect_charged(iter, ctx);
        }

        // Phase 1: Chain streamable stages ($match, $project) as iterators
        // These use with_counting internally if no $match is first
        let (streaming_iter, consumed) = chain_streamable_stages(docs, &self.stages, ctx.clone());

        // Remaining stages after streamable prefix
        let remaining_stages = &self.stages[consumed..];

        if remaining_stages.is_empty() {
            // All stages were streamable - collect results
            return collect_charged(streaming_iter, ctx);
        }

        // Phase 2: Handle $group stage specially (streaming accumulators)
        let (materialized, group_consumed) =
            if let Some(Stage::Group(group_stage)) = remaining_stages.first() {
                // CRITICAL FIX (2026-01): Use STREAMING execution, not collect()!
                // OLD (BAD): streaming_iter.collect() → 650K docs in memory
                // NEW (GOOD): process one at a time, only group states in memory
                ctx.enter_streaming_group();
                let result = match group_stage.execute_streaming_with_context(streaming_iter, ctx) {
                    Ok(res) => {
                        ctx.exit_streaming_group(res.len());
                        res
                    }
                    Err(err) => {
                        ctx.exit_streaming_group(0);
                        return Err(err);
                    }
                };
                (result, 1)
            } else if let Some(Stage::Count(count_stage)) = remaining_stages.first() {
                let mut count: i64 = 0;
                for doc_result in streaming_iter {
                    doc_result?;
                    count += 1;
                }
                // MongoDB semantics: empty input → no $count document (matches the
                // count-only $group path and CountStage::execute).
                let result = if count == 0 {
                    Vec::new()
                } else {
                    vec![serde_json::json!({ count_stage.field.clone(): count })]
                };
                (result, 1)
            } else {
                // No $group: materialize through the streamable prefix
                // ($skip/$limit/$match/$project), stopping at a used-up $limit
                materialize_streamable_prefix(
                    streaming_iter,
                    remaining_stages,
                    ctx.effective_limit(),
                    ctx,
                )?
            };

        // Phase 3: Execute remaining stages with context
        let remaining_stages = &remaining_stages[group_consumed..];
        let opt = analyze_pipeline(remaining_stages);

        let mut docs = materialized;
        for (i, stage) in remaining_stages.iter().enumerate() {
            // Check for Top-K optimization on $sort
            if let Stage::Sort(sort_stage) = stage {
                if opt.sort_stage_index == Some(i) && opt.sort_limit_hint.is_some() {
                    docs = sort_stage.execute_with_limit_hint(docs, opt.sort_limit_hint)?;
                    continue;
                }
            }

            // Use context-aware execution
            docs = stage.execute_with_context(docs, ctx)?;
        }

        Ok(docs)
    }

    /// Apply consecutive streamable stages ($match, $project) inline on the iterator
    ///
    /// Streamable stages are those that:
    /// 1. Process one document at a time (no cross-doc state)
    /// 2. Output 0 or 1 document per input ($match filters, $project transforms)
    ///
    /// NOT streamable: $sort (needs all), $unwind (1→N), $group (handled separately)
    ///
    /// OOM FIX (2026-01): Stages are now applied in original order.
    /// Previously [$match, $project, $match] would run as [$match, $match, $project].
    fn apply_streamable_stages<'a, I>(
        &'a self,
        docs: I,
        stage_iter: &mut std::iter::Peekable<std::slice::Iter<'a, Stage>>,
    ) -> Box<dyn Iterator<Item = Result<Value>> + 'a>
    where
        I: Iterator<Item = Result<Value>> + 'a,
    {
        // Collect all leading streamable stages IN ORDER
        // OOM FIX (2026-01): Preserve order instead of grouping by type
        enum StreamableStage<'b> {
            Match(&'b MatchStage),
            Project(&'b ProjectStage),
        }

        let mut stages: Vec<StreamableStage> = Vec::new();

        loop {
            match stage_iter.peek() {
                Some(Stage::Match(match_stage)) => {
                    stages.push(StreamableStage::Match(match_stage));
                    stage_iter.next();
                }
                Some(Stage::Project(project_stage)) => {
                    stages.push(StreamableStage::Project(project_stage));
                    stage_iter.next();
                }
                _ => break, // Non-streamable stage or end
            }
        }

        if stages.is_empty() {
            return Box::new(docs);
        }

        // Create streaming iterator that applies stages IN ORDER
        Box::new(docs.filter_map(move |doc_result| {
            match doc_result {
                Ok(mut doc) => {
                    // Apply stages in original pipeline order
                    for stage in &stages {
                        match stage {
                            StreamableStage::Match(match_stage) => {
                                match match_stage.matches(&doc) {
                                    Ok(true) => {}            // Continue
                                    Ok(false) => return None, // Filtered out
                                    Err(e) => return Some(Err(e)),
                                }
                            }
                            StreamableStage::Project(project_stage) => {
                                match project_stage.project_one(&doc) {
                                    Ok(projected) => doc = projected,
                                    Err(e) => return Some(Err(e)),
                                }
                            }
                        }
                    }

                    Some(Ok(doc))
                }
                Err(e) => Some(Err(e)),
            }
        }))
    }

    /// Get access to pipeline stages (for optimization analysis)
    #[allow(dead_code)]
    pub fn stages(&self) -> &[Stage] {
        &self.stages
    }
}

impl Stage {
    /// Parse stage from JSON
    pub(crate) fn from_json(stage_json: &Value) -> Result<Self> {
        if let Value::Object(obj) = stage_json {
            if obj.len() != 1 {
                return Err(IronBaseError::AggregationError(
                    "Each stage must have exactly one operator".to_string(),
                ));
            }

            let (stage_name, stage_spec) = obj.iter().next().unwrap();

            match stage_name.as_str() {
                "$match" => Ok(Stage::Match(MatchStage::from_json(stage_spec)?)),
                "$project" => Ok(Stage::Project(ProjectStage::from_json(stage_spec)?)),
                "$group" => Ok(Stage::Group(GroupStage::from_json(stage_spec)?)),
                "$count" => Ok(Stage::Count(CountStage::from_json(stage_spec)?)),
                "$sort" => Ok(Stage::Sort(SortStage::from_json(stage_spec)?)),
                "$limit" => Ok(Stage::Limit(LimitStage::from_json(stage_spec)?)),
                "$skip" => Ok(Stage::Skip(SkipStage::from_json(stage_spec)?)),
                "$unwind" => Ok(Stage::Unwind(UnwindStage::from_json(stage_spec)?)),
                _ => Err(IronBaseError::AggregationError(format!(
                    "Unknown pipeline stage: {}",
                    stage_name
                ))),
            }
        } else {
            Err(IronBaseError::AggregationError(
                "Stage must be an object".to_string(),
            ))
        }
    }

    // NOTE: Stage::execute() removed — it was dead code that bypassed OOM limits
    // for $group/$unwind. Use execute_with_context() for proper limit tracking.
}

/// Collect a pipeline's output, charging every document against the
/// context's memory budget (`max_memory_mb`).
fn collect_charged<I>(iter: I, ctx: &AggregationLimitContext) -> Result<Vec<Value>>
where
    I: Iterator<Item = Result<Value>>,
{
    let mut results = Vec::new();
    for doc in iter {
        let doc = doc?;
        ctx.charge_bytes(estimate_json_size(&doc))?;
        results.push(doc);
    }
    Ok(results)
}

/// One stage of the streamable prefix materialized by
/// [`materialize_streamable_prefix`], with its remaining skip/limit count.
enum PrefixStep<'s> {
    Skip(usize),
    Limit(usize),
    Match(&'s MatchStage),
    Project(&'s ProjectStage),
}

/// Materialize the documents after the leading run of `$skip`, `$limit`,
/// `$match` and `$project` stages, applying them per document in pipeline
/// order and stopping once a `$limit` is used up. Returns the documents and
/// the number of stages applied.
///
/// Every pulled document counts against `doc_limit`: needing more is an
/// error, never a silently shorter result (audit 2026-10-07 A5). `$match` and
/// `$project` between `$skip` and `$limit` are applied in place, so they do
/// not turn skip + limit into a document limit (A6).
fn materialize_streamable_prefix<'s, I, S>(
    mut docs: I,
    stages: S,
    doc_limit: usize,
    ctx: &AggregationLimitContext,
) -> Result<(Vec<Value>, usize)>
where
    I: Iterator<Item = Result<Value>>,
    S: IntoIterator<Item = &'s Stage>,
{
    let mut steps = Vec::new();
    for stage in stages {
        steps.push(match stage {
            Stage::Skip(s) => PrefixStep::Skip(s.skip),
            Stage::Limit(l) => PrefixStep::Limit(l.limit),
            Stage::Match(m) => PrefixStep::Match(m),
            Stage::Project(p) => PrefixStep::Project(p),
            _ => break,
        });
    }

    let mut results = Vec::new();
    let mut processed = 0usize;
    'docs: loop {
        // A used-up $limit lets no further document through
        if steps.iter().any(|s| matches!(s, PrefixStep::Limit(0))) {
            break;
        }
        let Some(doc_result) = docs.next() else {
            break;
        };
        let mut doc = doc_result?;
        processed += 1;
        if processed > doc_limit {
            return Err(IronBaseError::AggregationError(format!(
                "Aggregation exceeded document limit: {} documents processed. \
                 Add a $match stage to filter documents. Limit: {}",
                processed, doc_limit
            )));
        }

        for step in steps.iter_mut() {
            match step {
                PrefixStep::Skip(n) if *n > 0 => {
                    *n -= 1;
                    continue 'docs;
                }
                PrefixStep::Skip(_) => {}
                PrefixStep::Limit(n) => *n -= 1,
                PrefixStep::Match(m) => {
                    if !m.matches(&doc)? {
                        continue 'docs;
                    }
                }
                PrefixStep::Project(p) => doc = p.project_one(&doc)?,
            }
        }

        ctx.charge_bytes(estimate_json_size(&doc))?;
        results.push(doc);
    }

    Ok((results, steps.len()))
}
