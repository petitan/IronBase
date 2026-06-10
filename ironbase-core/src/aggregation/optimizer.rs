// src/aggregation/optimizer.rs
// Pipeline optimization - detects patterns for memory-efficient execution
//
// Pattern detection (`analyze_pipeline`) produces a `PipelineOptimization`:
// a `FastPath` for whole-pipeline bypass, plus a Top-K hint for $sort+$limit.
//
// ## Optimization Patterns
//
// ### CountOnly (O(1) when _id: null, $sum: 1 only)
// ```json
// [{"$group": {"_id": null, "count": {"$sum": 1}}}]
// ```
// → Uses count_documents() instead of full scan. Index-based per-field counting
//   ({"_id": "$field"}) is decided independently by the executor via
//   `GroupStage::can_use_index` / `try_index_based_execute_with_context`.
//
// ### TopK (O(n log k) instead of O(n log n))
// ```json
// [{"$sort": {...}}, {"$limit": k}]
// ```
// → Uses bounded heap instead of full sort

use crate::aggregation::types::{Accumulator, GroupId, GroupStage, Stage, SumExpression};
use serde_json::Value;

// ============================================================================
// PATTERN DETECTION TYPES
// ============================================================================

/// Simplified group _id representation
#[derive(Debug, Clone, PartialEq)]
pub enum GroupIdKind {
    Null,
    Field(String),
    Expression, // Compound or expression-based _id (not optimizable)
}

/// Simplified accumulator representation for optimization
#[derive(Debug, Clone, PartialEq)]
pub enum AccumulatorKind {
    Count(i64),       // $sum: constant
    SumField(String), // $sum: "$field"
    AvgField(String),
    MinField(String),
    MaxField(String),
    FirstField(String),
    LastField(String),
    PushField(String),
    AddToSetField(String),
    Expression,
}

impl AccumulatorKind {
    /// Check if this accumulator is count-compatible (can be done with count query)
    pub fn is_count_only(&self) -> bool {
        matches!(self, AccumulatorKind::Count(_))
    }
}

// ============================================================================
// GROUP SHAPE ANALYSIS
// ============================================================================

/// Analysis result for a $group stage
#[derive(Debug, Clone)]
pub struct GroupShape {
    /// Shape of the _id field
    pub id_kind: GroupIdKind,
    /// List of accumulators with their output field names
    pub accumulators: Vec<(String, AccumulatorKind)>,
}

impl GroupShape {
    /// Analyze a $group stage and extract its shape
    pub fn from_group_stage(group: &GroupStage) -> Self {
        let id_kind = match &group.id {
            GroupId::Null => GroupIdKind::Null,
            GroupId::Field(f) => GroupIdKind::Field(f.clone()),
            GroupId::Substring { .. } => GroupIdKind::Expression,
            GroupId::Object(_) => GroupIdKind::Expression, // Not optimizable
        };

        let accumulators: Vec<(String, AccumulatorKind)> = group
            .accumulators
            .iter()
            .map(|(name, acc)| {
                let kind = match acc {
                    Accumulator::Sum(SumExpression::Constant(n)) => AccumulatorKind::Count(*n),
                    Accumulator::Sum(SumExpression::Field(f)) => {
                        AccumulatorKind::SumField(f.clone())
                    }
                    Accumulator::Avg(f) => AccumulatorKind::AvgField(f.clone()),
                    Accumulator::Min(expr) => match expr {
                        crate::aggregation::types::ValueExpression::Field(f) => {
                            AccumulatorKind::MinField(f.clone())
                        }
                        _ => AccumulatorKind::Expression,
                    },
                    Accumulator::Max(expr) => match expr {
                        crate::aggregation::types::ValueExpression::Field(f) => {
                            AccumulatorKind::MaxField(f.clone())
                        }
                        _ => AccumulatorKind::Expression,
                    },
                    Accumulator::First(expr) => match expr {
                        crate::aggregation::types::ValueExpression::Field(f) => {
                            AccumulatorKind::FirstField(f.clone())
                        }
                        _ => AccumulatorKind::Expression,
                    },
                    Accumulator::Last(expr) => match expr {
                        crate::aggregation::types::ValueExpression::Field(f) => {
                            AccumulatorKind::LastField(f.clone())
                        }
                        _ => AccumulatorKind::Expression,
                    },
                    Accumulator::Push(expr) => match expr {
                        crate::aggregation::types::ValueExpression::Field(f) => {
                            AccumulatorKind::PushField(f.clone())
                        }
                        _ => AccumulatorKind::Expression,
                    },
                    Accumulator::AddToSet(expr) => match expr {
                        crate::aggregation::types::ValueExpression::Field(f) => {
                            AccumulatorKind::AddToSetField(f.clone())
                        }
                        _ => AccumulatorKind::Expression,
                    },
                };
                (name.clone(), kind)
            })
            .collect();

        GroupShape {
            id_kind,
            accumulators,
        }
    }

