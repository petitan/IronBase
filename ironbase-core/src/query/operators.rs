// src/query/operators.rs
//! Query operator trait definitions and implementations
//!
//! This module implements the Strategy pattern for MongoDB query operators.
//! Each operator is implemented as a separate type that implements the `OperatorMatcher` trait.
//!
//! # Architecture
//!
//! ```text
//! OperatorMatcher trait
//!     ↓
//! ┌────────────────┬────────────────┬────────────────┐
//! │ Comparison     │ Logical        │ Element        │
//! │ ($eq, $gt...)  │ ($and, $or...) │ ($exists...)   │
//! └────────────────┴────────────────┴────────────────┘
//! ```
//!
//! # Benefits
//!
//! - **Extensibility**: Add new operators without modifying existing code
//! - **Testability**: Each operator can be tested independently
//! - **Reduced Complexity**: Each operator has CC ~2-4 instead of one giant function
//! - **Type Safety**: Compile-time guarantees for operator implementations

use crate::document::Document;
use crate::error::{MongoLiteError, Result};
use lazy_static::lazy_static;
use serde_json::Value;
use std::collections::HashMap;

// ============================================================================
// TRAIT DEFINITION
// ============================================================================

/// Trait for all query operators
///
/// Each MongoDB query operator ($eq, $gt, $and, etc.) implements this trait.
/// The trait provides a uniform interface for matching documents against filter criteria.
///
/// # Examples
///
/// ```rust
/// use serde_json::json;
/// use ironbase_core::query::operators::EqOperator;
/// use ironbase_core::query::operators::OperatorMatcher;
///
/// let eq_op = EqOperator;
/// let matches = eq_op.matches(Some(&json!("Alice")), &json!("Alice"), None).unwrap();
/// assert!(matches);
/// ```
pub trait OperatorMatcher: Send + Sync {
    /// Returns the operator name (e.g., "$eq", "$gt", "$and")
    fn name(&self) -> &'static str;

    /// Checks if a document value matches the filter criteria
    ///
    /// # Arguments
    ///
    /// - `doc_value`: The value from the document field (None if field doesn't exist)
    /// - `filter_value`: The expected value from the query filter
    /// - `document`: Optional reference to the full document (for logical operators that recurse)
    ///
    /// # Returns
    ///
    /// - `Ok(true)` if the document matches
    /// - `Ok(false)` if the document doesn't match
    /// - `Err(...)` if there's a validation error (e.g., wrong type for operator)
    fn matches(
        &self,
        doc_value: Option<&Value>,
        filter_value: &Value,
        document: Option<&Document>,
    ) -> Result<bool>;
}

// ============================================================================
// COMPARISON OPERATORS
// ============================================================================

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
        Ok(doc_value.map_or(false, |v| v == filter_value))
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
        Ok(doc_value.map_or(true, |v| v != filter_value))
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
        match doc_value {
            None => Ok(false),
            Some(v) => {
                let ordering = compare_values(v, filter_value);
                Ok(ordering == Some(std::cmp::Ordering::Greater))
            }
        }
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
        match doc_value {
            None => Ok(false),
            Some(v) => {
                let ordering = compare_values(v, filter_value);
                Ok(matches!(
                    ordering,
                    Some(std::cmp::Ordering::Greater | std::cmp::Ordering::Equal)
                ))
            }
        }
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
        match doc_value {
            None => Ok(false),
            Some(v) => {
                let ordering = compare_values(v, filter_value);
                Ok(ordering == Some(std::cmp::Ordering::Less))
            }
        }
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
        match doc_value {
            None => Ok(false),
            Some(v) => {
                let ordering = compare_values(v, filter_value);
                Ok(matches!(
                    ordering,
                    Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)
                ))
            }
        }
    }
}

