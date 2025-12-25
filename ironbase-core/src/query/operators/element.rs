// src/query/operators/element.rs
// Element operators: $exists, $type

use crate::document::Document;
use crate::error::{IronBaseError, Result};
use serde_json::Value;

use super::traits::OperatorMatcher;

// BSON type codes (MongoDB specification)
// https://www.mongodb.com/docs/manual/reference/bson-types/
const BSON_TYPE_DOUBLE: i64 = 1;
const BSON_TYPE_STRING: i64 = 2;
const BSON_TYPE_OBJECT: i64 = 3;
const BSON_TYPE_ARRAY: i64 = 4;
const BSON_TYPE_BOOL: i64 = 8;
const BSON_TYPE_NULL: i64 = 10;
const BSON_TYPE_INT32: i64 = 16;
const BSON_TYPE_INT64: i64 = 18;

/// $exists operator: Matches documents that have the specified field
///
/// # MongoDB Spec
///
/// ```json
/// { field: { $exists: true } }  // field must exist
/// { field: { $exists: false } } // field must NOT exist
/// ```
///
/// # Complexity: CC = 4
pub struct ExistsOperator;

impl OperatorMatcher for ExistsOperator {
    fn name(&self) -> &'static str {
        "$exists"
    }

    fn matches(
        &self,
        doc_value: Option<&Value>,
        filter_value: &Value,
        _document: Option<&Document>,
    ) -> Result<bool> {
        if let Value::Bool(should_exist) = filter_value {
            Ok(doc_value.is_some() == *should_exist)
        } else {
            Err(IronBaseError::InvalidQuery(
                "$exists operator requires a boolean".to_string(),
            ))
        }
    }
}

/// $type operator: Selects documents where the value of a field is of the specified BSON type
///
/// # MongoDB Spec
///
/// ```json
/// { field: { $type: "string" } }
/// { field: { $type: 2 } }  // BSON type number
/// ```
///
/// # Complexity: CC = 10
pub struct TypeOperator;

impl OperatorMatcher for TypeOperator {
    fn name(&self) -> &'static str {
        "$type"
    }

    fn matches(
        &self,
        doc_value: Option<&Value>,
        filter_value: &Value,
        _document: Option<&Document>,
    ) -> Result<bool> {
        match doc_value {
            None => Ok(false),
            Some(val) => {
                let type_name = if let Value::String(s) = filter_value {
                    s.as_str()
                } else if let Value::Number(n) = filter_value {
                    // BSON type numbers (simplified, MongoDB has more)
                    match n.as_i64() {
                        Some(BSON_TYPE_DOUBLE) => "double",
                        Some(BSON_TYPE_STRING) => "string",
                        Some(BSON_TYPE_OBJECT) => "object",
                        Some(BSON_TYPE_ARRAY) => "array",
                        Some(BSON_TYPE_BOOL) => "bool",
                        Some(BSON_TYPE_NULL) => "null",
                        Some(BSON_TYPE_INT32) => "int",
                        Some(BSON_TYPE_INT64) => "long",
                        _ => {
                            return Err(IronBaseError::InvalidQuery(format!(
                                "Unknown BSON type number: {}",
                                n
                            )))
                        }
                    }
                } else {
                    return Err(IronBaseError::InvalidQuery(
                        "$type operator requires a string or number".to_string(),
                    ));
                };

                let matches = match type_name {
                    "double" => {
                        // Double should match floating-point numbers only (not integers)
                        // serde_json stores integers as PosInt/NegInt, floats as Float
                        // is_i64()/is_u64() returns false for Float-stored numbers
                        if let Value::Number(n) = val {
                            !n.is_i64() && !n.is_u64()
                        } else {
                            false
                        }
                    }
                    "number" => val.is_number(), // Alias - matches all numeric types
                    "string" => val.is_string(),
                    "object" => val.is_object(),
                    "array" => val.is_array(),
                    "bool" | "boolean" => val.is_boolean(),
                    "null" => val.is_null(),
                    "int" => {
                        // int32: must be integer AND fit in i32 range
                        if let Value::Number(n) = val {
                            if let Some(i) = n.as_i64() {
                                i >= i32::MIN as i64 && i <= i32::MAX as i64
                            } else if let Some(u) = n.as_u64() {
                                u <= i32::MAX as u64
                            } else {
                                false // Float-stored number
                            }
                        } else {
                            false
                        }
                    }
                    "long" => {
                        // int64: must be integer (stored as PosInt/NegInt, not Float)
                        if let Value::Number(n) = val {
                            n.is_i64()
                                || (n.is_u64() && n.as_u64().is_some_and(|u| u <= i64::MAX as u64))
                        } else {
                            false
                        }
                    }
                    _ => {
                        return Err(IronBaseError::InvalidQuery(format!(
                            "Unknown type name: {}",
                            type_name
                        )))
                    }
                };

                Ok(matches)
            }
        }
    }
}