    /// Check if this group shape is CountOnly eligible
    /// Requirements:
    /// - _id: null (single group)
    /// - Only $sum: 1 or $sum: N accumulators (no field-based)
    pub fn is_count_only(&self) -> bool {
        if self.id_kind != GroupIdKind::Null {
            return false;
        }
        self.accumulators
            .iter()
            .all(|(_, kind)| kind.is_count_only())
    }
}

// ============================================================================
// PIPELINE OPTIMIZATION HINTS
// ============================================================================

/// Pipeline optimization hints derived from pattern analysis
#[derive(Debug, Clone, Default)]
pub struct PipelineOptimization {
    /// If $sort is followed directly by $limit, this contains the limit value.
    /// This allows Top-K optimization: O(k) memory instead of O(n).
    pub sort_limit_hint: Option<usize>,

    /// Index of the $sort stage that can be optimized
    pub sort_stage_index: Option<usize>,

    /// Fast path for simple aggregations
    pub fast_path: Option<FastPath>,
}

/// Fast path optimization that bypasses the full pipeline
#[derive(Debug, Clone)]
pub enum FastPath {
    /// Use count_documents() directly
    CountOnly {
        /// Filter to apply (None = count all)
        filter: Option<Value>,
        /// Output field name (e.g., "total" for {"total": {"$sum": 1}})
        output_field: String,
        /// Constant multiplier (e.g., 2 for {"$sum": 2})
        multiplier: i64,
        /// Whether to include `_id: null` in the output document
        include_id: bool,
    },
}

// ============================================================================
// PIPELINE ANALYSIS
// ============================================================================

/// Analyze pipeline stages to find optimization opportunities.
///
/// # Detected Patterns
///
/// ## CountOnly - O(1) or O(index)
/// ```json
/// [{"$group": {"_id": null, "count": {"$sum": 1}}}]
/// [{"$match": {...}}, {"$group": {"_id": null, "total": {"$sum": 1}}}]
/// ```
///
/// ## $sort → $limit (Top-K Optimization)
/// ```json
/// [{"$sort": {"count": -1}}, {"$limit": 5}]
/// ```
///
/// # Arguments
/// * `stages` - Pipeline stages to analyze
///
/// # Returns
/// `PipelineOptimization` with detected hints and fast paths
pub fn analyze_pipeline(stages: &[Stage]) -> PipelineOptimization {
    let mut opt = PipelineOptimization::default();

    // =========================================================================
    // Pattern 1: CountOnly - [$match?] + [$group {_id: null, field: {$sum: 1}}]
    // =========================================================================
    opt.fast_path = detect_count_only_pattern(stages);

    // =========================================================================
    // Pattern 2: $sort → $limit (Top-K)
    // =========================================================================
    for i in 0..stages.len().saturating_sub(1) {
        if let Stage::Sort(_) = &stages[i] {
            if let Stage::Limit(limit_stage) = &stages[i + 1] {
                opt.sort_limit_hint = Some(limit_stage.limit);
                opt.sort_stage_index = Some(i);
                break;
            }
        }
    }

    opt
}

