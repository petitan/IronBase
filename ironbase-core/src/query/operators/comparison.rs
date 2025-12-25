// src/query/operators/comparison.rs
// Comparison operators: $eq, $ne, $gt, $gte, $lt, $lte

use crate::document::Document;
use crate::error::Result;
use serde_json::Value;

use super::helpers::compare_with_predicate;
use super::traits::OperatorMatcher;

/// $eq operator: Matches values that are equal to a specified value
///
/// # MongoDB Spec
///
/// ```json
/// { field: { $eq: value } }
/// // Shorthand: { field: value }
/// ```
///
/// # Complexity: CC = 2
pub struct EqOperator;

impl OperatorMatcher for EqOperator {
    fn name(&self) -> &'static str {
        "$eq"
    }

    fn matches(
        &self,
        doc_value: Option<&Value>,
        filter_value: &Value,
        _document: Option<&Document>,
    ) -> Result<bool> {
        match doc_value {
            None => Ok(false),
            Some(v) => {
                // Direct equality check
                if v == filter_value {
                    return Ok(true);
                }
                // MongoDB array element matching: if doc_value is an array,
                // check if any element equals filter_value
                if let Value::Array(arr) = v {
                    Ok(arr.iter().any(|elem| elem == filter_value))
                } else {
                    Ok(false)
                }
            }
        }
    }
}

/// $ne operator: Matches values that are not equal to a specified value
///
/// # MongoDB Spec
///
/// ```json
/// { field: { $ne: value } }
/// ```
///
/// **Note**: Returns true if field doesn't exist
///
/// # Complexity: CC = 2
pub struct NeOperator;

impl OperatorMatcher for NeOperator {
    fn name(&self) -> &'static str {
        "$ne"
    }

    fn matches(
        &self,
        doc_value: Option<&Value>,
        filter_value: &Value,
        _document: Option<&Document>,
    ) -> Result<bool> {
        match doc_value {
            None => Ok(true), // Field doesn't exist - not equal
            Some(v) => {
                // Direct inequality check
                if v == filter_value {
                    return Ok(false);
                }
                // MongoDB array element matching: if doc_value is an array,
                // return false if ANY element equals filter_value
                if let Value::Array(arr) = v {
                    Ok(!arr.iter().any(|elem| elem == filter_value))
                } else {
                    Ok(true)
                }
            }
        }
    }
}

/// $gt operator: Matches values that are greater than a specified value
///
/// # MongoDB Spec
///
/// ```json
/// { field: { $gt: value } }
/// ```
///
/// # Complexity: CC = 3
pub struct GtOperator;

impl OperatorMatcher for GtOperator {
    fn name(&self) -> &'static str {
        "$gt"
    }

    fn matches(
        &self,
        doc_value: Option<&Value>,
        filter_value: &Value,
        _document: Option<&Document>,
    ) -> Result<bool> {
        compare_with_predicate(doc_value, filter_value, |ord| {
            ord == std::cmp::Ordering::Greater
        })
    }
}

/// $gte operator: Matches values that are greater than or equal to a specified value
///
/// # Complexity: CC = 4
pub struct GteOperator;

impl OperatorMatcher for GteOperator {
    fn name(&self) -> &'static str {
        "$gte"
    }

    fn matches(
        &self,
        doc_value: Option<&Value>,
        filter_value: &Value,
        _document: Option<&Document>,
    ) -> Result<bool> {
        compare_with_predicate(doc_value, filter_value, |ord| {
            matches!(ord, std::cmp::Ordering::Greater | std::cmp::Ordering::Equal)
        })
    }
}

/// $lt operator: Matches values that are less than a specified value
///
/// # Complexity: CC = 3
pub struct LtOperator;

impl OperatorMatcher for LtOperator {
    fn name(&self) -> &'static str {
        "$lt"
    }

    fn matches(
        &self,
        doc_value: Option<&Value>,
        filter_value: &Value,
        _document: Option<&Document>,
    ) -> Result<bool> {
        compare_with_predicate(doc_value, filter_value, |ord| {
            ord == std::cmp::Ordering::Less
        })
    }
}

/// $lte operator: Matches values that are less than or equal to a specified value
///
/// # Complexity: CC = 4
pub struct LteOperator;

impl OperatorMatcher for LteOperator {
    fn name(&self) -> &'static str {
        "$lte"
    }

    fn matches(
        &self,
        doc_value: Option<&Value>,
        filter_value: &Value,
        _document: Option<&Document>,
    ) -> Result<bool> {
        compare_with_predicate(doc_value, filter_value, |ord| {
            matches!(ord, std::cmp::Ordering::Less | std::cmp::Ordering::Equal)
        })
    }
}
