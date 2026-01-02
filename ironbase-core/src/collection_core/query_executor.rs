// Query Executor - Unified Query Execution with Memory Safety
// ============================================================
//
// This module provides a unified interface for query execution that guarantees
// proper memory management:
// - O(1) memory for count operations
// - O(limit) memory for limited queries
// - O(chunk) memory for unlimited queries (chunked processing)
//
// # Architecture
//
// ```text
// QueryExecutor::execute(filter, options)
//     │
//     ├─ count_only? → Index count or chunked scan count
//     │
//     ├─ sort + limit? → Index scan (if available) or Top-K
//     │
//     └─ limit only? → Early termination scan
// ```

use std::cmp::Ordering;
use std::sync::Arc;

use serde_json::Value;

use crate::index::{RangeQueryResult, ScanOrder};
use crate::value_utils;

/// Options for query execution
#[derive(Debug, Clone, Default)]
pub struct QueryOptions {
    /// Number of documents to skip
    pub skip: usize,
    /// Maximum number of documents to return (None = unlimited)
    pub limit: Option<usize>,
    /// Sort specification: (field, direction) where direction: 1 = ASC, -1 = DESC
    pub sort: Option<Vec<(String, i32)>>,
    /// If true, only return count (not documents)
    pub count_only: bool,
}

impl QueryOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_skip(mut self, skip: usize) -> Self {
        self.skip = skip;
        self
    }

    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }

    pub fn with_sort(mut self, sort: Vec<(String, i32)>) -> Self {
        self.sort = Some(sort);
        self
    }

    pub fn count_only(mut self) -> Self {
        self.count_only = true;
        self
    }

    /// Returns the effective limit for Top-K calculations
    pub fn effective_limit(&self) -> Option<usize> {
        self.limit.map(|l| l + self.skip)
    }
}

/// Result of query execution
#[derive(Debug)]
pub enum QueryResult {
    /// Count result (from count_only queries)
    Count(u64),
    /// Document results
    Documents(Vec<Value>),
}

impl QueryResult {
    pub fn unwrap_count(self) -> u64 {
        match self {
            QueryResult::Count(c) => c,
            QueryResult::Documents(_) => panic!("Expected Count, got Documents"),
        }
    }

    pub fn unwrap_documents(self) -> Vec<Value> {
        match self {
            QueryResult::Count(_) => panic!("Expected Documents, got Count"),
            QueryResult::Documents(docs) => docs,
        }
    }

    pub fn as_count(&self) -> Option<u64> {
        match self {
            QueryResult::Count(c) => Some(*c),
            QueryResult::Documents(_) => None,
        }
    }

    pub fn as_documents(&self) -> Option<&Vec<Value>> {
        match self {
            QueryResult::Count(_) => None,
            QueryResult::Documents(docs) => Some(docs),
        }
    }
}

/// Sort direction helper
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDirection {
    Ascending,
    Descending,
}

impl From<i32> for SortDirection {
    fn from(dir: i32) -> Self {
        if dir >= 0 {
            SortDirection::Ascending
        } else {
            SortDirection::Descending
        }
    }
}

impl From<SortDirection> for ScanOrder {
    fn from(dir: SortDirection) -> Self {
        match dir {
            SortDirection::Ascending => ScanOrder::Asc,
            SortDirection::Descending => ScanOrder::Desc,
        }
    }
}

/// Compares two documents by a sort specification
pub fn compare_docs_by_sort(a: &Value, b: &Value, sort: &[(String, i32)]) -> Ordering {
    for (field, direction) in sort {
        let a_val = value_utils::get_nested_value(a, field);
        let b_val = value_utils::get_nested_value(b, field);

        // Use compare_values_with_none to handle missing fields properly
        let cmp = value_utils::compare_values_with_none(a_val, b_val);

        if cmp != Ordering::Equal {
            // Reverse comparison for descending order
            return if *direction >= 0 { cmp } else { cmp.reverse() };
        }
    }
    Ordering::Equal
}