// ============================================================================
// ARRAY OPERATORS
// ============================================================================

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
                if let Value::Array(arr) = filter_value {
                    Ok(arr.contains(v))
                } else {
                    Err(MongoLiteError::InvalidQuery(
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
        if let Value::Array(arr) = filter_value {
            Ok(doc_value.map_or(true, |v| !arr.contains(v)))
        } else {
            Err(MongoLiteError::InvalidQuery(
                "$nin operator requires an array".to_string(),
            ))
        }
    }
}

// ============================================================================
// ELEMENT OPERATORS
// ============================================================================

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
            Err(MongoLiteError::InvalidQuery(
                "$exists operator requires a boolean".to_string(),
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
                    Err(MongoLiteError::InvalidQuery(
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
/// # Complexity: CC = 6
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
                    // Create a temporary document from the array element
                    if let Value::Object(obj) = elem {
                        // Check if this element matches all conditions
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
                    }
                }
                Ok(false)
            }
            Some(_) => Ok(false), // Not an array
        }
    }
}

/// $regex operator: Provides regular expression capabilities for pattern matching strings
///
/// # MongoDB Spec
///
/// ```json
/// { field: { $regex: "pattern" } }
/// { field: { $regex: "pattern", $options: "i" } }
/// ```
///
/// # Complexity: CC = 5
pub struct RegexOperator;

