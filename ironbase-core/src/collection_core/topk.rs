// Top-K Algorithm Implementation
// ================================
//
// Provides O(k) memory Top-K selection instead of O(n) full sort.
// Critical for queries like: find({}).sort({date: -1}).limit(10)

use std::cmp::Ordering;
use std::collections::BinaryHeap;

/// Wrapper to reverse ordering for max-heap to work as min-heap
struct Reverse<T, F>
where
    F: Fn(&T, &T) -> Ordering,
{
    item: T,
    cmp: F,
}

impl<T, F> PartialEq for Reverse<T, F>
where
    F: Fn(&T, &T) -> Ordering,
{
    fn eq(&self, other: &Self) -> bool {
        (self.cmp)(&self.item, &other.item) == Ordering::Equal
    }
}

impl<T, F> Eq for Reverse<T, F> where F: Fn(&T, &T) -> Ordering {}

impl<T, F> PartialOrd for Reverse<T, F>
where
    F: Fn(&T, &T) -> Ordering,
{
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<T, F> Ord for Reverse<T, F>
where
    F: Fn(&T, &T) -> Ordering,
{
    fn cmp(&self, other: &Self) -> Ordering {
        // For top-k selection:
        // - We keep track of k elements that would appear FIRST in sorted output
        // - Heap's peek() should return the WORST element (would be kicked first)
        // - For ascending order: worst = largest of the k smallest
        // - So larger values should have HIGHER heap priority
        // - cmp(a,b)=Greater means a is larger, so a should be > in heap order
        (self.cmp)(&self.item, &other.item)
    }
}

/// Select top-K elements from an iterator using O(k) memory.
///
/// Uses a min-heap (via reversed BinaryHeap) to track the k smallest elements
/// according to the comparator. The comparator should return:
/// - `Ordering::Less` if first item should come BEFORE second in sorted output
/// - `Ordering::Greater` if first item should come AFTER second in sorted output
///
/// # Algorithm
/// 1. Maintain a min-heap of size k
/// 2. For each new element:
///    - If heap has less than k elements: push
///    - If new element should come before heap's max (worst in top-k): replace
/// 3. At end, drain heap to get sorted result
///
/// # Complexity
/// - Time: O(n log k) where n = iterator length
/// - Space: O(k)
///
/// # Example
/// ```ignore
/// // Get top 5 documents by age (ascending)
/// let top5 = topk_select(
///     docs.into_iter(),
///     5,
///     |a, b| a.get("age").cmp(&b.get("age"))
/// );
/// ```
pub fn topk_select<T, I, F>(items: I, k: usize, cmp: F) -> Vec<T>
where
    I: Iterator<Item = T>,
    F: Fn(&T, &T) -> Ordering + Clone,
{
    if k == 0 {
        return Vec::new();
    }

    let mut heap: BinaryHeap<Reverse<T, F>> = BinaryHeap::with_capacity(k);

    for item in items {
        if heap.len() < k {
            heap.push(Reverse {
                item,
                cmp: cmp.clone(),
            });
        } else if let Some(top) = heap.peek() {
            // If new item should come BEFORE the current worst in top-k
            // (cmp returns Less), then replace
            if cmp(&item, &top.item) == Ordering::Less {
                heap.pop();
                heap.push(Reverse {
                    item,
                    cmp: cmp.clone(),
                });
            }
        }
    }

    // Extract items and sort them
    let mut result: Vec<T> = heap.into_iter().map(|r| r.item).collect();
    result.sort_by(|a, b| cmp(a, b));
    result
}

/// Select top-K elements with skip support.
///
/// Handles skip by selecting k + skip elements, then discarding the first skip.
/// Still O(k + skip) memory which is usually much better than O(n).
///
/// # Note
/// For very large skip values, a different algorithm might be more efficient,
/// but in practice skip is usually small (pagination).
pub fn topk_select_with_skip<T, I, F>(items: I, skip: usize, k: usize, cmp: F) -> Vec<T>
where
    I: Iterator<Item = T>,
    F: Fn(&T, &T) -> Ordering + Clone,
{
    if k == 0 {
        return Vec::new();
    }

    // Select skip + k elements
    let mut result = topk_select(items, skip + k, cmp);

    // Remove first 'skip' elements
    if skip > 0 && result.len() > skip {
        result.drain(0..skip);
    } else if skip >= result.len() {
        return Vec::new();
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_topk_basic() {
        let items = vec![5, 2, 8, 1, 9, 3, 7, 4, 6];
        let top3 = topk_select(items.into_iter(), 3, |a, b| a.cmp(b));
        assert_eq!(top3, vec![1, 2, 3]);
    }

    #[test]
    fn test_topk_descending() {
        let items = vec![5, 2, 8, 1, 9, 3, 7, 4, 6];
        // Reverse comparator for descending
        let top3 = topk_select(items.into_iter(), 3, |a, b| b.cmp(a));
        assert_eq!(top3, vec![9, 8, 7]);
    }

    #[test]
    fn test_topk_empty() {
        let items: Vec<i32> = vec![];
        let result = topk_select(items.into_iter(), 5, |a, b| a.cmp(b));
        assert!(result.is_empty());
    }

    #[test]
    fn test_topk_k_zero() {
        let items = vec![1, 2, 3];
        let result = topk_select(items.into_iter(), 0, |a, b| a.cmp(b));
        assert!(result.is_empty());
    }

    #[test]
    fn test_topk_k_larger_than_input() {
        let items = vec![3, 1, 2];
        let result = topk_select(items.into_iter(), 10, |a, b| a.cmp(b));
        assert_eq!(result, vec![1, 2, 3]);
    }

    #[test]
    fn test_topk_with_skip() {
        let items = vec![5, 2, 8, 1, 9, 3, 7, 4, 6];
        // Skip first 2, then take 3
        let result = topk_select_with_skip(items.into_iter(), 2, 3, |a, b| a.cmp(b));
        assert_eq!(result, vec![3, 4, 5]); // 1, 2 skipped
    }

    #[test]
    fn test_topk_with_skip_exceeds() {
        let items = vec![1, 2, 3];
        let result = topk_select_with_skip(items.into_iter(), 10, 3, |a, b| a.cmp(b));
        assert!(result.is_empty());
    }

    #[test]
    fn test_topk_with_duplicates() {
        let items = vec![3, 1, 3, 2, 1, 2];
        let top3 = topk_select(items.into_iter(), 3, |a, b| a.cmp(b));
        assert_eq!(top3, vec![1, 1, 2]);
    }

    #[test]
    fn test_topk_structs() {
        #[derive(Debug, PartialEq, Clone)]
        struct Doc {
            id: i32,
            score: i32,
        }

        let docs = vec![
            Doc { id: 1, score: 80 },
            Doc { id: 2, score: 95 },
            Doc { id: 3, score: 70 },
            Doc { id: 4, score: 85 },
        ];

        // Top 2 by score (descending)
        let top2 = topk_select(docs.into_iter(), 2, |a, b| b.score.cmp(&a.score));
        assert_eq!(top2[0].id, 2); // score 95
        assert_eq!(top2[1].id, 4); // score 85
    }
}
