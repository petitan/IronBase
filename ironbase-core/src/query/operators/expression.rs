// src/query/operators/expression.rs
// Expression operator: $expr

use crate::document::Document;
use crate::error::{IronBaseError, Result};
use crate::value_utils::compare_values;
use serde_json::Value;

use super::traits::OperatorMatcher;

/// $expr operator: Allows use of aggregation expressions within the query language
///
/// # MongoDB Spec
///
/// ```json
/// { "$expr": { "$gt": ["$qty", "$reorderLevel"] } }
/// { "$expr": { "$eq": ["$field1", "$field2"] } }
/// ```
///
/// The $expr operator evaluates aggregation expressions to compare fields
/// within the same document.
///
/// # Supported aggregation operators:
/// - Comparison: $eq, $ne, $gt, $gte, $lt, $lte
/// - Arithmetic: $add, $subtract, $multiply, $divide (for computed comparisons)
///
/// # Complexity: CC = 8
pub struct ExprOperator;

/// Helper: Resolve a value that might be a field reference or a computed expression
///
/// - If value is an object with an operator, evaluate it recursively
/// - If value starts with "$", extract field from document
/// - Otherwise return the literal value
fn resolve_expr_value(value: &Value, document: &Document) -> Result<Option<Value>> {
    // Check if it's a computed expression (object with operator)
    if let Some(obj) = value.as_object() {
        if obj.len() == 1 {
            let (op, _) = obj.iter().next().unwrap();
            if op.starts_with('$') {
                return evaluate_expression_value(value, document);
            }
        }
    }

    // Check if it's a field reference
    if let Some(field_ref) = value.as_str() {
        if let Some(field_name) = field_ref.strip_prefix('$') {
            // It's a field reference like "$quantity"
            return Ok(document.get(field_name).cloned());
        }
    }
    // Return the literal value
    Ok(Some(value.clone()))
}

fn evaluate_expression_value(expr: &Value, document: &Document) -> Result<Option<Value>> {
    let expr_obj = expr
        .as_object()
        .ok_or_else(|| IronBaseError::InvalidQuery("Expression must be an object".to_string()))?;
    if expr_obj.len() != 1 {
        return Err(IronBaseError::InvalidQuery(
            "Expression must have exactly one operator".to_string(),
        ));
    }

    let (op, args) = expr_obj.iter().next().unwrap();
    match op.as_str() {
        "$substr" => {
            let arr = args.as_array().ok_or_else(|| {
                IronBaseError::InvalidQuery("$substr requires an array".to_string())
            })?;
            if arr.len() != 3 {
                return Err(IronBaseError::InvalidQuery(
                    "$substr requires [string, start, length]".to_string(),
                ));
            }

            let input = resolve_expr_value(&arr[0], document)?;
            let text = match input {
                Some(Value::String(s)) => s,
                Some(Value::Null) | None => return Ok(Some(Value::Null)),
                Some(other) => {
                    return Err(IronBaseError::InvalidQuery(format!(
                        "$substr requires a string, got {:?}",
                        other
                    )))
                }
            };

            let start = parse_substr_number(&arr[1], "start")?;
            let length = parse_substr_number(&arr[2], "length")?;
            Ok(Some(Value::String(crate::value_utils::substr_string(
                &text, start, length,
            ))))
        }
        _ => {
            let computed = evaluate_arithmetic_expr(expr, document)?;
            Ok(computed.map(Value::from))
        }
    }
}

fn parse_substr_number(value: &Value, label: &str) -> Result<usize> {
    let num = value
        .as_i64()
        .ok_or_else(|| IronBaseError::InvalidQuery(format!("$substr {} must be integer", label)))?;
    if num < 0 {
        return Err(IronBaseError::InvalidQuery(format!(
            "$substr {} must be non-negative",
            label
        )));
    }
    Ok(num as usize)
}

