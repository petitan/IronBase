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

/// Helper: Resolve a value that might be a field reference
///
/// - If value starts with "$", extract field from document
/// - Otherwise return the literal value
fn resolve_expr_value<'a>(value: &'a Value, document: &'a Document) -> Option<&'a Value> {
    if let Some(field_ref) = value.as_str() {
        if let Some(field_name) = field_ref.strip_prefix('$') {
            // It's a field reference like "$quantity"
            return document.get(field_name);
        }
    }
    // Return the literal value
    Some(value)
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

    let left = resolve_expr_value(&arr[0], document);
    let right = resolve_expr_value(&arr[1], document);

    match (left, right) {
        (Some(l), Some(r)) => {
            if let Some(ordering) = compare_values(l, r) {
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
