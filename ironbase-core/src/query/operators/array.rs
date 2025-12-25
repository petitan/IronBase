// src/query/operators/array.rs
// Array operators: $in, $nin, $all, $elemMatch, $size

use crate::document::Document;
use crate::error::{IronBaseError, Result};
use serde_json::Value;

use super::traits::OperatorMatcher;
use super::OPERATOR_REGISTRY;

/// $in operator: Matches any of the values specified in an array
///
/// # MongoDB Spec
///
/// ```json
/// { field: { $in: [value1, value2, ...] } }
/// ```
///
/// # Complexity: CC = 4
pub struct InOperator;

impl OperatorMatcher for InOperator {
    fn name(&self) -> &'static str {
        "$in"
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
                if let Value::Array(filter_arr) = filter_value {
                    // Direct check: is doc_value in the filter array?
                    if filter_arr.contains(v) {
                        return Ok(true);
                    }
                    // MongoDB array element matching: if doc_value is an array,
                    // check if ANY element of doc_value matches ANY value in filter_arr
                    if let Value::Array(doc_arr) = v {
                        Ok(doc_arr.iter().any(|elem| filter_arr.contains(elem)))
                    } else {
                        Ok(false)
                    }
                } else {
                    Err(IronBaseError::InvalidQuery(
                        "$in operator requires an array".to_string(),
                    ))
                }
            }
        }
    }
}

/// $nin operator: Matches none of the values specified in an array
///
/// # MongoDB Spec
///
/// ```json
/// { field: { $nin: [value1, value2, ...] } }
/// ```
///
/// **Note**: Returns true if field doesn't exist
///
/// # Complexity: CC = 4
pub struct NinOperator;

impl OperatorMatcher for NinOperator {
    fn name(&self) -> &'static str {
        "$nin"
    }

    fn matches(
        &self,
        doc_value: Option<&Value>,
        filter_value: &Value,
        _document: Option<&Document>,
    ) -> Result<bool> {
        if let Value::Array(filter_arr) = filter_value {
            match doc_value {
                None => Ok(true), // Field doesn't exist - not in
                Some(v) => {
                    // Direct check: is doc_value in the filter array?
                    if filter_arr.contains(v) {
                        return Ok(false);
                    }
                    // MongoDB array element matching: if doc_value is an array,
                    // return false if ANY element of doc_value matches ANY value in filter_arr
                    if let Value::Array(doc_arr) = v {
                        Ok(!doc_arr.iter().any(|elem| filter_arr.contains(elem)))
                    } else {
                        Ok(true)
                    }
                }
            }
        } else {
            Err(IronBaseError::InvalidQuery(
                "$nin operator requires an array".to_string(),
            ))
        }
    }
}

/// $all operator: Matches arrays that contain all specified elements
///
/// # MongoDB Spec
///
/// ```json
/// { field: { $all: [value1, value2, ...] } }
/// ```
///
/// # Complexity: CC = 6
pub struct AllOperator;

impl OperatorMatcher for AllOperator {
    fn name(&self) -> &'static str {
        "$all"
    }

    fn matches(
        &self,
        doc_value: Option<&Value>,
        filter_value: &Value,
        _document: Option<&Document>,
    ) -> Result<bool> {
        match doc_value {
            None => Ok(false),
            Some(Value::Array(doc_arr)) => {
                if let Value::Array(required) = filter_value {
                    // All required values must be in the document array
                    Ok(required.iter().all(|req| doc_arr.contains(req)))
                } else {
                    Err(IronBaseError::InvalidQuery(
                        "$all operator requires an array".to_string(),
                    ))
                }
            }
            Some(_) => Ok(false), // Not an array
        }
    }
}

