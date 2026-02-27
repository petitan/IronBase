// src/aggregation/helpers.rs
// Helper functions for aggregation pipeline

use crate::aggregation::types::ValueExpression;
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

pub(crate) fn parse_value_expression(value: &Value, op_name: &str) -> Result<ValueExpression> {
    if let Some(s) = value.as_str() {
        if s.starts_with('$') {
            return Ok(ValueExpression::Field(
                s.trim_start_matches('$').to_string(),
            ));
        }
        return Err(IronBaseError::AggregationError(format!(
            "{} field reference must start with $",
            op_name
        )));
    }

    if let Some(obj) = value.as_object() {
        if obj.is_empty() {
            return Err(IronBaseError::AggregationError(format!(
                "{} expression object cannot be empty",
                op_name
            )));
        }

        // Check if first key is an operator ($)
        // Safety: obj.is_empty() checked above, so .next() is guaranteed Some
        let first_key = obj.keys().next().ok_or_else(|| {
            IronBaseError::AggregationError(format!("{} expression unexpectedly empty", op_name))
        })?;
        if first_key.starts_with('$') {
            // Operator expression — must have exactly one key
            if obj.len() != 1 {
                return Err(IronBaseError::AggregationError(format!(
                    "{} expression must have exactly one operator",
                    op_name
                )));
            }
            let (op, arg) = obj.iter().next().ok_or_else(|| {
                IronBaseError::AggregationError(format!(
                    "{} operator expression unexpectedly empty",
                    op_name
                ))
            })?;
            match op.as_str() {
                "$substr" => {
                    let (field, start, length) = parse_substr_spec(arg, op_name)?;
                    return Ok(ValueExpression::Substr {
                        field,
                        start,
                        length,
                    });
                }
                _ => {
                    return Err(IronBaseError::AggregationError(format!(
                        "{} expression operator not supported: {}",
                        op_name, op
                    )))
                }
            }
        } else {
            // Nested object: {title: "$title", year: "$year"}
            let mut fields = Vec::with_capacity(obj.len());
            for (key, val) in obj {
                let field_expr = parse_value_expression(val, op_name)?; // Recursive
                fields.push((key.clone(), Box::new(field_expr)));
            }
            return Ok(ValueExpression::Object(fields));
        }
    }

    Err(IronBaseError::AggregationError(format!(
        "{} must be a field reference or expression",
        op_name
    )))
}

fn parse_substr_spec(spec: &Value, op_name: &str) -> Result<(String, usize, usize)> {
    let arr = spec.as_array().ok_or_else(|| {
        IronBaseError::AggregationError(format!("{} $substr must be an array", op_name))
    })?;
    if arr.len() != 3 {
        return Err(IronBaseError::AggregationError(format!(
            "{} $substr requires [field, start, length]",
            op_name
        )));
    }

    let field = arr[0].as_str().ok_or_else(|| {
        IronBaseError::AggregationError(format!("{} $substr field must be a string", op_name))
    })?;
    if !field.starts_with('$') {
        return Err(IronBaseError::AggregationError(format!(
            "{} $substr field must start with $",
            op_name
        )));
    }

    let start = parse_substr_number(&arr[1], op_name, "start")?;
    let length = parse_substr_number(&arr[2], op_name, "length")?;

    Ok((field.trim_start_matches('$').to_string(), start, length))
}

fn parse_substr_number(value: &Value, op_name: &str, label: &str) -> Result<usize> {
    let num = value.as_i64().ok_or_else(|| {
        IronBaseError::AggregationError(format!("{} $substr {} must be integer", op_name, label))
    })?;
    if num < 0 {
        return Err(IronBaseError::AggregationError(format!(
            "{} $substr {} must be non-negative",
            op_name, label
        )));
    }
    Ok(num as usize)
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
