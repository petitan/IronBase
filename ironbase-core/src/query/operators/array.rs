// src/query/operators/array.rs
// Array operators: $in, $nin, $all, $elemMatch, $size

use crate::document::{Document, DocumentId};
use crate::error::{IronBaseError, Result};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

use super::filter::{matches_filter, matches_filter_value};
use super::traits::OperatorMatcher;

// ============================================================================
// HashableValue - O(1) lookup support for $in/$nin/$all operators
// ============================================================================

/// Wrapper for JSON values that can be hashed for O(1) HashSet lookup.
/// Only primitive types are supported (null, bool, number, string).
/// Arrays and objects fall back to O(n) linear search.
#[derive(Hash, Eq, PartialEq, Clone, Debug)]
enum HashableValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(u64), // f64.to_bits() for hashable float representation
    String(String),
}

/// Convert a JSON Value to a HashableValue for O(1) lookup.
/// Returns None for arrays and objects (not hashable).
fn value_to_hashable(v: &Value) -> Option<HashableValue> {
    match v {
        Value::Null => Some(HashableValue::Null),
        Value::Bool(b) => Some(HashableValue::Bool(*b)),
        Value::Number(n) => {
            // Prefer integer representation for whole numbers
            if let Some(i) = n.as_i64() {
                Some(HashableValue::Int(i))
            } else {
                // Use bit representation for floats (handles NaN, Inf correctly)
                n.as_f64().map(|f| HashableValue::Float(f.to_bits()))
            }
        }
        Value::String(s) => Some(HashableValue::String(s.clone())),
        Value::Array(_) | Value::Object(_) => None, // Not hashable
    }
}

/// Build a HashSet from a JSON array for O(1) lookups.
/// Non-hashable values (arrays, objects) are skipped.
fn build_hash_set(arr: &[Value]) -> HashSet<HashableValue> {
    arr.iter().filter_map(value_to_hashable).collect()
}

/// Check if a value exists in the hash set.
/// Returns false for non-hashable values.
fn hash_set_contains(set: &HashSet<HashableValue>, v: &Value) -> bool {
    value_to_hashable(v)
        .map(|hv| set.contains(&hv))
        .unwrap_or(false)
}

/// Check if a value exists in the filter array.
/// Uses HashSet for O(1) lookup on hashable values,
/// falls back to O(n) linear search for arrays/objects.
fn value_in_array(v: &Value, filter_arr: &[Value], hash_set: &HashSet<HashableValue>) -> bool {
    // Try O(1) HashSet lookup first
    if hash_set_contains(hash_set, v) {
        return true;
    }
    // Fallback to O(n) for non-hashable types (arrays, objects)
    if matches!(v, Value::Array(_) | Value::Object(_)) {
        filter_arr.contains(v)
    } else {
        false
    }
}

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
                    // Build HashSet once for O(1) lookups
                    let hash_set = build_hash_set(filter_arr);

                    // Direct check: is doc_value in the filter array? O(1)
                    if value_in_array(v, filter_arr, &hash_set) {
                        return Ok(true);
                    }
                    // MongoDB array element matching: if doc_value is an array,
                    // check if ANY element of doc_value matches ANY value in filter_arr
                    if let Value::Array(doc_arr) = v {
                        Ok(doc_arr
                            .iter()
                            .any(|elem| value_in_array(elem, filter_arr, &hash_set)))
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
            // Build HashSet once for O(1) lookups
            let hash_set = build_hash_set(filter_arr);

            match doc_value {
                None => Ok(true), // Field doesn't exist - not in
                Some(v) => {
                    // Direct check: is doc_value in the filter array? O(1)
                    if value_in_array(v, filter_arr, &hash_set) {
                        return Ok(false);
                    }
                    // MongoDB array element matching: if doc_value is an array,
                    // return false if ANY element of doc_value matches ANY value in filter_arr
                    if let Value::Array(doc_arr) = v {
                        Ok(!doc_arr
                            .iter()
                            .any(|elem| value_in_array(elem, filter_arr, &hash_set)))
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
                    // MongoDB semantics: `$all: []` matches NO documents.
                    // Without this guard `required.iter().all(...)` would
                    // be vacuously true and silently match every doc that
                    // has any array field (audit #28 finding C).
                    if required.is_empty() {
                        return Ok(false);
                    }

                    // Build HashSet from document array for O(1) lookups
                    let doc_hash_set = build_hash_set(doc_arr);

                    // All required values must be in the document array
                    Ok(required
                        .iter()
                        .all(|req| value_in_array(req, doc_arr, &doc_hash_set)))
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

/// Top-level operators that make an `$elemMatch` body a sub-query on the
/// element as a document instead of a condition on the element value.
const ELEM_MATCH_QUERY_OPERATORS: [&str; 4] = ["$and", "$or", "$nor", "$expr"];

impl ElemMatchOperator {
    /// MongoDB: when every key of the `$elemMatch` body is a (non-logical)
    /// operator, the body is a condition on each element value itself
    /// (`{$elemMatch: {$gt: 80, $lt: 85}}`); otherwise it is a query run
    /// against each element as a document (`{$elemMatch: {a: 1, b: {$gt: 2}}}`).
    fn is_value_condition(conditions: &serde_json::Map<String, Value>) -> bool {
        !conditions.is_empty()
            && conditions
                .keys()
                .all(|k| k.starts_with('$') && !ELEM_MATCH_QUERY_OPERATORS.contains(&k.as_str()))
    }
}

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
        // Validate that filter_value is an object
        let conditions = match filter_value {
            Value::Object(obj) => obj,
            _ => {
                return Err(IronBaseError::InvalidQuery(
                    "$elemMatch requires an object with query conditions".to_string(),
                ))
            }
        };

        let arr = match doc_value {
            Some(Value::Array(arr)) => arr,
            _ => return Ok(false), // Missing or not an array
        };

        if Self::is_value_condition(conditions) {
            // Value condition: every operator applies to the element itself
            // (scalars and sub-documents alike). An empty document is the
            // context, so nested `$not` evaluates instead of erroring.
            let context = Document::new(DocumentId::Int(0), HashMap::new());
            for elem in arr {
                if matches_filter_value(Some(elem), filter_value, Some(&context))? {
                    return Ok(true);
                }
            }
            return Ok(false);
        }

        // Sub-query: each sub-document element is matched as a document, so
        // dotted paths, implicit array matching, deep equality of object
        // values and logical operators behave as in a top-level query.
        // Elements that are not documents cannot match a field condition.
        for elem in arr {
            if let Value::Object(obj) = elem {
                let fields: HashMap<String, Value> =
                    obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                let element_doc = Document::new(DocumentId::Int(0), fields);
                if matches_filter(&element_doc, filter_value)? {
                    return Ok(true);
                }
            }
        }
        Ok(false)
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
                    if size < 0 {
                        return Err(IronBaseError::InvalidQuery(
                            "$size operator does not accept negative values".to_string(),
                        ));
                    }
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