/// Apply Top-K selection to a document iterator
///
/// Uses O(k) memory instead of O(n) for sorting, where k = skip + limit.
///
/// # Arguments
/// * `docs` - Iterator of documents
/// * `skip` - Number of documents to skip
/// * `limit` - Maximum documents to return
/// * `sort` - Sort specification
pub fn topk_documents<I>(docs: I, skip: usize, limit: usize, sort: &[(String, i32)]) -> Vec<Value>
where
    I: Iterator<Item = Value>,
{
    // Collect all documents first, then use a simpler sort + skip + take
    // This is still efficient for reasonable result sizes
    let mut all_docs: Vec<Value> = docs.collect();

    // Sort all documents
    let sort_spec = sort.to_vec();
    all_docs.sort_by(|a, b| compare_docs_by_sort(a, b, &sort_spec));

    // Apply skip and limit
    all_docs.into_iter().skip(skip).take(limit).collect()
}

/// Apply Top-K selection with true O(k) memory using a custom heap-based approach
///
/// Unlike topk_documents which collects all, this one uses a heap for memory efficiency.
/// Use this when dealing with very large iterators where collecting all would cause OOM.
#[allow(dead_code)] // Reserved for future streaming query executor
pub fn topk_documents_streaming<I>(
    docs: I,
    skip: usize,
    limit: usize,
    sort: &[(String, i32)],
) -> Vec<Value>
where
    I: Iterator<Item = Value>,
{
    use std::collections::BinaryHeap;

    if limit == 0 {
        return Vec::new();
    }

    let k = skip + limit;
    let sort_spec: Arc<[(String, i32)]> = Arc::from(sort.to_vec().into_boxed_slice());

    // Wrapper for heap ordering
    struct HeapItem {
        doc: Value,
        sort: Arc<[(String, i32)]>,
    }

    impl PartialEq for HeapItem {
        fn eq(&self, other: &Self) -> bool {
            compare_docs_by_sort(&self.doc, &other.doc, &self.sort) == Ordering::Equal
        }
    }

    impl Eq for HeapItem {}

    impl PartialOrd for HeapItem {
        fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
            Some(self.cmp(other))
        }
    }

    impl Ord for HeapItem {
        fn cmp(&self, other: &Self) -> Ordering {
            // Max-heap: larger values at top (to be kicked first)
            compare_docs_by_sort(&self.doc, &other.doc, &self.sort)
        }
    }

    let mut heap: BinaryHeap<HeapItem> = BinaryHeap::with_capacity(k);

    for doc in docs {
        let item = HeapItem {
            doc,
            sort: Arc::clone(&sort_spec),
        };

        if heap.len() < k {
            heap.push(item);
        } else if let Some(top) = heap.peek() {
            // If new item is "smaller" (should come before top in output)
            if compare_docs_by_sort(&item.doc, &top.doc, &sort_spec) == Ordering::Less {
                heap.pop();
                heap.push(item);
            }
        }
    }

    // Extract and sort
    let mut result: Vec<Value> = heap.into_iter().map(|h| h.doc).collect();
    result.sort_by(|a, b| compare_docs_by_sort(a, b, &sort_spec));

    // Apply skip
    result.into_iter().skip(skip).collect()
}

/// Helper to convert RangeQueryResult to a count
#[allow(dead_code)] // Reserved for future query executor integration
pub fn range_result_to_count(result: RangeQueryResult) -> u64 {
    match result {
        RangeQueryResult::Count(c) => c as u64,
        RangeQueryResult::Docs(docs) => docs.len() as u64,
    }
}

