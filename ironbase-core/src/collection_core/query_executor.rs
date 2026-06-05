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
use std::collections::BinaryHeap;
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

        // Match find_options::apply_sort EXACTLY: missing < present, compatible
        // types use the core comparator, and incompatible/mixed types fall back to
        // a stable type rank. This must agree with the full in-memory sort, or the
        // top-k heap would evict the wrong documents on a mixed-type field (P1-5).
        let cmp = match (a_val, b_val) {
            (None, None) => Ordering::Equal,
            (None, Some(_)) => Ordering::Less,
            (Some(_), None) => Ordering::Greater,
            (Some(av), Some(bv)) => value_utils::compare_values(av, bv).unwrap_or_else(|| {
                value_utils::type_priority(av).cmp(&value_utils::type_priority(bv))
            }),
        };

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
    // Delegate to streaming O(k) heap-based implementation
    // Previous implementation collected ALL docs O(N) then sorted — OOM risk on large result sets
    topk_documents_streaming(docs, skip, limit, sort)
}

/// Apply Top-K selection with true O(k) memory using a custom heap-based approach
///
/// Unlike topk_documents which collects all, this one uses a heap for memory efficiency.
/// Use this when dealing with very large iterators where collecting all would cause OOM.
pub fn topk_documents_streaming<I>(
    docs: I,
    skip: usize,
    limit: usize,
    sort: &[(String, i32)],
) -> Vec<Value>
where
    I: Iterator<Item = Value>,
{
    if limit == 0 {
        return Vec::new();
    }

    let mut heap = TopKHeap::new(skip, limit, sort);
    for doc in docs {
        heap.push(doc);
    }
    heap.into_sorted()
}

/// Heap entry ordered by `(sort key, scan sequence)`.
///
/// `Ord` yields a **max-heap**: the document that sorts *last* — or, among equal
/// sort keys, the one scanned *later* (higher `seq`) — sits at the top, so it is
/// the first evicted once the heap is full. The `seq` tiebreaker makes the top-k
/// **stable**: among tied keys the earliest-scanned documents are retained and
/// ordered by scan order, byte-for-byte matching the old full stable
/// `apply_sort` + truncate (which kept the first k in scan order on ties).
struct HeapItem {
    doc: Value,
    /// Monotonic scan index; lower = scanned earlier. Tiebreaker for equal keys.
    seq: u64,
    sort: Arc<[(String, i32)]>,
}

impl PartialEq for HeapItem {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
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
        // Primary: sort spec. Tiebreaker: scan sequence (lower = earlier = kept).
        // Max-heap → largest at top → evicted first; a later-scanned tie has a
        // higher seq, so it ranks "larger" and is evicted before an earlier tie.
        compare_docs_by_sort(&self.doc, &other.doc, &self.sort).then(self.seq.cmp(&other.seq))
    }
}

/// Reusable O(k) top-k selector backed by a max-heap (k = skip + limit).
///
/// Streams documents through a heap that retains only the k smallest seen so far
/// (per the sort spec, ties broken by scan order), so memory stays O(k) regardless
/// of how many documents are offered. Use this when the input would otherwise
/// materialize O(n) before being truncated to a limit — e.g. an unindexed `find`
/// sort+limit scan.
///
/// `topk_documents_streaming` is the iterator-friendly wrapper; callers with a
/// fallible loop (e.g. document loading that returns `Result`) can drive a
/// `TopKHeap` directly via [`TopKHeap::push`].
pub struct TopKHeap {
    heap: BinaryHeap<HeapItem>,
    k: usize,
    skip: usize,
    next_seq: u64,
    sort_spec: Arc<[(String, i32)]>,
}

impl TopKHeap {
    /// Create a selector that yields `limit` documents after skipping `skip`,
    /// ordered by `sort`.
    pub fn new(skip: usize, limit: usize, sort: &[(String, i32)]) -> Self {
        let k = skip.saturating_add(limit);
        let sort_spec: Arc<[(String, i32)]> = Arc::from(sort.to_vec().into_boxed_slice());
        Self {
            // Deliberately NOT `with_capacity(k)`: for deep pagination k = skip +
            // limit can be enormous while only a handful of documents match, and
            // `with_capacity` is infallible — it would eagerly allocate (and abort
            // on failure, or panic on usize::MAX) gigabytes regardless of data
            // size. Starting empty lets the heap grow lazily to the number of
            // retained docs (≤ k), so a deep skip over few matches stays cheap.
            heap: BinaryHeap::new(),
            k,
            skip,
            next_seq: 0,
            sort_spec,
        }
    }

