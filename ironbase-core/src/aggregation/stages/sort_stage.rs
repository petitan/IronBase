// src/aggregation/stages/sort_stage.rs
// $sort stage implementation
//
// TOP-K OPTIMIZATION (2026-01):
// When $sort is followed by $limit K, we use a bounded BinaryHeap
// to keep only the top K elements instead of sorting all documents.
// This reduces memory from O(n) to O(k) and time from O(n log n) to O(n log k).

use crate::aggregation::types::{SortDirection, SortStage};
use crate::error::{IronBaseError, Result};
use crate::value_utils::{compare_values as compare_values_core, get_nested_value};
use serde_json::Value;
use std::cmp::Ordering;
use std::collections::BinaryHeap;

/// Wrapper for sorting documents in a BinaryHeap.
/// Stores pre-extracted sort keys for efficient comparison.
struct SortableDoc {
    doc: Value,
    /// Pre-extracted sort key values for faster comparison
    sort_keys: Vec<Option<Value>>,
    /// Sort field directions (true = ascending, false = descending)
    directions: Vec<bool>,
}

impl SortableDoc {
    fn new(doc: Value, fields: &[(String, SortDirection)]) -> Self {
        // Pre-extract all sort keys once
        let sort_keys: Vec<Option<Value>> = fields
            .iter()
            .map(|(field, _)| get_nested_value(&doc, field).cloned())
            .collect();

        let directions: Vec<bool> = fields
            .iter()
            .map(|(_, dir)| matches!(dir, SortDirection::Ascending))
            .collect();

        Self {
            doc,
            sort_keys,
            directions,
        }
    }
}

impl PartialEq for SortableDoc {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for SortableDoc {}

impl PartialOrd for SortableDoc {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SortableDoc {
    fn cmp(&self, other: &Self) -> Ordering {
        for i in 0..self.sort_keys.len() {
            let cmp =
                compare_optional_values(self.sort_keys[i].as_ref(), other.sort_keys[i].as_ref());

            // Apply direction
            let cmp = if self.directions[i] {
                cmp // Ascending
            } else {
                cmp.reverse() // Descending
            };

            if cmp != Ordering::Equal {
                return cmp;
            }
        }
        Ordering::Equal
    }
}

impl SortStage {
    pub(crate) fn from_json(spec: &Value) -> Result<Self> {
        if let Value::Object(obj) = spec {
            let mut fields = Vec::new();

            for (field, value) in obj {
                let direction = if let Some(n) = value.as_i64() {
                    match n {
                        1 => SortDirection::Ascending,
                        -1 => SortDirection::Descending,
                        _ => {
                            return Err(IronBaseError::AggregationError(
                                "Sort direction must be 1 or -1".to_string(),
                            ))
                        }
                    }
                } else {
                    return Err(IronBaseError::AggregationError(
                        "Sort direction must be 1 or -1".to_string(),
                    ));
                };

                fields.push((field.clone(), direction));
            }

            Ok(SortStage { fields })
        } else {
            Err(IronBaseError::AggregationError(
                "$sort must be an object".to_string(),
            ))
        }
    }

    /// Execute sort with optional Top-K optimization.
    ///
    /// # Arguments
    /// * `docs` - Documents to sort
    /// * `limit_hint` - If Some(k), use Top-K optimization for O(k) memory
    ///
    /// # Top-K Optimization
    ///
    /// When `limit_hint` is Some(k) and k < docs.len(), we use a bounded heap:
    /// - Maintain a max-heap of size k (for ascending sort)
    /// - For each document, push to heap and pop if size > k
    /// - Result: only k documents in memory, O(n log k) time
    ///
    /// Example: 50,000 documents, limit 5
    /// - Without optimization: Sort all 50K, O(n log n) = ~780K comparisons
    /// - With optimization: Heap of 5, O(n log k) = ~116K comparisons
    pub(crate) fn execute_with_limit_hint(
        &self,
        docs: Vec<Value>,
        limit_hint: Option<usize>,
    ) -> Result<Vec<Value>> {
        match limit_hint {
            Some(k) if k > 0 && k < docs.len() => self.execute_topk(docs, k),
            _ => self.execute_full_sort(docs),
        }
    }

    /// Standard full sort - O(n log n) time, O(n) memory
    pub(crate) fn execute(&self, docs: Vec<Value>) -> Result<Vec<Value>> {
        self.execute_full_sort(docs)
    }

    fn execute_full_sort(&self, mut docs: Vec<Value>) -> Result<Vec<Value>> {
        docs.sort_by(|a, b| {
            for (field, direction) in &self.fields {
                let val_a = get_nested_value(a, field);
                let val_b = get_nested_value(b, field);

                let cmp = compare_values(val_a, val_b);
                let cmp = match direction {
                    SortDirection::Ascending => cmp,
                    SortDirection::Descending => cmp.reverse(),
                };

                if cmp != std::cmp::Ordering::Equal {
                    return cmp;
                }
            }
            std::cmp::Ordering::Equal
        });

        Ok(docs)
    }

    /// Top-K sort using bounded heap - O(n log k) time, O(k) memory
    ///
    /// Uses a max-heap to keep only the top K elements (smallest by sort order).
    /// The heap's Ord impl considers sort direction, so "largest" in heap = item
    /// that should be ejected (furthest from top of final sorted result).
    fn execute_topk(&self, docs: Vec<Value>, k: usize) -> Result<Vec<Value>> {
        // Max-heap: largest element (by SortableDoc Ord) at top
        // SortableDoc Ord considers sort direction:
        // - Ascending: 0 < 1 < 2, so 2 is "largest", gets popped
        // - Descending: 9 < 8 < 7 (inverted), so 7 is "largest", gets popped
        // Result: we keep the k "smallest" = top k of final sorted result
        let mut heap: BinaryHeap<SortableDoc> = BinaryHeap::with_capacity(k + 1);

        for doc in docs {
            let sortable = SortableDoc::new(doc, &self.fields);
            heap.push(sortable);

            // Keep only k elements - pop the "largest" which we don't want
            if heap.len() > k {
                heap.pop();
            }
        }

        // Extract and sort the final k elements
        let mut result: Vec<Value> = heap.into_iter().map(|sd| sd.doc).collect();

        // Final sort to ensure correct order (heap doesn't guarantee sorted order)
        result.sort_by(|a, b| {
            for (field, direction) in &self.fields {
                let val_a = get_nested_value(a, field);
                let val_b = get_nested_value(b, field);

                let cmp = compare_values(val_a, val_b);
                let cmp = match direction {
                    SortDirection::Ascending => cmp,
                    SortDirection::Descending => cmp.reverse(),
                };

                if cmp != Ordering::Equal {
                    return cmp;
                }
            }
            Ordering::Equal
        });

        Ok(result)
    }
}

/// Compare optional values for SortableDoc (used by Ord impl)
fn compare_optional_values(a: Option<&Value>, b: Option<&Value>) -> Ordering {
    match (a, b) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (Some(a_val), Some(b_val)) => compare_values_core(a_val, b_val).unwrap_or(Ordering::Equal),
    }
}

fn compare_values(a: Option<&Value>, b: Option<&Value>) -> std::cmp::Ordering {
    match (a, b) {
        (None, None) => std::cmp::Ordering::Equal,
        (None, Some(_)) => std::cmp::Ordering::Less,
        (Some(_), None) => std::cmp::Ordering::Greater,
        (Some(a_val), Some(b_val)) => {
            compare_values_core(a_val, b_val).unwrap_or(std::cmp::Ordering::Equal)
        }
    }
}