/// Evaluate an arithmetic expression and return a numeric result
fn evaluate_arithmetic_expr(expr: &Value, document: &Document) -> Result<Option<f64>> {
    let expr_obj = match expr.as_object() {
        Some(obj) => obj,
        None => {
            // It's a literal or field reference
            let resolved = resolve_expr_value(expr, document)?;
            return Ok(resolved.and_then(|v| value_to_number(&v)));
        }
    };

    if expr_obj.len() != 1 {
        return Err(IronBaseError::InvalidQuery(
            "Arithmetic expression must have exactly one operator".to_string(),
        ));
    }

    let (op, args) = expr_obj.iter().next().unwrap();

    match op.as_str() {
        "$add" => {
            let arr = args
                .as_array()
                .ok_or_else(|| IronBaseError::InvalidQuery("$add requires an array".to_string()))?;
            let mut result = 0.0;
            for arg in arr {
                if let Some(num) = evaluate_arithmetic_expr(arg, document)? {
                    result += num;
                } else {
                    return Ok(None); // Missing field or non-numeric value
                }
            }
            Ok(Some(result))
        }
        "$subtract" => {
            let arr = args.as_array().ok_or_else(|| {
                IronBaseError::InvalidQuery("$subtract requires an array".to_string())
            })?;
            if arr.len() != 2 {
                return Err(IronBaseError::InvalidQuery(
                    "$subtract requires exactly 2 arguments".to_string(),
                ));
            }
            let left = evaluate_arithmetic_expr(&arr[0], document)?;
            let right = evaluate_arithmetic_expr(&arr[1], document)?;
            match (left, right) {
                (Some(l), Some(r)) => Ok(Some(l - r)),
                _ => Ok(None),
            }
        }
        "$multiply" => {
            let arr = args.as_array().ok_or_else(|| {
                IronBaseError::InvalidQuery("$multiply requires an array".to_string())
            })?;
            let mut result = 1.0;
            for arg in arr {
                if let Some(num) = evaluate_arithmetic_expr(arg, document)? {
                    result *= num;
                } else {
                    return Ok(None);
                }
            }
            Ok(Some(result))
        }
        "$divide" => {
            let arr = args.as_array().ok_or_else(|| {
                IronBaseError::InvalidQuery("$divide requires an array".to_string())
            })?;
            if arr.len() != 2 {
                return Err(IronBaseError::InvalidQuery(
                    "$divide requires exactly 2 arguments".to_string(),
                ));
            }
            let left = evaluate_arithmetic_expr(&arr[0], document)?;
            let right = evaluate_arithmetic_expr(&arr[1], document)?;
            match (left, right) {
                (Some(l), Some(r)) => {
                    if r == 0.0 {
                        Ok(None) // Division by zero returns null in MongoDB
                    } else {
                        Ok(Some(l / r))
                    }
                }
                _ => Ok(None),
            }
        }
        "$mod" => {
            let arr = args
                .as_array()
                .ok_or_else(|| IronBaseError::InvalidQuery("$mod requires an array".to_string()))?;
            if arr.len() != 2 {
                return Err(IronBaseError::InvalidQuery(
                    "$mod requires exactly 2 arguments".to_string(),
                ));
            }
            let left = evaluate_arithmetic_expr(&arr[0], document)?;
            let right = evaluate_arithmetic_expr(&arr[1], document)?;
            match (left, right) {
                (Some(l), Some(r)) => {
                    if r == 0.0 {
                        Ok(None)
                    } else {
                        Ok(Some(l % r))
                    }
                }
                _ => Ok(None),
            }
        }
        "$abs" => {
            let arr = args
                .as_array()
                .ok_or_else(|| IronBaseError::InvalidQuery("$abs requires an array".to_string()))?;
            if arr.len() != 1 {
                return Err(IronBaseError::InvalidQuery(
                    "$abs requires exactly 1 argument".to_string(),
                ));
            }
            let val = evaluate_arithmetic_expr(&arr[0], document)?;
            Ok(val.map(|v| v.abs()))
        }
        "$ceil" => {
            let arr = args.as_array().ok_or_else(|| {
                IronBaseError::InvalidQuery("$ceil requires an array".to_string())
            })?;
            if arr.len() != 1 {
                return Err(IronBaseError::InvalidQuery(
                    "$ceil requires exactly 1 argument".to_string(),
                ));
            }
            let val = evaluate_arithmetic_expr(&arr[0], document)?;
            Ok(val.map(|v| v.ceil()))
        }
        "$floor" => {
            let arr = args.as_array().ok_or_else(|| {
                IronBaseError::InvalidQuery("$floor requires an array".to_string())
            })?;
            if arr.len() != 1 {
                return Err(IronBaseError::InvalidQuery(
                    "$floor requires exactly 1 argument".to_string(),
                ));
            }
            let val = evaluate_arithmetic_expr(&arr[0], document)?;
            Ok(val.map(|v| v.floor()))
        }
        _ => {
            // Not an arithmetic operator - try to resolve as a value
            let resolved = resolve_expr_value(expr, document)?;
            Ok(resolved.and_then(|v| value_to_number(&v)))
        }
    }
}

