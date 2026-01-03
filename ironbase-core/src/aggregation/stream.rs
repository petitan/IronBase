// src/aggregation/stream.rs
// Iterator adapters for streamable aggregation stages
//
// DESIGN (2026-01):
// Instead of materializing intermediate results into Vec, we chain iterators.
// This reduces memory usage for $match → $project sequences from O(n) to O(1).
//
// Each adapter wraps an inner iterator and applies a transformation:
// - MatchIterator: filters documents based on query
// - ProjectIterator: transforms document structure
//
// The AggregationLimitContext is shared (Rc<RefCell>) so all adapters
// can track document counts against the same counters.

use crate::aggregation::context::AggregationLimitContext;
use crate::aggregation::types::{MatchStage, ProjectStage, Stage};
use crate::error::Result;
use serde_json::Value;

/// Iterator adapter for $match stage
///
/// Filters documents by evaluating the match query on each document.
/// Increments the context's document counter for limit checking.
#[allow(dead_code)] // Phase 2: will be used in pipeline.rs
pub struct MatchIterator<I> {
    inner: I,
    matcher: MatchStage,
    ctx: AggregationLimitContext,
}

#[allow(dead_code)]
impl<I> MatchIterator<I> {
    pub fn new(inner: I, matcher: MatchStage, ctx: AggregationLimitContext) -> Self {
        Self {
            inner,
            matcher,
            ctx,
        }
    }
}

impl<I> Iterator for MatchIterator<I>
where
    I: Iterator<Item = Result<Value>>,
{
    type Item = Result<Value>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.inner.next()? {
                Ok(doc) => {
                    // Increment document counter and check limit
                    if let Err(e) = self.ctx.increment_docs() {
                        return Some(Err(e));
                    }

                    // Evaluate match condition
                    match self.matcher.matches(&doc) {
                        Ok(true) => return Some(Ok(doc)),
                        Ok(false) => continue, // Skip non-matching docs
                        Err(e) => return Some(Err(e)),
                    }
                }
                Err(e) => return Some(Err(e)),
            }
        }
    }
}

/// Iterator adapter for $project stage
///
/// Transforms each document according to the projection specification.
/// Does NOT increment document counter (assuming $match already did).
#[allow(dead_code)] // Phase 2: will be used in pipeline.rs
pub struct ProjectIterator<I> {
    inner: I,
    projector: ProjectStage,
}

#[allow(dead_code)]
impl<I> ProjectIterator<I> {
    pub fn new(inner: I, projector: ProjectStage) -> Self {
        Self { inner, projector }
    }
}

impl<I> Iterator for ProjectIterator<I>
where
    I: Iterator<Item = Result<Value>>,
{
    type Item = Result<Value>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner
            .next()
            .map(|result| result.and_then(|doc| self.projector.project_one(&doc)))
    }
}

/// Iterator adapter for counting documents without $match
///
/// Used when there's no leading $match stage to track document count.
#[allow(dead_code)] // Phase 2: will be used in pipeline.rs
pub struct CountingIterator<I> {
    inner: I,
    ctx: AggregationLimitContext,
}

#[allow(dead_code)]
impl<I> CountingIterator<I> {
    pub fn new(inner: I, ctx: AggregationLimitContext) -> Self {
        Self { inner, ctx }
    }
}

impl<I> Iterator for CountingIterator<I>
where
    I: Iterator<Item = Result<Value>>,
{
    type Item = Result<Value>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.inner.next()? {
            Ok(doc) => {
                if let Err(e) = self.ctx.increment_docs() {
                    Some(Err(e))
                } else {
                    Some(Ok(doc))
                }
            }
            Err(e) => Some(Err(e)),
        }
    }
}