    /// Offer one document. O(log k). Documents that cannot enter the top-k are
    /// dropped immediately, so the heap never holds more than k entries.
    pub fn push(&mut self, doc: Value) {
        if self.k == 0 {
            return;
        }
        let item = HeapItem {
            doc,
            seq: self.next_seq,
            sort: Arc::clone(&self.sort_spec),
        };
        // saturating_add: a single scan never approaches u64::MAX docs, but this
        // avoids any overflow panic/wrap if it somehow did.
        self.next_seq = self.next_seq.saturating_add(1);
        if self.heap.len() < self.k {
            self.heap.push(item);
        } else if let Some(top) = self.heap.peek() {
            // Keep `item` only if it ranks before the current worst-of-the-best.
            // A tie never displaces an earlier doc: a later scan seq ranks larger.
            if item.cmp(top) == Ordering::Less {
                self.heap.pop();
                self.heap.push(item);
            }
        }
    }

    /// Drain into a fully sorted Vec with `skip` applied. O(k log k).
    ///
    /// Sorted by `(sort key, scan seq)` so ties resolve in scan order, reproducing
    /// the old stable `apply_sort` + truncate output exactly. `into_iter` drains
    /// the heap in arbitrary order, so the explicit sort is required.
    pub fn into_sorted(self) -> Vec<Value> {
        let mut items: Vec<HeapItem> = self.heap.into_iter().collect();
        items.sort();
        items.into_iter().skip(self.skip).map(|h| h.doc).collect()
    }
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
    fn test_topk_heap_direct_bounded() {
        // Drive the heap directly (the fallible-loop entry point used by `find`).
        // Offer 1000 docs in arbitrary order but keep only the top 3 ascending —
        // the heap must never hold more than k = skip + limit = 3 entries.
        let sort = vec![("age".to_string(), 1)];
        let mut heap = TopKHeap::new(0, 3, &sort);
        for age in [500, 1, 999, 2, 3, 750, 4, 250] {
            heap.push(json!({ "age": age }));
            assert!(
                heap.heap.len() <= 3,
                "heap must stay bounded at k=3, got {}",
                heap.heap.len()
            );
        }
        let result = heap.into_sorted();
        assert_eq!(result.len(), 3);
        assert_eq!(result[0]["age"], 1);
        assert_eq!(result[1]["age"], 2);
        assert_eq!(result[2]["age"], 3);
    }

    #[test]
    fn test_topk_heap_direct_skip() {
        // skip applied after the bounded selection: k = skip(2) + limit(2) = 4,
        // then drop the first 2 → ages 30, 40.
        let sort = vec![("age".to_string(), 1)];
        let mut heap = TopKHeap::new(2, 2, &sort);
        for age in [50, 10, 40, 20, 30] {
            heap.push(json!({ "age": age }));
        }
        let result = heap.into_sorted();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0]["age"], 30);
        assert_eq!(result[1]["age"], 40);
    }

    #[test]
    fn test_topk_heap_stable_ties() {
        // Many docs tie on age=5; tag records scan order. With limit 2 and the
        // age-1 doc arriving last, the survivors must be the age-1 doc plus the
        // FIRST-scanned age-5 doc (stable), not an arbitrary tied doc. The
        // scan-seq tiebreaker guarantees this matches the old stable full sort.
        let sort = vec![("age".to_string(), 1)];
        let mut heap = TopKHeap::new(0, 2, &sort);
        for (age, tag) in [(5, "a"), (5, "b"), (5, "c"), (1, "d")] {
            heap.push(json!({ "age": age, "tag": tag }));
        }
        let result = heap.into_sorted();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0]["tag"], json!("d")); // age 1
        assert_eq!(result[1]["tag"], json!("a")); // first-scanned age-5 tie
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
