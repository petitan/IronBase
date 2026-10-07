// src/query/operators/comparison.rs
// Comparison operators: $eq, $ne, $gt, $gte, $lt, $lte

use crate::document::Document;
use crate::error::Result;
use serde_json::Value;

use super::helpers::compare_with_predicate;
use super::traits::OperatorMatcher;
use crate::value_utils::values_equal;

/// MongoDB `$eq` semantics shared by `$eq`, `$ne`, `$in` and `$nin`:
/// - a missing field equals `null` (`{f: null}` matches null and missing);
/// - numbers compare by value (`1 == 1.0`), also inside arrays/objects;
/// - an array field matches if the array itself or any element is equal.
pub(crate) fn eq_matches(doc_value: Option<&Value>, filter_value: &Value) -> bool {
    match doc_value {
        None => filter_value.is_null(),
        Some(v) => {
            values_equal(v, filter_value)
                || matches!(v, Value::Array(arr) if arr.iter().any(|e| values_equal(e, filter_value)))
        }
    }
}

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
        Ok(eq_matches(doc_value, filter_value))
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
/// **Note**: the negation of `$eq`: a missing field is not equal to a
/// value, but it IS equal to `null` (`{f: {$ne: null}}` = "has a value").
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
        Ok(!eq_matches(doc_value, filter_value))
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