/// $elemMatch operator: Matches documents that contain an array field with at least one element
/// that matches all the specified query criteria
///
/// # MongoDB Spec
///
/// ```json
/// { field: { $elemMatch: { query1, query2, ... } } }
/// ```
///
/// Supports both object arrays and scalar arrays:
/// - Object arrays: `{items: {$elemMatch: {type: "fruit", qty: {$gte: 5}}}}`
/// - Scalar arrays: `{scores: {$elemMatch: {$gt: 80, $lt: 85}}}`
///
/// # Complexity: CC = 8
pub struct ElemMatchOperator;

impl OperatorMatcher for ElemMatchOperator {
    fn name(&self) -> &'static str {
        "$elemMatch"
    }

    fn matches(
        &self,
        doc_value: Option<&Value>,
        filter_value: &Value,
        _document: Option<&Document>,
    ) -> Result<bool> {
        match doc_value {
            None => Ok(false),
            Some(Value::Array(arr)) => {
                // At least one element in the array must match all conditions in filter_value
                for elem in arr {
                    if let Value::Object(obj) = elem {
                        // OBJECT ELEMENT: Check fields within the object
                        let mut matches_all = true;

                        if let Value::Object(conditions) = filter_value {
                            for (key, value) in conditions {
                                let field_value = obj.get(key);

                                // If condition has operators, evaluate them
                                if let Value::Object(op_obj) = value {
                                    for (op_name, op_value) in op_obj {
                                        if op_name.starts_with('$') {
                                            if let Some(operator) =
                                                OPERATOR_REGISTRY.get(op_name.as_str())
                                            {
                                                if !operator.matches(field_value, op_value, None)? {
                                                    matches_all = false;
                                                    break;
                                                }
                                            }
                                        }
                                    }
                                } else {
                                    // Direct equality
                                    if field_value != Some(value) {
                                        matches_all = false;
                                        break;
                                    }
                                }
                            }
                        }

                        if matches_all {
                            return Ok(true);
                        }
                    } else {
                        // SCALAR ELEMENT: Apply operators directly to the element value
                        // E.g., {scores: {$elemMatch: {$gt: 80, $lt: 85}}} with [75, 82, 90]
                        if let Value::Object(conditions) = filter_value {
                            let mut matches_all = true;

                            for (op_name, op_value) in conditions {
                                if op_name.starts_with('$') {
                                    // Operator condition: apply directly to scalar
                                    if let Some(operator) = OPERATOR_REGISTRY.get(op_name.as_str())
                                    {
                                        if !operator.matches(Some(elem), op_value, None)? {
                                            matches_all = false;
                                            break;
                                        }
                                    }
                                } else {
                                    // Non-operator key on scalar - this doesn't make sense
                                    // (e.g., {$elemMatch: {field: value}} on [1, 2, 3])
                                    // Skip this element as it can't match field-based conditions
                                    matches_all = false;
                                    break;
                                }
                            }

                            if matches_all {
                                return Ok(true);
                            }
                        }
                    }
                }
                Ok(false)
            }
            Some(_) => Ok(false), // Not an array
        }
    }
}

/// $size operator: Matches arrays with the specified number of elements
///
/// # MongoDB Spec
///
/// ```json
/// { field: { $size: 3 } }
/// ```
///
/// Matches documents where the array field has exactly 3 elements.
///
/// # Complexity: CC = 4
pub struct SizeOperator;

impl OperatorMatcher for SizeOperator {
    fn name(&self) -> &'static str {
        "$size"
    }

    fn matches(
        &self,
        doc_value: Option<&Value>,
        filter_value: &Value,
        _document: Option<&Document>,
    ) -> Result<bool> {
        match doc_value {
            None => Ok(false),
            Some(Value::Array(arr)) => {
                if let Some(size) = filter_value.as_i64() {
                    Ok(arr.len() as i64 == size)
                } else if let Some(size) = filter_value.as_u64() {
                    Ok(arr.len() as u64 == size)
                } else {
                    Err(IronBaseError::InvalidQuery(
                        "$size operator requires an integer".to_string(),
                    ))
                }
            }
            Some(_) => Ok(false), // Not an array
        }
    }
}
