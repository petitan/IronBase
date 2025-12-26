// src/aggregation/helpers.rs
// Helper functions for aggregation pipeline

use crate::error::{IronBaseError, Result};
use crate::value_utils::get_nested_value;
use serde_json::Value;

/// Parse a field reference from JSON value (e.g., "$fieldName" -> "fieldName")
///
/// Used by accumulators like $avg, $min, $max, $first, $last
pub(crate) fn parse_field_reference(value: &Value, op_name: &str) -> Result<String> {
    if let Some(s) = value.as_str() {
        if s.starts_with('$') {
            Ok(s.trim_start_matches('$').to_string())
        } else {
            Err(IronBaseError::AggregationError(format!(
                "{} field reference must start with $",
                op_name
            )))
        }
    } else {
        Err(IronBaseError::AggregationError(format!(
            "{} must be a field reference",
            op_name
        )))
    }
}

/// DEPRECATED: Used by the old batch-based Accumulator::compute() method.
/// The new streaming accumulators use compare_values() directly.
///
/// Compute min or max over documents with integer-first comparison
///
/// BUG #15 FIX: Preserves integer precision for values > 2^53.
/// Uses integer comparison when possible, only falls back to f64 for floats.
///
/// Used by $min and $max accumulators
#[allow(dead_code)]
pub(crate) fn compute_extremum(docs: &[Value], field: &str, is_min: bool) -> Result<Value> {
    let mut result_i64: Option<i64> = None;
    let mut result_u64: Option<u64> = None;
    let mut result_f64: Option<f64> = None;

    for doc in docs {
        if let Some(value) = get_nested_value(doc, field) {
            if let Some(n) = value.as_i64() {
                // Integer value - compare in integer domain
                result_i64 = Some(match result_i64 {
                    Some(current) => {
                        if is_min {
                            current.min(n)
                        } else {
                            current.max(n)
                        }
                    }
                    None => n,
                });
            } else if let Some(n) = value.as_u64() {
                // Large positive integer (> i64::MAX)
                result_u64 = Some(match result_u64 {
                    Some(current) => {
                        if is_min {
                            current.min(n)
                        } else {
                            current.max(n)
                        }
                    }
                    None => n,
                });
            } else if let Some(n) = value.as_f64() {
                // Float value
                result_f64 = Some(match result_f64 {
                    Some(current) => {
                        if is_min {
                            current.min(n)
                        } else {
                            current.max(n)
                        }
                    }
                    None => n,
                });
            }
        }
    }

    // Combine results: compare across types only when necessary
    // Priority: return in original type when possible
    match (result_i64, result_u64, result_f64) {
        // Single type results - return in original type
        (Some(i), None, None) => Ok(Value::from(i)),
        (None, Some(u), None) => Ok(Value::from(u)),
        (None, None, Some(f)) => Ok(Value::from(f)),

        // Mixed i64 and u64 - compare as u64 if i64 is non-negative
        (Some(i), Some(u), None) => {
            if i >= 0 {
                let i_as_u = i as u64;
                if is_min {
                    if i_as_u < u {
                        Ok(Value::from(i))
                    } else {
                        Ok(Value::from(u))
                    }
                } else if i_as_u > u {
                    Ok(Value::from(i))
                } else {
                    Ok(Value::from(u))
                }
            } else {
                // Negative i64 is always less than any u64
                if is_min {
                    Ok(Value::from(i))
                } else {
                    Ok(Value::from(u))
                }
            }
        }

        // Mixed with floats - must compare as f64 (loses precision for very large ints)
        (Some(i), None, Some(f)) => {
            let i_as_f = i as f64;
            if is_min {
                if i_as_f < f {
                    Ok(Value::from(i))
                } else {
                    Ok(Value::from(f))
                }
            } else if i_as_f > f {
                Ok(Value::from(i))
            } else {
                Ok(Value::from(f))
            }
        }
        (None, Some(u), Some(f)) => {
            let u_as_f = u as f64;
            if is_min {
                if u_as_f < f {
                    Ok(Value::from(u))
                } else {
                    Ok(Value::from(f))
                }
            } else if u_as_f > f {
                Ok(Value::from(u))
            } else {
                Ok(Value::from(f))
            }
        }

        // All three types present - very rare edge case
        (Some(i), Some(u), Some(f)) => {
            // Compare all as f64 (best we can do for mixed types)
            let i_as_f = i as f64;
            let u_as_f = u as f64;
            if is_min {
                if i_as_f <= u_as_f && i_as_f <= f {
                    Ok(Value::from(i))
                } else if u_as_f <= f {
                    Ok(Value::from(u))
                } else {
                    Ok(Value::from(f))
                }
            } else if i_as_f >= u_as_f && i_as_f >= f {
                Ok(Value::from(i))
            } else if u_as_f >= f {
                Ok(Value::from(u))
            } else {
                Ok(Value::from(f))
            }
        }

        // No values found
        (None, None, None) => Ok(Value::Null),
    }
}