impl OperatorMatcher for RegexOperator {
    fn name(&self) -> &'static str {
        "$regex"
    }

    fn matches(
        &self,
        doc_value: Option<&Value>,
        filter_value: &Value,
        _document: Option<&Document>,
    ) -> Result<bool> {
        match doc_value {
            None => Ok(false),
            Some(Value::String(s)) => {
                if let Value::String(pattern) = filter_value {
                    // FEATURE: Full regex support (requires regex crate)
                    //
                    // Current: Simple substring matching (field.contains(pattern))
                    // Missing: Regex anchors (^, $), character classes ([a-z]), quantifiers (+, *, ?), etc.
                    //
                    // Implementation:
                    // 1. Add dependency: regex = "1.10" to Cargo.toml
                    // 2. Replace with: Regex::new(pattern)?.is_match(s)
                    // 3. Cache compiled regexes (Regex::new is expensive!)
                    //    - LRU cache: HashMap<String, Regex> with 100 entry limit
                    //
                    // Trade-offs:
                    // - Binary size: +500KB (regex crate)
                    // - Performance: 2-10x slower than substring matching
                    // - Compatibility: Full MongoDB $regex compatibility
                    //
                    // Priority: Low (substring matching covers 80% of use cases)
                    Ok(s.contains(pattern.as_str()))
                } else {
                    Err(MongoLiteError::InvalidQuery(
                        "$regex operator requires a string pattern".to_string(),
                    ))
                }
            }
            Some(_) => Ok(false), // Not a string
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
                        Some(1) => "double",
                        Some(2) => "string",
                        Some(3) => "object",
                        Some(4) => "array",
                        Some(8) => "bool",
                        Some(10) => "null",
                        Some(16) => "int",
                        Some(18) => "long",
                        _ => {
                            return Err(MongoLiteError::InvalidQuery(format!(
                                "Unknown BSON type number: {}",
                                n
                            )))
                        }
                    }
                } else {
                    return Err(MongoLiteError::InvalidQuery(
                        "$type operator requires a string or number".to_string(),
                    ));
                };

                let matches = match type_name {
                    "double" | "number" => val.is_number(),
                    "string" => val.is_string(),
                    "object" => val.is_object(),
                    "array" => val.is_array(),
                    "bool" | "boolean" => val.is_boolean(),
                    "null" => val.is_null(),
                    "int" | "long" => val.is_i64() || val.is_u64(),
                    _ => {
                        return Err(MongoLiteError::InvalidQuery(format!(
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

// ============================================================================
// LOGICAL OPERATORS
// ============================================================================

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
            MongoLiteError::InvalidQuery("$and operator requires document context".to_string())
        })?;

        if let Value::Array(conditions) = filter_value {
            for condition in conditions {
                // Recursively evaluate each condition
                if !matches_filter(doc, condition)? {
                    return Ok(false);
                }
            }
            Ok(true)
        } else {
            Err(MongoLiteError::InvalidQuery(
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
            MongoLiteError::InvalidQuery("$or operator requires document context".to_string())
        })?;

        if let Value::Array(conditions) = filter_value {
            for condition in conditions {
                // If any condition matches, return true
                if matches_filter(doc, condition)? {
                    return Ok(true);
                }
            }
            Ok(false)
        } else {
            Err(MongoLiteError::InvalidQuery(
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
            MongoLiteError::InvalidQuery("$nor operator requires document context".to_string())
        })?;

        if let Value::Array(conditions) = filter_value {
            for condition in conditions {
                // If any condition matches, return false
                if matches_filter(doc, condition)? {
                    return Ok(false);
                }
            }
            Ok(true)
        } else {
            Err(MongoLiteError::InvalidQuery(
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
            Err(MongoLiteError::InvalidQuery(
                "$not operator requires document context".to_string(),
            ))
        }
    }
}

// ============================================================================
// OPERATOR REGISTRY
// ============================================================================

lazy_static! {
    /// Global registry of all query operators
    ///
    /// This registry allows dynamic dispatch to the appropriate operator implementation
    /// based on the operator name string (e.g., "$eq", "$gt").
    ///
    /// # Thread Safety
    ///
    /// The registry is initialized once at program startup and is immutable thereafter.
    /// All operator implementations are required to be `Send + Sync`.
    pub static ref OPERATOR_REGISTRY: HashMap<&'static str, Box<dyn OperatorMatcher>> = {
        let mut registry: HashMap<&'static str, Box<dyn OperatorMatcher>> = HashMap::new();

        // Comparison operators
        registry.insert("$eq", Box::new(EqOperator));
        registry.insert("$ne", Box::new(NeOperator));
        registry.insert("$gt", Box::new(GtOperator));
        registry.insert("$gte", Box::new(GteOperator));
        registry.insert("$lt", Box::new(LtOperator));
        registry.insert("$lte", Box::new(LteOperator));

        // Array operators
        registry.insert("$in", Box::new(InOperator));
        registry.insert("$nin", Box::new(NinOperator));
        registry.insert("$all", Box::new(AllOperator));
        registry.insert("$elemMatch", Box::new(ElemMatchOperator));

        // Element operators
        registry.insert("$exists", Box::new(ExistsOperator));
        registry.insert("$type", Box::new(TypeOperator));

        // Regex operators
        registry.insert("$regex", Box::new(RegexOperator));

        // Logical operators
        registry.insert("$and", Box::new(AndOperator));
        registry.insert("$or", Box::new(OrOperator));
        registry.insert("$nor", Box::new(NorOperator));
        registry.insert("$not", Box::new(NotOperator));

        registry
    };
}

// ============================================================================
// HELPER FUNCTIONS
// ============================================================================

/// Compares two JSON values for ordering
///
/// # Returns
///
/// - `Some(Ordering)` if values are comparable
/// - `None` if values are incompatible types
///
/// # Complexity: CC = 5
fn compare_values(a: &Value, b: &Value) -> Option<std::cmp::Ordering> {
    match (a, b) {
        (Value::Number(n1), Value::Number(n2)) => {
            let f1 = n1.as_f64()?;
            let f2 = n2.as_f64()?;
            f1.partial_cmp(&f2)
        }
        (Value::String(s1), Value::String(s2)) => Some(s1.cmp(s2)),
        (Value::Bool(b1), Value::Bool(b2)) => Some(b1.cmp(b2)),
        _ => None,
    }
}

/// Matches a single filter value against a document value
///
/// This is used by $not and other operators that need to recursively evaluate conditions
///
/// # Complexity: CC = 6
fn matches_filter_value(
    doc_value: Option<&Value>,
    filter_value: &Value,
    document: Option<&Document>,
) -> Result<bool> {
    // If filter is an object with operators, evaluate them
    if let Value::Object(filter_obj) = filter_value {
        for (op_name, op_value) in filter_obj {
            if op_name.starts_with('$') {
                // Look up operator in registry
                if let Some(operator) = OPERATOR_REGISTRY.get(op_name.as_str()) {
                    if !operator.matches(doc_value, op_value, document)? {
                        return Ok(false);
                    }
                } else {
                    return Err(MongoLiteError::InvalidQuery(format!(
                        "Unknown operator: {}",
                        op_name
                    )));
                }
            } else {
                // Field-level condition (shouldn't happen in this context)
                return Err(MongoLiteError::InvalidQuery(
                    "Unexpected field in filter value".to_string(),
                ));
            }
        }
        Ok(true)
    } else {
        // Direct value comparison (implicit $eq)
        Ok(doc_value.map_or(false, |v| v == filter_value))
    }
}

/// Main entry point for filter matching
///
/// This function has been simplified to CC ~8 (down from original 67+)
///
/// # Arguments
///
/// - `document`: The document to match against
/// - `filter`: The query filter (MongoDB JSON format)
///
/// # Returns
///
/// - `Ok(true)` if document matches filter
/// - `Ok(false)` if document doesn't match
/// - `Err(...)` if filter is malformed
///
/// # Complexity: CC = 8 (was 67+)
pub fn matches_filter(document: &Document, filter: &Value) -> Result<bool> {
    // Empty filter matches all documents
    if filter.as_object().map(|o| o.is_empty()).unwrap_or(false) {
        return Ok(true);
    }

    let filter_obj = filter
        .as_object()
        .ok_or_else(|| MongoLiteError::InvalidQuery("Filter must be an object".to_string()))?;

    for (key, value) in filter_obj {
        if key.starts_with('$') {
            // Top-level logical operator
            if let Some(operator) = OPERATOR_REGISTRY.get(key.as_str()) {
                if !operator.matches(None, value, Some(document))? {
                    return Ok(false);
                }
            } else {
                return Err(MongoLiteError::InvalidQuery(format!(
                    "Unknown operator: {}",
                    key
                )));
            }
        } else {
            // Field-level condition
            let doc_value = document.get(key);

            if let Value::Object(condition_obj) = value {
                // Field has operators like { age: { $gt: 18 } }
                for (op_name, op_value) in condition_obj {
                    if op_name.starts_with('$') {
                        if let Some(operator) = OPERATOR_REGISTRY.get(op_name.as_str()) {
                            if !operator.matches(doc_value, op_value, Some(document))? {
                                return Ok(false);
                            }
                        } else {
                            return Err(MongoLiteError::InvalidQuery(format!(
                                "Unknown operator: {}",
                                op_name
                            )));
                        }
                    }
                }
            } else {
                // Direct equality check like { name: "Alice" }
                if doc_value != Some(value) {
                    return Ok(false);
                }
            }
        }
    }

    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::DocumentId;
    use serde_json::json;
    use std::collections::HashMap as StdHashMap;

    fn create_test_document(id: i64, fields: Vec<(&str, Value)>) -> Document {
        let mut field_map = StdHashMap::new();
        for (k, v) in fields {
            field_map.insert(k.to_string(), v);
        }
        Document::new(DocumentId::Int(id), field_map)
    }

    #[test]
    fn test_eq_operator() {
        let op = EqOperator;
        assert!(op
            .matches(Some(&json!("Alice")), &json!("Alice"), None)
            .unwrap());
        assert!(!op
            .matches(Some(&json!("Bob")), &json!("Alice"), None)
            .unwrap());
        assert!(!op.matches(None, &json!("Alice"), None).unwrap());
    }

    #[test]
    fn test_ne_operator() {
        let op = NeOperator;
        assert!(op
            .matches(Some(&json!("Bob")), &json!("Alice"), None)
            .unwrap());
        assert!(!op
            .matches(Some(&json!("Alice")), &json!("Alice"), None)
            .unwrap());
        assert!(op.matches(None, &json!("Alice"), None).unwrap()); // Missing field != value
    }

    #[test]
    fn test_gt_operator() {
        let op = GtOperator;
        assert!(op.matches(Some(&json!(10)), &json!(5), None).unwrap());
        assert!(!op.matches(Some(&json!(5)), &json!(10), None).unwrap());
        assert!(!op.matches(Some(&json!(5)), &json!(5), None).unwrap());
    }

    #[test]
    fn test_in_operator() {
        let op = InOperator;
        let array = json!(["NYC", "LA", "SF"]);
        assert!(op.matches(Some(&json!("NYC")), &array, None).unwrap());
        assert!(!op.matches(Some(&json!("Chicago")), &array, None).unwrap());
    }

    #[test]
    fn test_exists_operator() {
        let op = ExistsOperator;
        assert!(op
            .matches(Some(&json!("value")), &json!(true), None)
            .unwrap());
        assert!(!op.matches(None, &json!(true), None).unwrap());
        assert!(op.matches(None, &json!(false), None).unwrap());
    }

    #[test]
    fn test_and_operator() {
        let doc = create_test_document(1, vec![("age", json!(25)), ("city", json!("NYC"))]);
        let filter = json!([{"age": {"$gt": 18}}, {"city": "NYC"}]);

        let op = AndOperator;
        assert!(op.matches(None, &filter, Some(&doc)).unwrap());
    }

    #[test]
    fn test_or_operator() {
        let doc = create_test_document(1, vec![("age", json!(15))]);
        let filter = json!([{"age": {"$lt": 18}}, {"age": {"$gt": 65}}]);

        let op = OrOperator;
        assert!(op.matches(None, &filter, Some(&doc)).unwrap());
    }

    #[test]
    fn test_matches_filter_simple() {
        let doc = create_test_document(1, vec![("name", json!("Alice"))]);
        let filter = json!({"name": "Alice"});
        assert!(matches_filter(&doc, &filter).unwrap());
    }

    #[test]
    fn test_matches_filter_with_operators() {
        let doc = create_test_document(1, vec![("age", json!(25))]);
        let filter = json!({"age": {"$gte": 18, "$lt": 30}});
        assert!(matches_filter(&doc, &filter).unwrap());
    }

    #[test]
    fn test_matches_filter_logical_and() {
        let doc = create_test_document(1, vec![("age", json!(25)), ("city", json!("NYC"))]);
        let filter = json!({"$and": [{"age": {"$gte": 18}}, {"city": "NYC"}]});
        assert!(matches_filter(&doc, &filter).unwrap());
    }

    #[test]
    fn test_matches_filter_nested_dot_notation() {
        let doc = create_test_document(
            1,
            vec![
                ("address", json!({"city": "Budapest", "zip": 1111})),
                ("stats", json!({"login_count": 42})),
            ],
        );
        let filter = json!({"address.city": "Budapest", "stats.login_count": {"$gte": 40}});
        assert!(matches_filter(&doc, &filter).unwrap());
    }

    #[test]
    fn test_operator_registry() {
        assert!(OPERATOR_REGISTRY.contains_key("$eq"));
        assert!(OPERATOR_REGISTRY.contains_key("$gt"));
        assert!(OPERATOR_REGISTRY.contains_key("$and"));
        assert!(OPERATOR_REGISTRY.contains_key("$exists"));
        assert!(OPERATOR_REGISTRY.contains_key("$all"));
        assert!(OPERATOR_REGISTRY.contains_key("$elemMatch"));
        assert!(OPERATOR_REGISTRY.contains_key("$type"));
        assert!(OPERATOR_REGISTRY.contains_key("$regex"));
        assert_eq!(OPERATOR_REGISTRY.len(), 17); // Total operators implemented
    }

    #[test]
    fn test_all_operator() {
        let op = AllOperator;
        let doc_value = json!(["apple", "banana", "cherry"]);
        let filter_value = json!(["apple", "banana"]);
        assert!(op.matches(Some(&doc_value), &filter_value, None).unwrap());

        let filter_value_fail = json!(["apple", "grape"]);
        assert!(!op
            .matches(Some(&doc_value), &filter_value_fail, None)
            .unwrap());
    }

    #[test]
    fn test_type_operator() {
        let op = TypeOperator;
        assert!(op
            .matches(Some(&json!("hello")), &json!("string"), None)
            .unwrap());
        assert!(op
            .matches(Some(&json!(42)), &json!("number"), None)
            .unwrap());
        assert!(op.matches(Some(&json!([])), &json!("array"), None).unwrap());
        assert!(!op
            .matches(Some(&json!("hello")), &json!("number"), None)
            .unwrap());
    }

    #[test]
    fn test_regex_operator() {
        let op = RegexOperator;
        assert!(op
            .matches(Some(&json!("hello world")), &json!("world"), None)
            .unwrap());
        assert!(!op
            .matches(Some(&json!("hello world")), &json!("xyz"), None)
            .unwrap());
    }
}
