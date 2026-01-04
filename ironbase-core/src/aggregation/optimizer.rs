// src/aggregation/optimizer.rs
// Pipeline optimization - detects patterns for memory-efficient execution
//
// ## Phase 1: Logical Plan + Pattern Detection
// - GroupShape detector: identifies _id shape and accumulator types
// - Fast path detection: CountOnly, CountByField, IndexGroup
//
// ## Phase 2: Physical Plan Selection
// - Cost model: estimates based on collection stats and index presence
// - Plan selection: chooses between FullScan, CountOnly, IndexGroup
//
// ## Optimization Patterns
//
// ### CountOnly (O(1) when _id: null, $sum: 1 only)
// ```json
// [{"$group": {"_id": null, "count": {"$sum": 1}}}]
// ```
// → Uses count_documents() instead of full scan
//
// ### CountByField (O(index) when index on _id field)
// ```json
// [{"$group": {"_id": "$email", "count": {"$sum": 1}}}]
// ```
// → Uses index distinct + count per key
//
// ### TopK (O(n log k) instead of O(n log n))
// ```json
// [{"$sort": {...}}, {"$limit": k}]
// ```
// → Uses bounded heap instead of full sort

use crate::aggregation::types::{Accumulator, GroupId, GroupStage, Stage, SumExpression};
use serde_json::Value;

// ============================================================================
// LOGICAL PLAN TYPES
// ============================================================================

/// Logical plan representation for pipeline optimization
#[derive(Debug, Clone)]
pub enum LogicalPlan {
    /// Full collection scan with optional filter
    Scan { filter: Option<Value> },
    /// Count all documents (or filtered subset)
    CountOnly { filter: Option<Value> },
    /// Count documents grouped by a field
    CountByField {
        field: String,
        filter: Option<Value>,
    },
    /// Index-based group (when index exists on group key)
    IndexGroup {
        field: String,
        accumulators: Vec<AccumulatorKind>,
    },
    /// Full scan with group
    FullScanGroup {
        id: GroupIdKind,
        accumulators: Vec<AccumulatorKind>,
        filter: Option<Value>,
    },
}

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

    /// Check if this accumulator can use index min/max optimization
    pub fn is_index_minmax(&self) -> bool {
        matches!(
            self,
            AccumulatorKind::MinField(_) | AccumulatorKind::MaxField(_)
        )
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

    /// Check if this group shape could use index-based counting
    /// Requirements:
    /// - _id: "$field" (field-based grouping)
    /// - Only $sum: 1 accumulators
    pub fn is_count_by_field(&self) -> Option<&str> {
        if let GroupIdKind::Field(ref field) = self.id_kind {
            if self
                .accumulators
                .iter()
                .all(|(_, kind)| kind.is_count_only())
            {
                return Some(field);
            }
        }
        None
    }
}

// ============================================================================
// PHYSICAL PLAN TYPES
// ============================================================================