/// Detect CountOnly pattern: optional $match followed by $group with _id: null and only $sum: 1
fn detect_count_only_pattern(stages: &[Stage]) -> Option<FastPath> {
    let mut filter: Option<Value> = None;
    let mut group_idx = 0;

    // Check for leading $match
    if let Some(Stage::Match(match_stage)) = stages.first() {
        filter = Some(match_stage.query.clone().into_json());
        group_idx = 1;
    }

    // Check for $count stage (optional leading $match)
    if let Some(Stage::Count(count_stage)) = stages.get(group_idx) {
        let remaining = &stages[group_idx + 1..];
        if remaining.is_empty() {
            return Some(FastPath::CountOnly {
                filter,
                output_field: count_stage.field.clone(),
                multiplier: 1,
                include_id: false,
            });
        }
    }

    // Check if next stage is $group
    let group_stage = stages.get(group_idx)?;
    let group = match group_stage {
        Stage::Group(g) => g,
        _ => return None,
    };

    // Analyze group shape
    let shape = GroupShape::from_group_stage(group);

    // Check for CountOnly eligibility
    if shape.is_count_only() && shape.accumulators.len() == 1 {
        if let Some((output_field, AccumulatorKind::Count(multiplier))) = shape.accumulators.first()
        {
            let remaining = &stages[group_idx + 1..];
            if remaining.is_empty() || are_post_group_stages_simple(remaining) {
                return Some(FastPath::CountOnly {
                    filter,
                    output_field: output_field.clone(),
                    multiplier: *multiplier,
                    include_id: true,
                });
            }
        }
    }

    None
}

