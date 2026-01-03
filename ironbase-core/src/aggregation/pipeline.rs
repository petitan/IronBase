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
    AggregationLimits, GroupStage, LimitStage, MatchStage, Pipeline, ProjectStage, SkipStage,
    SortStage, Stage, UnwindStage,
};
use crate::error::{IronBaseError, Result};
use serde_json::Value;
use std::cell::Cell;
use std::rc::Rc;

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

    /// Execute pipeline on documents (legacy - loads all into memory)
    /// DEPRECATED: Use execute_streaming() for memory-efficient execution
    #[allow(dead_code)]
    pub fn execute(&self, mut docs: Vec<Value>) -> Result<Vec<Value>> {
        for stage in &self.stages {
            docs = stage.execute(docs)?;
        }
        Ok(docs)
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
        if self.stages.is_empty() {
            // OOM FIX (2026-01): Apply document limit even for empty pipeline
            // Previously collected ALL documents without limit!
            // Respect has_leading_match: if $match was extracted, use higher limit
            let doc_limit = if self.has_leading_match {
                limits.max_docs_with_match
            } else {
                limits.max_docs_without_match
            };
            let mut results = Vec::new();
            for doc_result in docs {
                let doc = doc_result?;
                results.push(doc);
                if results.len() > doc_limit {
                    return Err(IronBaseError::AggregationError(format!(
                        "Empty pipeline exceeded document limit: {} documents. \
                         Add pipeline stages or use find() instead. Limit: {}",
                        results.len(),
                        doc_limit
                    )));
                }
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
        let check_interval = std::cmp::min(1000, doc_limit.saturating_div(10).max(1));

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
            } else {
                // No $group - check for early $limit/$skip optimization
                // OOM FIX (2026-01): Apply $limit during streaming, not after materialization
                let early_limit = Self::detect_early_limit(remaining_stages.iter().copied());

                // CRITICAL: User's $limit MUST NOT bypass doc_limit protection!
                // If user specifies $limit: 1_000_000 but doc_limit is 10K, use 10K.
                let effective_limit = match early_limit {
                    Some((skip, user_limit, stages)) => {
                        // The effective limit includes skip count + materialized docs
                        // e.g., $skip: 100, $limit: 50 → we need 150 docs total from source
                        let total_needed = skip.saturating_add(user_limit);
                        let safe_limit = total_needed.min(doc_limit);
                        Some((
                            skip,
                            user_limit.min(safe_limit.saturating_sub(skip)),
                            stages,
                        ))
                    }
                    None => None,
                };

                let mut results = Vec::new();
                let mut skipped = 0;
                let mut total_processed = 0usize;

                for doc_result in counted_iter {
                    let doc = doc_result?;
                    total_processed += 1;

                    // Always check doc_limit, regardless of early limit
                    if total_processed > doc_limit {
                        return Err(IronBaseError::AggregationError(format!(
                            "Aggregation exceeded document limit: {} documents processed. \
                             Add a $match stage to filter documents. Limit: {}",
                            total_processed, doc_limit
                        )));
                    }

                    // Early termination if we have enough docs (OOM FIX)
                    if let Some((skip, limit, _)) = effective_limit {
                        // Skip documents first (MongoDB semantics)
                        if skipped < skip {
                            skipped += 1;
                            continue; // Skip without materializing
                        }
                        // Check if we have enough documents
                        if results.len() >= limit {
                            break; // EARLY EXIT - don't load more!
                        }
                    }

                    results.push(doc);
                }

                let consumed = effective_limit.map(|(_, _, c)| c).unwrap_or(0);
                (results, consumed)
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
            // Use execute_with_limits to pass limits to stages that need them (e.g., $unwind)
            docs = stage.execute_with_limits(docs, limits)?;
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
    pub fn execute_with_context<I>(
        &self,
        docs: I,
        ctx: &AggregationLimitContext,
    ) -> Result<Vec<Value>>
    where
        I: Iterator<Item = Result<Value>> + 'static,
    {
        // Set leading match flag in context
        ctx.set_leading_match(self.has_leading_match);

        if self.stages.is_empty() {
            // Empty pipeline - just collect with limit checking
            let iter = with_counting(docs, ctx.clone());
            return iter.collect();
        }

        // Detect early $limit/$skip BEFORE streaming to set context limit
        // This ensures doc_limit is respected even with large $limit values
        let remaining_check_stages = &self.stages[..];
        if let Some((skip, limit, _)) = Self::detect_early_limit_from_start(remaining_check_stages)
        {
            // Set effective limit in context (capped at doc_limit)
            ctx.set_early_limit(skip.saturating_add(limit));
        }

        // Phase 1: Chain streamable stages ($match, $project) as iterators
        // These use with_counting internally if no $match is first
        let (streaming_iter, consumed) = chain_streamable_stages(docs, &self.stages, ctx.clone());

        // Remaining stages after streamable prefix
        let remaining_stages = &self.stages[consumed..];

        if remaining_stages.is_empty() {
            // All stages were streamable - collect results
            return streaming_iter.collect();
        }

        // Phase 2: Handle $group stage specially (streaming accumulators)
        let (materialized, group_consumed) =
            if let Some(Stage::Group(group_stage)) = remaining_stages.first() {
                // Use context-aware execution for $group
                let docs_vec: Vec<Value> = streaming_iter.collect::<Result<Vec<_>>>()?;
                let result = group_stage.execute_with_context_impl(docs_vec, ctx)?;
                (result, 1)
            } else {
                // No $group - check for early $limit/$skip
                let early_limit = Self::detect_early_limit(remaining_stages.iter());

                if let Some((skip, limit, stages_to_skip)) = early_limit {
                    // Apply $skip/$limit during materialization
                    // The context already has the effective limit set
                    let effective = ctx.effective_limit();
                    let mut results = Vec::new();
                    let mut skipped = 0;
                    let mut processed = 0;

                    for doc_result in streaming_iter {
                        let doc = doc_result?;
                        processed += 1;

                        // Check against effective limit (includes doc_limit cap)
                        if processed > effective {
                            return Err(IronBaseError::AggregationError(format!(
                                "Aggregation exceeded document limit: {} documents (limit: {})",
                                processed, effective
                            )));
                        }

                        if skipped < skip {
                            skipped += 1;
                            continue;
                        }

                        // Use min of user limit and remaining effective budget
                        if results.len() >= limit.min(effective.saturating_sub(skip)) {
                            break; // Early termination
                        }

                        results.push(doc);
                    }

                    (results, stages_to_skip)
                } else {
                    // No early limit - materialize all
                    let docs_vec: Vec<Value> = streaming_iter.collect::<Result<Vec<_>>>()?;
                    (docs_vec, 0)
                }
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

    /// Detect early limit from start of pipeline (before streaming extraction)
    fn detect_early_limit_from_start(stages: &[Stage]) -> Option<(usize, usize, usize)> {
        let mut skip = 0;
        let mut limit = None;
        let mut stages_consumed = 0;
        let mut found_non_passthrough = false;

        for stage in stages {
            match stage {
                Stage::Skip(s) => {
                    skip += s.skip;
                    stages_consumed += 1;
                }
                Stage::Limit(l) => {
                    limit = Some(l.limit);
                    stages_consumed += 1;
                    break;
                }
                Stage::Match(_) | Stage::Project(_) => {
                    // Streamable stages - continue looking
                    found_non_passthrough = true;
                }
                _ => {
                    found_non_passthrough = true;
                    break;
                }
            }
        }

        // Only return if we found a $limit and didn't skip required stages
        if !found_non_passthrough {
            limit.map(|l| (skip, l, stages_consumed))
        } else {
            // Check after non-passthrough for $group -> $limit patterns
            None
        }
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

    /// Detect if remaining stages start with $skip/$limit that can be applied early
    /// during materialization to prevent loading all documents.
    ///
    /// Returns (skip_count, limit_count, stages_to_remove) if early limit is applicable.
    /// The stages_to_remove is how many leading $skip/$limit stages were consumed.
    ///
    /// Example: [$skip: 10, $limit: 5] → (10, 5, 2)
    /// Example: [$limit: 5] → (0, 5, 1)
    /// Example: [$sort, $limit: 5] → None (can't apply early, $sort needs all docs)
    fn detect_early_limit<'a, I>(stages: I) -> Option<(usize, usize, usize)>
    where
        I: Iterator<Item = &'a Stage>,
    {
        let mut skip = 0;
        let mut limit = None;
        let mut stages_consumed = 0;

        for stage in stages {
            match stage {
                Stage::Skip(s) => {
                    skip += s.skip;
                    stages_consumed += 1;
                }
                Stage::Limit(l) => {
                    limit = Some(l.limit);
                    stages_consumed += 1;
                    break; // $limit terminates early detection
                }
                _ => break, // Non-streamable stage encountered, stop
            }
        }

        limit.map(|l| (skip, l, stages_consumed))
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

    /// Execute this stage
    pub(crate) fn execute(&self, docs: Vec<Value>) -> Result<Vec<Value>> {
        match self {
            Stage::Match(stage) => stage.execute(docs),
            Stage::Project(stage) => stage.execute(docs),
            Stage::Group(stage) => stage.execute(docs),
            Stage::Sort(stage) => stage.execute(docs),
            Stage::Limit(stage) => stage.execute(docs),
            Stage::Skip(stage) => stage.execute(docs),
            Stage::Unwind(stage) => stage.execute(docs),
        }
    }

    /// Execute this stage with explicit limits
    /// Currently only affects $unwind (uses max_unwind_output from limits)
    pub(crate) fn execute_with_limits(
        &self,
        docs: Vec<Value>,
        limits: AggregationLimits,
    ) -> Result<Vec<Value>> {
        match self {
            Stage::Unwind(stage) => stage.execute_with_limit(docs, limits.max_unwind_output),
            // Other stages don't currently use limits in execute (they stream or have own limits)
            _ => self.execute(docs),
        }
    }
}