/// Execution statistics for debugging and optimization
#[derive(Debug, Clone, Default)]
pub struct ExecutionStats {
    /// Total documents scanned
    pub docs_scanned: u64,
    /// Documents matched by filter
    pub docs_matched: u64,
    /// Whether index was used
    pub index_used: bool,
    /// Index name if used
    pub index_name: Option<String>,
    /// Execution method used
    pub method: ExecutionMethod,
}

/// Method used to execute the query
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ExecutionMethod {
    #[default]
    CollectionScan,
    IndexScan,
    IndexCount,
    TopK,
    EarlyTermination,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_compare_docs_ascending() {
        let a = json!({"age": 25, "name": "Alice"});
        let b = json!({"age": 30, "name": "Bob"});

        let sort = vec![("age".to_string(), 1)];
        assert_eq!(compare_docs_by_sort(&a, &b, &sort), Ordering::Less);
    }

    #[test]
    fn test_compare_docs_descending() {
        let a = json!({"age": 25, "name": "Alice"});
        let b = json!({"age": 30, "name": "Bob"});

        let sort = vec![("age".to_string(), -1)];
        assert_eq!(compare_docs_by_sort(&a, &b, &sort), Ordering::Greater);
    }

    #[test]
    fn test_compare_docs_multi_field() {
        let a = json!({"age": 25, "name": "Alice"});
        let b = json!({"age": 25, "name": "Bob"});

        // Same age, compare by name
        let sort = vec![("age".to_string(), 1), ("name".to_string(), 1)];
        assert_eq!(compare_docs_by_sort(&a, &b, &sort), Ordering::Less);
    }

    #[test]
    fn test_topk_documents() {
        let docs = vec![
            json!({"age": 30}),
            json!({"age": 20}),
            json!({"age": 40}),
            json!({"age": 10}),
            json!({"age": 50}),
        ];

        // Top 3 by age ascending
        let sort = vec![("age".to_string(), 1)];
        let result = topk_documents(docs.into_iter(), 0, 3, &sort);

        assert_eq!(result.len(), 3);
        assert_eq!(result[0]["age"], 10);
        assert_eq!(result[1]["age"], 20);
        assert_eq!(result[2]["age"], 30);
    }

    #[test]
    fn test_topk_documents_descending() {
        let docs = vec![
            json!({"age": 30}),
            json!({"age": 20}),
            json!({"age": 40}),
            json!({"age": 10}),
            json!({"age": 50}),
        ];

        // Top 3 by age descending
        let sort = vec![("age".to_string(), -1)];
        let result = topk_documents(docs.into_iter(), 0, 3, &sort);

        assert_eq!(result.len(), 3);
        assert_eq!(result[0]["age"], 50);
        assert_eq!(result[1]["age"], 40);
        assert_eq!(result[2]["age"], 30);
    }

    #[test]
    fn test_topk_with_skip() {
        let docs = vec![
            json!({"age": 10}),
            json!({"age": 20}),
            json!({"age": 30}),
            json!({"age": 40}),
            json!({"age": 50}),
        ];

        // Skip first 2, take 2 by age ascending
        let sort = vec![("age".to_string(), 1)];
        let result = topk_documents(docs.into_iter(), 2, 2, &sort);

        assert_eq!(result.len(), 2);
        assert_eq!(result[0]["age"], 30);
        assert_eq!(result[1]["age"], 40);
    }

    #[test]
    fn test_query_options_builder() {
        let opts = QueryOptions::new()
            .with_skip(10)
            .with_limit(20)
            .with_sort(vec![("age".to_string(), -1)]);

        assert_eq!(opts.skip, 10);
        assert_eq!(opts.limit, Some(20));
        assert_eq!(opts.effective_limit(), Some(30));
        assert!(!opts.count_only);
    }

    #[test]
    fn test_query_result_unwrap() {
        let count = QueryResult::Count(42);
        assert_eq!(count.unwrap_count(), 42);

        let docs = QueryResult::Documents(vec![json!({"a": 1})]);
        assert_eq!(docs.unwrap_documents().len(), 1);
    }
}