/// Physical execution plan after optimization
#[derive(Debug, Clone)]
pub enum PhysicalPlan {
    /// Use count_documents() - O(1) for unfiltered, O(index) for filtered
    CountOnly {
        filter: Option<Value>,
        output_field: String,
    },
    /// Use index-based distinct counting
    CountByIndex { field: String, output_field: String },
    /// Full scan with streaming pipeline (default fallback)
    FullScanPipeline,
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
    /// Use index-based counting per unique value
    CountByField {
        /// Field to group by
        field: String,
        /// Output field name for count
        output_field: String,
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

    // Check for CountByField eligibility (index-based)
    if let Some(field) = shape.is_count_by_field() {
        if let Some((output_field, _)) = shape.accumulators.first() {
            let remaining = &stages[group_idx + 1..];
            // Allow $sort + $limit after for top-k pattern
            if remaining.is_empty()
                || are_post_group_stages_simple(remaining)
                || is_sort_limit_pattern(remaining)
            {
                return Some(FastPath::CountByField {
                    field: field.to_string(),
                    output_field: output_field.clone(),
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

/// Check if stages form a $sort + $limit pattern
fn is_sort_limit_pattern(stages: &[Stage]) -> bool {
    if stages.len() < 2 {
        return false;
    }
    matches!((&stages[0], &stages[1]), (Stage::Sort(_), Stage::Limit(_)))
}

// ============================================================================
// COST MODEL (Phase 2)
// ============================================================================

/// Collection statistics for cost estimation
#[derive(Debug, Clone, Default)]
pub struct CollectionStats {
    /// Total document count
    pub doc_count: usize,
    /// Whether an index exists on a specific field
    pub indexed_fields: Vec<String>,
    /// Estimated average document size in bytes
    pub avg_doc_size: usize,
}

impl CollectionStats {
    /// Check if a field has an index
    pub fn has_index(&self, field: &str) -> bool {
        self.indexed_fields.iter().any(|f| f == field)
    }
}

/// Cost estimate for a physical plan
#[derive(Debug, Clone)]
pub struct CostEstimate {
    /// Estimated documents to scan
    pub docs_to_scan: usize,
    /// Estimated memory usage in bytes
    pub memory_bytes: usize,
    /// Cost score (lower is better)
    pub score: f64,
}

impl CostEstimate {
    /// Create cost estimate for CountOnly plan
    pub fn for_count_only(_stats: &CollectionStats, has_filter: bool) -> Self {
        if has_filter {
            // Filtered count may need index scan
            Self {
                docs_to_scan: 0, // Index-based count
                memory_bytes: 64,
                score: 10.0,
            }
        } else {
            // Unfiltered count is O(1)
            Self {
                docs_to_scan: 0,
                memory_bytes: 64,
                score: 1.0,
            }
        }
    }

    /// Create cost estimate for full scan
    pub fn for_full_scan(stats: &CollectionStats) -> Self {
        Self {
            docs_to_scan: stats.doc_count,
            memory_bytes: stats.doc_count * stats.avg_doc_size,
            score: stats.doc_count as f64,
        }
    }

    /// Create cost estimate for index-based group
    pub fn for_index_group(_stats: &CollectionStats, estimated_groups: usize) -> Self {
        Self {
            docs_to_scan: 0,                     // Index scan only
            memory_bytes: estimated_groups * 64, // ~64 bytes per group
            score: (estimated_groups as f64).sqrt() * 10.0,
        }
    }
}

/// Select the best physical plan based on cost
pub fn select_plan(logical: &LogicalPlan, stats: &CollectionStats) -> (PhysicalPlan, CostEstimate) {
    match logical {
        LogicalPlan::CountOnly { filter } => {
            let cost = CostEstimate::for_count_only(stats, filter.is_some());
            let plan = PhysicalPlan::CountOnly {
                filter: filter.clone(),
                output_field: "count".to_string(),
            };
            (plan, cost)
        }
        LogicalPlan::CountByField { field, filter: _ } => {
            if stats.has_index(field) {
                let cost = CostEstimate::for_index_group(stats, stats.doc_count / 100);
                let plan = PhysicalPlan::CountByIndex {
                    field: field.clone(),
                    output_field: "count".to_string(),
                };
                (plan, cost)
            } else {
                let cost = CostEstimate::for_full_scan(stats);
                (PhysicalPlan::FullScanPipeline, cost)
            }
        }
        LogicalPlan::IndexGroup { field, .. } => {
            if stats.has_index(field) {
                let cost = CostEstimate::for_index_group(stats, stats.doc_count / 100);
                let plan = PhysicalPlan::CountByIndex {
                    field: field.clone(),
                    output_field: "count".to_string(),
                };
                (plan, cost)
            } else {
                let cost = CostEstimate::for_full_scan(stats);
                (PhysicalPlan::FullScanPipeline, cost)
            }
        }
        _ => {
            let cost = CostEstimate::for_full_scan(stats);
            (PhysicalPlan::FullScanPipeline, cost)
        }
    }
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
        // _id: "$field" - not count only!
        let pipeline = Pipeline::from_json(&json!([
            {"$group": {"_id": "$email", "count": {"$sum": 1}}}
        ]))
        .unwrap();

        let opt = analyze_pipeline(pipeline.stages());
        // Should be CountByField, not CountOnly
        // Note: field is stored with $ prefix (as parsed from JSON)
        if let Some(FastPath::CountByField { field, .. }) = opt.fast_path {
            assert_eq!(field, "$email");
        } else {
            panic!("Expected CountByField fast path");
        }
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
            assert!(shape.is_count_by_field().is_none());
        }
    }

    #[test]
    fn test_group_shape_count_by_field() {
        let pipeline = Pipeline::from_json(&json!([
            {"$group": {"_id": "$city", "count": {"$sum": 1}}}
        ]))
        .unwrap();

        if let Stage::Group(group) = &pipeline.stages()[0] {
            let shape = GroupShape::from_group_stage(group);
            assert!(!shape.is_count_only());
            // Note: field is stored with $ prefix (as parsed from JSON)
            assert_eq!(shape.is_count_by_field(), Some("$city"));
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
            // Not count_by_field because of $sum: "$amount"
            assert!(shape.is_count_by_field().is_none());
        }
    }

    // ========== Cost Model Tests ==========

    #[test]
    fn test_cost_count_only_unfiltered() {
        let stats = CollectionStats {
            doc_count: 100_000,
            indexed_fields: vec![],
            avg_doc_size: 500,
        };

        let cost = CostEstimate::for_count_only(&stats, false);
        assert_eq!(cost.docs_to_scan, 0);
        assert!(cost.score < 10.0); // Very cheap
    }

    #[test]
    fn test_cost_full_scan() {
        let stats = CollectionStats {
            doc_count: 100_000,
            indexed_fields: vec![],
            avg_doc_size: 500,
        };

        let cost = CostEstimate::for_full_scan(&stats);
        assert_eq!(cost.docs_to_scan, 100_000);
        assert_eq!(cost.score, 100_000.0);
    }

    #[test]
    fn test_select_plan_count_only() {
        let stats = CollectionStats::default();
        let logical = LogicalPlan::CountOnly { filter: None };

        let (plan, cost) = select_plan(&logical, &stats);
        assert!(matches!(plan, PhysicalPlan::CountOnly { .. }));
        assert!(cost.score < 10.0);
    }

    #[test]
    fn test_select_plan_count_by_field_with_index() {
        let stats = CollectionStats {
            doc_count: 100_000,
            indexed_fields: vec!["email".to_string()],
            avg_doc_size: 500,
        };
        let logical = LogicalPlan::CountByField {
            field: "email".to_string(),
            filter: None,
        };

        let (plan, _cost) = select_plan(&logical, &stats);
        assert!(matches!(plan, PhysicalPlan::CountByIndex { .. }));
    }

    #[test]
    fn test_select_plan_count_by_field_no_index() {
        let stats = CollectionStats {
            doc_count: 100_000,
            indexed_fields: vec![], // No index
            avg_doc_size: 500,
        };
        let logical = LogicalPlan::CountByField {
            field: "email".to_string(),
            filter: None,
        };

        let (plan, _cost) = select_plan(&logical, &stats);
        // Falls back to full scan without index
        assert!(matches!(plan, PhysicalPlan::FullScanPipeline));
    }
}