/// Check if remaining stages are simple (don't need full pipeline)
fn are_post_group_stages_simple(stages: &[Stage]) -> bool {
    stages.iter().all(|s| {
        matches!(
            s,
            Stage::Limit(_) | Stage::Skip(_) | Stage::Project(_) | Stage::Sort(_)
        )
    })
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aggregation::Pipeline;
    use serde_json::json;

    // ========== Top-K Pattern Tests ==========

    #[test]
    fn test_detect_sort_limit_pattern() {
        let pipeline = Pipeline::from_json(&json!([
            {"$group": {"_id": "$city", "count": {"$sum": 1}}},
            {"$sort": {"count": -1}},
            {"$limit": 5}
        ]))
        .unwrap();

        let opt = analyze_pipeline(pipeline.stages());
        assert_eq!(opt.sort_limit_hint, Some(5));
        assert_eq!(opt.sort_stage_index, Some(1));
    }

    #[test]
    fn test_no_limit_after_sort() {
        let pipeline = Pipeline::from_json(&json!([
            {"$sort": {"age": 1}},
            {"$project": {"name": 1}}
        ]))
        .unwrap();

        let opt = analyze_pipeline(pipeline.stages());
        assert_eq!(opt.sort_limit_hint, None);
        assert_eq!(opt.sort_stage_index, None);
    }

    #[test]
    fn test_limit_without_sort() {
        let pipeline = Pipeline::from_json(&json!([
            {"$match": {"active": true}},
            {"$limit": 10}
        ]))
        .unwrap();

        let opt = analyze_pipeline(pipeline.stages());
        assert_eq!(opt.sort_limit_hint, None);
    }

    #[test]
    fn test_sort_skip_limit_no_optimization() {
        let pipeline = Pipeline::from_json(&json!([
            {"$sort": {"count": -1}},
            {"$skip": 10},
            {"$limit": 5}
        ]))
        .unwrap();

        let opt = analyze_pipeline(pipeline.stages());
        assert_eq!(opt.sort_limit_hint, None);
    }

    #[test]
    fn test_multiple_sorts_first_optimized() {
        let pipeline = Pipeline::from_json(&json!([
            {"$sort": {"a": 1}},
            {"$limit": 10},
            {"$sort": {"b": -1}},
            {"$limit": 5}
        ]))
        .unwrap();

        let opt = analyze_pipeline(pipeline.stages());
        assert_eq!(opt.sort_limit_hint, Some(10));
        assert_eq!(opt.sort_stage_index, Some(0));
    }

    // ========== CountOnly Pattern Tests ==========

    #[test]
    fn test_detect_count_only_simple() {
        let pipeline = Pipeline::from_json(&json!([
            {"$group": {"_id": null, "total": {"$sum": 1}}}
        ]))
        .unwrap();

        let opt = analyze_pipeline(pipeline.stages());
        assert!(opt.fast_path.is_some());

        if let Some(FastPath::CountOnly {
            filter,
            output_field,
            multiplier,
            include_id,
        }) = opt.fast_path
        {
            assert!(filter.is_none());
            assert_eq!(output_field, "total");
            assert_eq!(multiplier, 1);
            assert!(include_id);
        } else {
            panic!("Expected CountOnly fast path");
        }
    }

    #[test]
    fn test_detect_count_only_with_match() {
        let pipeline = Pipeline::from_json(&json!([
            {"$match": {"status": "active"}},
            {"$group": {"_id": null, "count": {"$sum": 1}}}
        ]))
        .unwrap();

        let opt = analyze_pipeline(pipeline.stages());
        assert!(opt.fast_path.is_some());

        if let Some(FastPath::CountOnly {
            filter,
            output_field,
            ..
        }) = opt.fast_path
        {
            assert!(filter.is_some());
            assert_eq!(output_field, "count");
        } else {
            panic!("Expected CountOnly fast path");
        }
    }

    #[test]
    fn test_detect_count_stage() {
        let pipeline = Pipeline::from_json(&json!([
            {"$count": "total"}
        ]))
        .unwrap();

        let opt = analyze_pipeline(pipeline.stages());
        assert!(opt.fast_path.is_some());

        if let Some(FastPath::CountOnly {
            output_field,
            include_id,
            ..
        }) = opt.fast_path
        {
            assert_eq!(output_field, "total");
            assert!(!include_id);
        } else {
            panic!("Expected CountOnly fast path for $count");
        }
    }

    #[test]
    fn test_detect_count_only_with_multiplier() {
        let pipeline = Pipeline::from_json(&json!([
            {"$group": {"_id": null, "total": {"$sum": 2}}}
        ]))
        .unwrap();

        let opt = analyze_pipeline(pipeline.stages());
        assert!(opt.fast_path.is_some());

        if let Some(FastPath::CountOnly { multiplier, .. }) = opt.fast_path {
            assert_eq!(multiplier, 2);
        } else {
            panic!("Expected CountOnly fast path with multiplier");
        }
    }

    #[test]
    fn test_no_count_only_with_field_sum() {
        // $sum: "$amount" - not count only!
        let pipeline = Pipeline::from_json(&json!([
            {"$group": {"_id": null, "total": {"$sum": "$amount"}}}
        ]))
        .unwrap();

        let opt = analyze_pipeline(pipeline.stages());
        assert!(opt.fast_path.is_none());
    }

    #[test]
    fn test_no_count_only_with_field_id() {
        // _id: "$email" — a per-field count is NOT CountOnly (id_kind guard);
        // it belongs to the executor's index-based path (GroupStage::can_use_index)
        let pipeline = Pipeline::from_json(&json!([
            {"$group": {"_id": "$email", "count": {"$sum": 1}}}
        ]))
        .unwrap();

        let opt = analyze_pipeline(pipeline.stages());
        assert!(opt.fast_path.is_none());
    }

    // ========== GroupShape Tests ==========

    #[test]
    fn test_group_shape_count_only() {
        let pipeline = Pipeline::from_json(&json!([
            {"$group": {"_id": null, "n": {"$sum": 1}}}
        ]))
        .unwrap();

        if let Stage::Group(group) = &pipeline.stages()[0] {
            let shape = GroupShape::from_group_stage(group);
            assert!(shape.is_count_only());
        }
    }

    #[test]
    fn test_group_shape_complex() {
        // Multiple accumulators - not simple count
        let pipeline = Pipeline::from_json(&json!([
            {"$group": {"_id": "$city", "count": {"$sum": 1}, "total": {"$sum": "$amount"}}}
        ]))
        .unwrap();

        if let Stage::Group(group) = &pipeline.stages()[0] {
            let shape = GroupShape::from_group_stage(group);
            assert!(!shape.is_count_only());
        }
    }
}
