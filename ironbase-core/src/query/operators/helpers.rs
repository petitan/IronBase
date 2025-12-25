// src/query/operators/helpers.rs
// Helper functions for operators

use crate::error::Result;
use crate::value_utils::compare_values;
use serde_json::Value;

/// Generic comparison helper for $gt, $gte, $lt, $lte operators
///
/// Handles both direct comparison and MongoDB array element matching.
/// The predicate function determines which orderings are considered a match.
pub fn compare_with_predicate<F>(
    doc_value: Option<&Value>,
    filter_value: &Value,
    predicate: F,
) -> Result<bool>
where
    F: Fn(std::cmp::Ordering) -> bool,
{
    match doc_value {
        None => Ok(false),
        Some(v) => {
            // Direct comparison
            if let Some(ordering) = compare_values(v, filter_value) {
                if predicate(ordering) {
                    return Ok(true);
                }
            }
            // MongoDB array element matching
            if let Value::Array(arr) = v {
                Ok(arr.iter().any(|elem| {
                    compare_values(elem, filter_value)
                        .map(&predicate)
                        .unwrap_or(false)
                }))
            } else {
                Ok(false)
            }
        }
    }
}
