// src/aggregation/optimizer.rs
// Pipeline optimization - detects patterns for memory-efficient execution

use crate::aggregation::types::Stage;

/// Pipeline optimization hints derived from pattern analysis
#[derive(Debug, Clone, Default)]
pub struct PipelineOptimization {
    /// If $sort is followed directly by $limit, this contains the limit value.
    /// This allows Top-K optimization: O(k) memory instead of O(n).
    pub sort_limit_hint: Option<usize>,

    /// Index of the $sort stage that can be optimized
    pub sort_stage_index: Option<usize>,
}

/// Analyze pipeline stages to find optimization opportunities.
///
/// # Detected Patterns
///
/// ## $sort → $limit (Top-K Optimization)
///
/// When $sort is immediately followed by $limit, we can use a bounded heap
/// to keep only the top K elements instead of sorting all documents.
///
/// Before: O(n log n) time, O(n) memory
/// After:  O(n log k) time, O(k) memory
///
/// Example pipeline:
/// ```json
/// [
///   {"$group": {"_id": "$email", "count": {"$sum": 1}}},
///   {"$sort": {"count": -1}},
///   {"$limit": 5}
/// ]
/// ```
///
/// With 50,000 groups:
/// - Without optimization: sort all 50K, use 50KB+ memory
/// - With optimization: maintain 5-element heap, use ~500 bytes
///
/// # Arguments
/// * `stages` - Pipeline stages to analyze
///
/// # Returns
/// `PipelineOptimization` with detected hints
pub fn analyze_pipeline(stages: &[Stage]) -> PipelineOptimization {
    let mut opt = PipelineOptimization::default();

    // Look for $sort → $limit pattern
    for i in 0..stages.len().saturating_sub(1) {
        if let Stage::Sort(_) = &stages[i] {
            if let Stage::Limit(limit_stage) = &stages[i + 1] {
                opt.sort_limit_hint = Some(limit_stage.limit);
                opt.sort_stage_index = Some(i);
                // Found the pattern - no need to continue
                // (only first $sort → $limit is optimized)
                break;
            }
        }
    }

    opt
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aggregation::Pipeline;
    use serde_json::json;

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
        // $sort → $skip → $limit - NOT optimizable
        // (skip in between breaks the pattern)
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
        // Only first $sort → $limit is detected
        assert_eq!(opt.sort_limit_hint, Some(10));
        assert_eq!(opt.sort_stage_index, Some(0));
    }
}
