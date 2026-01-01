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

use crate::aggregation::optimizer::analyze_pipeline;
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
            // Collect all if no stages
            return docs.collect();
        }

        let mut stage_iter = self.stages.iter().peekable();

        // Phase 1: Process streamable prefix stages ($match, $project)
        // These are applied inline without materializing
        let streaming_iter = self.apply_streamable_stages(docs, &mut stage_iter);

        // MEMORY SAFETY: Wrap iterator with document counter
        // Only enforce limit if there's no leading $match
        let doc_limit = if self.has_leading_match {
            usize::MAX // No limit when $match filters documents
        } else {
            limits.max_docs_without_match
        };

        // Use Rc<Cell> to share counter between iterator and error checking
        let doc_count = Rc::new(Cell::new(0usize));
        let doc_count_clone = Rc::clone(&doc_count);

        let counted_iter = streaming_iter.map(move |doc_result| {
            if doc_result.is_ok() {
                let count = doc_count_clone.get() + 1;
                doc_count_clone.set(count);

                // Check limit every 1000 docs to reduce overhead
                if count % 1000 == 0 && count > doc_limit {
                    return Err(IronBaseError::AggregationError(format!(
                        "Aggregation exceeded document limit: {} documents processed without $match. \
                         Add a $match stage to filter documents, or increase the limit. \
                         Current limit: {} (set via AggregationLimits)",
                        count, doc_limit
                    )));
                }
            }
            doc_result
        });

        // Phase 2: Check for $group stage (main optimization target)
        let materialized = if let Some(Stage::Group(group_stage)) = stage_iter.peek() {
            // Pass limits to group stage for cardinality checking
            let result = group_stage.execute_streaming_with_limits(counted_iter, limits)?;
            stage_iter.next(); // consume the $group stage
            result
        } else {
            // No $group - materialize streamed results with limit check
            let mut results = Vec::new();
            for doc_result in counted_iter {
                let doc = doc_result?;
                results.push(doc);

                // Final limit check
                if results.len() > doc_limit {
                    return Err(IronBaseError::AggregationError(format!(
                        "Aggregation exceeded document limit: {} documents without $match. \
                         Add a $match stage to filter documents. Limit: {}",
                        results.len(),
                        doc_limit
                    )));
                }
            }
            results
        };

        // Phase 3: Execute remaining stages on materialized results
        // Apply Top-K optimization if $sort → $limit pattern detected
        let remaining_stages: Vec<&Stage> = stage_iter.collect();
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
            docs = stage.execute(docs)?;
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
    fn apply_streamable_stages<'a, I>(
        &'a self,
        docs: I,
        stage_iter: &mut std::iter::Peekable<std::slice::Iter<'a, Stage>>,
    ) -> Box<dyn Iterator<Item = Result<Value>> + 'a>
    where
        I: Iterator<Item = Result<Value>> + 'a,
    {
        // Collect all leading streamable stages
        let mut match_stages: Vec<&MatchStage> = Vec::new();
        let mut project_stages: Vec<&ProjectStage> = Vec::new();

        loop {
            match stage_iter.peek() {
                Some(Stage::Match(match_stage)) => {
                    match_stages.push(match_stage);
                    stage_iter.next();
                }
                Some(Stage::Project(project_stage)) => {
                    project_stages.push(project_stage);
                    stage_iter.next();
                }
                _ => break, // Non-streamable stage or end
            }
        }

        if match_stages.is_empty() && project_stages.is_empty() {
            return Box::new(docs);
        }

        // Create streaming iterator that applies all filters and projections
        Box::new(docs.filter_map(move |doc_result| {
            match doc_result {
                Ok(mut doc) => {
                    // Apply all $match filters first
                    for match_stage in &match_stages {
                        match match_stage.matches(&doc) {
                            Ok(true) => {}                 // Continue checking other match stages
                            Ok(false) => return None,      // Filtered out
                            Err(e) => return Some(Err(e)), // Propagate error
                        }
                    }

                    // Apply all $project transformations
                    for project_stage in &project_stages {
                        match project_stage.project_one(&doc) {
                            Ok(projected) => doc = projected,
                            Err(e) => return Some(Err(e)),
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
}
