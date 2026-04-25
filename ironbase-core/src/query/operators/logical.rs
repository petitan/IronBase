// src/query/operators/logical.rs
// Logical operators: $and, $or, $nor, $not

use crate::document::Document;
use crate::error::{IronBaseError, Result};
use serde_json::Value;

use super::filter::{matches_filter, matches_filter_value};
use super::traits::OperatorMatcher;

/// $and operator: Joins query clauses with a logical AND
///
/// # MongoDB Spec
///
/// ```json
/// { $and: [ { condition1 }, { condition2 }, ... ] }
/// ```
///
/// # Complexity: CC = 5 (array validation + iteration)
pub struct AndOperator;

impl OperatorMatcher for AndOperator {
    fn name(&self) -> &'static str {
        "$and"
    }

    fn matches(
        &self,
        _doc_value: Option<&Value>,
        filter_value: &Value,
        document: Option<&Document>,
    ) -> Result<bool> {
        let doc = document.ok_or_else(|| {
            IronBaseError::InvalidQuery("$and operator requires document context".to_string())
        })?;

        if let Value::Array(conditions) = filter_value {
            // MongoDB-compat: `$and: []` is a malformed query
            // ("$and/$or/$nor must be a nonempty array", error code 2).
            // Pre-fix the empty array fell through the for-loop and
            // returned Ok(true), silently matching every document — a
            // data-loss risk on delete_many / update_many with a
            // dynamically-built filter that ended up empty
            // (audit #28 follow-up).
            if conditions.is_empty() {
                return Err(IronBaseError::InvalidQuery(
                    "$and operator requires a nonempty array".to_string(),
                ));
            }
            for condition in conditions {
                // Recursively evaluate each condition
                if !matches_filter(doc, condition)? {
                    return Ok(false);
                }
            }
            Ok(true)
        } else {
            Err(IronBaseError::InvalidQuery(
                "$and operator requires an array".to_string(),
            ))
        }
    }
}

/// $or operator: Joins query clauses with a logical OR
///
/// # MongoDB Spec
///
/// ```json
/// { $or: [ { condition1 }, { condition2 }, ... ] }
/// ```
///
/// # Complexity: CC = 5
pub struct OrOperator;

impl OperatorMatcher for OrOperator {
    fn name(&self) -> &'static str {
        "$or"
    }

    fn matches(
        &self,
        _doc_value: Option<&Value>,
        filter_value: &Value,
        document: Option<&Document>,
    ) -> Result<bool> {
        let doc = document.ok_or_else(|| {
            IronBaseError::InvalidQuery("$or operator requires document context".to_string())
        })?;

        if let Value::Array(conditions) = filter_value {
            // MongoDB-compat: `$or: []` is malformed (audit #28 follow-up).
            if conditions.is_empty() {
                return Err(IronBaseError::InvalidQuery(
                    "$or operator requires a nonempty array".to_string(),
                ));
            }
            for condition in conditions {
                // If any condition matches, return true
                if matches_filter(doc, condition)? {
                    return Ok(true);
                }
            }
            Ok(false)
        } else {
            Err(IronBaseError::InvalidQuery(
                "$or operator requires an array".to_string(),
            ))
        }
    }
}

/// $nor operator: Joins query clauses with a logical NOR
///
/// # MongoDB Spec
///
/// ```json
/// { $nor: [ { condition1 }, { condition2 }, ... ] }
/// ```
///
/// Returns true only if ALL conditions are false
///
/// # Complexity: CC = 5
pub struct NorOperator;

impl OperatorMatcher for NorOperator {
    fn name(&self) -> &'static str {
        "$nor"
    }

    fn matches(
        &self,
        _doc_value: Option<&Value>,
        filter_value: &Value,
        document: Option<&Document>,
    ) -> Result<bool> {
        let doc = document.ok_or_else(|| {
            IronBaseError::InvalidQuery("$nor operator requires document context".to_string())
        })?;

        if let Value::Array(conditions) = filter_value {
            // MongoDB-compat: `$nor: []` is malformed. Same data-loss
            // risk as `$and: []` — empty array would otherwise return
            // Ok(true) and match all docs (audit #28 follow-up).
            if conditions.is_empty() {
                return Err(IronBaseError::InvalidQuery(
                    "$nor operator requires a nonempty array".to_string(),
                ));
            }
            for condition in conditions {
                // If any condition matches, return false
                if matches_filter(doc, condition)? {
                    return Ok(false);
                }
            }
            Ok(true)
        } else {
            Err(IronBaseError::InvalidQuery(
                "$nor operator requires an array".to_string(),
            ))
        }
    }
}

/// $not operator: Inverts the effect of a query expression
///
/// # MongoDB Spec
///
/// ```json
/// { field: { $not: { $gt: 5 } } }
/// ```
///
/// # Complexity: CC = 3
pub struct NotOperator;

impl OperatorMatcher for NotOperator {
    fn name(&self) -> &'static str {
        "$not"
    }

    fn matches(
        &self,
        doc_value: Option<&Value>,
        filter_value: &Value,
        document: Option<&Document>,
    ) -> Result<bool> {
        // $not wraps another operator object like { $not: { $gt: 5 } }
        // We need to evaluate the inner operator and negate the result

        // Create a temporary document with just this field for evaluation
        if document.is_some() {
            // Find the field name by looking for the $not operator in the original filter
            // This is a simplified approach - we evaluate the inner condition
            let result = matches_filter_value(doc_value, filter_value, document)?;
            Ok(!result)
        } else {
            Err(IronBaseError::InvalidQuery(
                "$not operator requires document context".to_string(),
            ))
        }
    }
}