/// Convert a JSON Value to a number (f64)
fn value_to_number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64(),
        _ => None,
    }
}

/// Evaluate an aggregation expression against a document
fn evaluate_expr(expr: &Value, document: &Document) -> Result<bool> {
    let expr_obj = expr.as_object().ok_or_else(|| {
        IronBaseError::InvalidQuery("$expr expression must be an object".to_string())
    })?;

    // Expression should have exactly one operator
    if expr_obj.len() != 1 {
        return Err(IronBaseError::InvalidQuery(
            "$expr expression must have exactly one operator".to_string(),
        ));
    }

    let (op, args) = expr_obj.iter().next().unwrap();

    match op.as_str() {
        // Comparison operators
        "$eq" => evaluate_comparison_expr(args, document, |ord| ord == std::cmp::Ordering::Equal),
        "$ne" => evaluate_comparison_expr(args, document, |ord| ord != std::cmp::Ordering::Equal),
        "$gt" => evaluate_comparison_expr(args, document, |ord| ord == std::cmp::Ordering::Greater),
        "$gte" => evaluate_comparison_expr(args, document, |ord| {
            ord == std::cmp::Ordering::Greater || ord == std::cmp::Ordering::Equal
        }),
        "$lt" => evaluate_comparison_expr(args, document, |ord| ord == std::cmp::Ordering::Less),
        "$lte" => evaluate_comparison_expr(args, document, |ord| {
            ord == std::cmp::Ordering::Less || ord == std::cmp::Ordering::Equal
        }),

        // Logical operators for nested expressions
        "$and" => {
            let arr = args.as_array().ok_or_else(|| {
                IronBaseError::InvalidQuery("$and in $expr requires an array".to_string())
            })?;
            for sub_expr in arr {
                if !evaluate_expr(sub_expr, document)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        "$or" => {
            let arr = args.as_array().ok_or_else(|| {
                IronBaseError::InvalidQuery("$or in $expr requires an array".to_string())
            })?;
            for sub_expr in arr {
                if evaluate_expr(sub_expr, document)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        "$not" => {
            let arr = args.as_array().ok_or_else(|| {
                IronBaseError::InvalidQuery("$not in $expr requires an array".to_string())
            })?;
            if arr.len() != 1 {
                return Err(IronBaseError::InvalidQuery(
                    "$not in $expr requires exactly one element".to_string(),
                ));
            }
            Ok(!evaluate_expr(&arr[0], document)?)
        }

        _ => Err(IronBaseError::InvalidQuery(format!(
            "Unsupported operator in $expr: {}",
            op
        ))),
    }
}

/// Evaluate a comparison expression like { "$gt": ["$field1", "$field2"] }
/// Supports computed expressions: { "$gt": [{ "$add": ["$a", "$b"] }, "$c"] }
fn evaluate_comparison_expr<F>(args: &Value, document: &Document, compare_fn: F) -> Result<bool>
where
    F: Fn(std::cmp::Ordering) -> bool,
{
    let arr = args.as_array().ok_or_else(|| {
        IronBaseError::InvalidQuery("Comparison in $expr requires an array".to_string())
    })?;

    if arr.len() != 2 {
        return Err(IronBaseError::InvalidQuery(
            "Comparison in $expr requires exactly 2 arguments".to_string(),
        ));
    }

    let left = resolve_expr_value(&arr[0], document)?;
    let right = resolve_expr_value(&arr[1], document)?;

    match (left, right) {
        (Some(l), Some(r)) => {
            if let Some(ordering) = compare_values(&l, &r) {
                Ok(compare_fn(ordering))
            } else {
                // Incompatible types - return false for comparison
                Ok(false)
            }
        }
        // If either field is missing, comparison returns false
        _ => Ok(false),
    }
}

impl OperatorMatcher for ExprOperator {
    fn name(&self) -> &'static str {
        "$expr"
    }

    fn matches(
        &self,
        _doc_value: Option<&Value>,
        filter_value: &Value,
        document: Option<&Document>,
    ) -> Result<bool> {
        let doc = document.ok_or_else(|| {
            IronBaseError::InvalidQuery("$expr operator requires document context".to_string())
        })?;

        evaluate_expr(filter_value, doc)
    }
}