/// Build an iterator chain for streamable stages
///
/// Takes the initial document iterator and a slice of stages, and chains
/// together the streamable stages ($match, $project) into an iterator pipeline.
///
/// IMPORTANT: If the first stage is NOT $match, a CountingIterator is added
/// to ensure document limits are enforced. The $match iterator handles counting
/// internally, so we only add explicit counting when $match is not first.
///
/// Returns the chained iterator and the index of the first non-streamable stage.
///
/// # Example
/// ```rust,ignore
/// let (iter, remaining_idx) = chain_streamable_stages(
///     docs.into_iter().map(Ok),
///     &stages,
///     ctx.clone()
/// );
/// // iter now filters by $match and transforms by $project
/// // remaining_idx points to the first $group/$sort/$unwind stage
/// ```
#[allow(dead_code)] // Phase 2: will be used in pipeline.rs
pub fn chain_streamable_stages<'a, I>(
    docs: I,
    stages: &'a [Stage],
    ctx: AggregationLimitContext,
) -> (Box<dyn Iterator<Item = Result<Value>> + 'a>, usize)
where
    I: Iterator<Item = Result<Value>> + 'a,
{
    enum StreamableStage<'b> {
        Match(&'b MatchStage),
        Project(&'b ProjectStage),
    }

    let mut collected: Vec<StreamableStage> = Vec::new();
    let mut consumed = 0;

    for stage in stages {
        match stage {
            Stage::Match(m) => {
                collected.push(StreamableStage::Match(m));
                consumed += 1;
            }
            Stage::Project(p) => {
                collected.push(StreamableStage::Project(p));
                consumed += 1;
            }
            _ => break,
        }
    }

    if collected.is_empty() {
        return (Box::new(CountingIterator::new(docs, ctx)), 0);
    }

    let has_match_stage = collected
        .iter()
        .any(|s| matches!(s, StreamableStage::Match(_)));

    let mut iter: Box<dyn Iterator<Item = Result<Value>> + 'a> = if has_match_stage {
        Box::new(docs)
    } else {
        Box::new(CountingIterator::new(docs, ctx.clone()))
    };

    for stage in collected {
        match stage {
            StreamableStage::Match(m) => {
                iter = Box::new(MatchIterator::new(iter, m.clone(), ctx.clone()));
            }
            StreamableStage::Project(p) => {
                iter = Box::new(ProjectIterator::new(iter, p.clone()));
            }
        }
    }

    (iter, consumed)
}

/// Wrap an iterator with document counting
///
/// Use this when there's no leading $match to ensure document limits are enforced.
#[allow(dead_code)] // Phase 2: will be used in pipeline.rs
pub fn with_counting<'a, I>(docs: I, ctx: AggregationLimitContext) -> CountingIterator<I>
where
    I: Iterator<Item = Result<Value>> + 'a,
{
    CountingIterator::new(docs, ctx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aggregation::types::AggregationLimits;
    use serde_json::json;

    fn make_docs(n: usize) -> Vec<Result<Value>> {
        (0..n)
            .map(|i| Ok(json!({"i": i, "even": i % 2 == 0})))
            .collect()
    }

    #[test]
    fn test_counting_iterator_respects_limit() {
        let limits = AggregationLimits {
            max_docs_without_match: 5,
            ..Default::default()
        };
        let ctx = AggregationLimitContext::new(limits);

        let docs = make_docs(10);
        let mut iter = CountingIterator::new(docs.into_iter(), ctx);

        // First 5 should succeed
        for _ in 0..5 {
            assert!(iter.next().unwrap().is_ok());
        }

        // 6th should fail
        assert!(iter.next().unwrap().is_err());
    }

    #[test]
    fn test_match_iterator_filters() {
        let limits = AggregationLimits::default();
        let ctx = AggregationLimitContext::new(limits);

        let docs = make_docs(10);
        let matcher = MatchStage::from_json(&json!({"even": true})).unwrap();
        let iter = MatchIterator::new(docs.into_iter(), matcher, ctx);

        let results: Vec<_> = iter.filter_map(Result::ok).collect();
        assert_eq!(results.len(), 5); // Only even numbers: 0, 2, 4, 6, 8
    }

    #[test]
    fn test_match_iterator_respects_limit() {
        let limits = AggregationLimits {
            max_docs_without_match: 5,
            max_docs_with_match: 5,
            ..Default::default()
        };
        let ctx = AggregationLimitContext::new(limits).with_leading_match(true);

        let docs = make_docs(10);
        let matcher = MatchStage::from_json(&json!({})).unwrap(); // Match all
        let mut iter = MatchIterator::new(docs.into_iter(), matcher, ctx);

        // First 5 should succeed
        for _ in 0..5 {
            assert!(iter.next().unwrap().is_ok());
        }

        // 6th should fail
        assert!(iter.next().unwrap().is_err());
    }

    #[test]
    fn test_project_iterator_transforms() {
        let docs = make_docs(3);
        let projector = ProjectStage::from_json(&json!({"i": 1})).unwrap();
        let iter = ProjectIterator::new(docs.into_iter(), projector);

        let results: Vec<_> = iter.filter_map(Result::ok).collect();
        assert_eq!(results.len(), 3);

        // Check that only "i" field is present
        for result in &results {
            assert!(result.get("i").is_some());
            assert!(result.get("even").is_none());
        }
    }

    #[test]
    fn test_chain_streamable_stages() {
        let limits = AggregationLimits::default();
        let ctx = AggregationLimitContext::new(limits);

        let docs = make_docs(10);

        let stages = vec![
            Stage::Match(MatchStage::from_json(&json!({"even": true})).unwrap()),
            Stage::Project(ProjectStage::from_json(&json!({"i": 1})).unwrap()),
        ];

        let (iter, consumed) = chain_streamable_stages(docs.into_iter(), &stages, ctx);

        assert_eq!(consumed, 2);

        let results: Vec<_> = iter.filter_map(Result::ok).collect();
        assert_eq!(results.len(), 5); // Filtered to even only

        // Check projection was applied
        for result in &results {
            assert!(result.get("i").is_some());
            assert!(result.get("even").is_none());
        }
    }

    #[test]
    fn test_chain_stops_at_non_streamable() {
        let limits = AggregationLimits::default();
        let ctx = AggregationLimitContext::new(limits);

        let docs = make_docs(5);

        // $match, $sort, $project - $sort is not streamable
        let stages = vec![
            Stage::Match(MatchStage::from_json(&json!({})).unwrap()),
            Stage::Sort(crate::aggregation::types::SortStage {
                fields: vec![(
                    "i".to_string(),
                    crate::aggregation::types::SortDirection::Ascending,
                )],
            }),
            Stage::Project(ProjectStage::from_json(&json!({"i": 1})).unwrap()),
        ];

        let (_, consumed) = chain_streamable_stages(docs.into_iter(), &stages, ctx);

        // Should stop at $sort (index 1)
        assert_eq!(consumed, 1);
    }
}
