// src/aggregation/pipeline.rs
// Pipeline and Stage implementation

use crate::aggregation::types::{
    GroupStage, LimitStage, MatchStage, Pipeline, ProjectStage, SkipStage, SortStage, Stage,
    UnwindStage,
};
use crate::error::{IronBaseError, Result};
use serde_json::Value;

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
            for stage_json in stages_array {
                let stage = Stage::from_json(stage_json)?;
                stages.push(stage);
            }

            Ok(Pipeline { stages })
        } else {
            Err(IronBaseError::AggregationError(
                "Pipeline must be an array".to_string(),
            ))
        }
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
    pub fn execute_streaming<I>(&self, docs: I) -> Result<Vec<Value>>
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

        // Phase 2: Check for $group stage (main optimization target)
        let materialized = if let Some(Stage::Group(group_stage)) = stage_iter.peek() {
            let result = group_stage.execute_streaming(streaming_iter)?;
            stage_iter.next(); // consume the $group stage
            result
        } else {
            // No $group - materialize streamed results
            streaming_iter.collect::<Result<Vec<Value>>>()?
        };

        // Phase 3: Execute remaining stages on materialized results
        let mut docs = materialized;
        for stage in stage_iter {
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
