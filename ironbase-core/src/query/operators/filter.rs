// src/query/operators/filter.rs
// Main filter matching logic

use crate::document::Document;
use crate::error::{MongoLiteError, Result};
use serde_json::Value;

use super::comparison::EqOperator;
use super::text_search::regex_match_with_options;
use super::traits::OperatorMatcher;
use super::OPERATOR_REGISTRY;

/// Checks if ANY value from a multi-value list matches an operator condition.
/// MongoDB semantics: if ANY value matches, the condition is satisfied.
///
/// # Arguments
/// - `doc_values`: List of values from document (e.g., from array traversal)
/// - `doc_value`: Single value fallback (when doc_values is empty)
/// - `operator`: The operator to apply
/// - `op_value`: The value to compare against
/// - `document`: The full document (for context)
pub(crate) fn check_operator_match(
    doc_values: &[&Value],
    doc_value: Option<&Value>,
    operator: &dyn OperatorMatcher,
    op_value: &Value,
    document: Option<&Document>,
) -> Result<bool> {
    if doc_values.is_empty() {
        // Single value mode
        operator.matches(doc_value, op_value, document)
    } else {
        // Multi-value mode: ANY match is success
        for dv in doc_values {
            if operator.matches(Some(*dv), op_value, document)? {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

/// Matches a single filter value against a document value
///
/// This is used by $not and other operators that need to recursively evaluate conditions
///
/// # Complexity: CC = 6
pub fn matches_filter_value(
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
        Ok(doc_value == Some(filter_value))
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
        // Special handling for $** wildcard operator (must be checked BEFORE regular $ operators)
        if key.starts_with("$**") {
            // This is a $** wildcard query - treat as field-level condition below
        } else if key.starts_with('$') {
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
            continue; // Move to next filter condition after handling operator
        }
        // Field-level condition (including $** wildcard)
        {
            // Field-level condition
            // Check for $** wildcard operator (recursive descent match)
            let doc_values = if key.starts_with("$**.") {
                let field_name = key.strip_prefix("$**.").unwrap();
                // Validate: only simple field name, not a path
                // Note: errors are swallowed by Query::matches() - invalid patterns just don't match
                if field_name.contains('.') {
                    return Err(MongoLiteError::InvalidQuery(format!(
                        "$** wildcard does not support nested paths. Use $**.{} instead of $**.{}",
                        field_name.split('.').next().unwrap(),
                        field_name
                    )));
                }
                document.get_all_by_field_name(field_name)
            } else if key == "$**" {
                return Err(MongoLiteError::InvalidQuery(
                    "$** must be followed by a field name (e.g., $**.fieldName)".to_string(),
                ));
            } else {
                // Use get_all() for MongoDB-style implicit array flattening
                document.get_all(key)
            };

            // If get_all() returns values, check if ANY matches (MongoDB semantics)
            // Otherwise, fall back to traditional get() for backward compat
            // Note: For $** wildcard queries, we don't fall back to get() since the key is not a valid path
            let is_wildcard_query = key.starts_with("$**.");
            let doc_value = if doc_values.is_empty() {
                if is_wildcard_query {
                    None // $** query returned no values - field not found anywhere
                } else {
                    document.get(key)
                }
            } else {
                // For get_all() results, we need to check if ANY value matches
                // We'll do this by trying each value
                None // Will be handled specially below
            };

            // MongoDB-style: if we have multiple values from array traversal,
            // check if ANY of them matches the condition
            let use_multi_value_matching = !doc_values.is_empty();

            if let Value::Object(condition_obj) = value {
                // Special handling for $regex + $options combination
                // MongoDB allows: { field: { $regex: "pattern", $options: "i" } }
                let has_regex = condition_obj.contains_key("$regex");
                let has_options = condition_obj.contains_key("$options");

                if has_regex && has_options {
                    // Handle $regex with $options as a single operation
                    let pattern = condition_obj
                        .get("$regex")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            MongoLiteError::InvalidQuery(
                                "$regex requires a string pattern".to_string(),
                            )
                        })?;
                    let options = condition_obj
                        .get("$options")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");

                    // Helper to check regex match on a single value
                    let check_regex_match = |val: &Value| -> Result<bool> {
                        match val {
                            Value::String(s) => regex_match_with_options(s, pattern, options),
                            Value::Array(arr) => {
                                for v in arr {
                                    if let Value::String(s) = v {
                                        if regex_match_with_options(s, pattern, options)? {
                                            return Ok(true);
                                        }
                                    }
                                }
                                Ok(false)
                            }
                            _ => Ok(false),
                        }
                    };

                    // MongoDB-style: if we have multiple values, ANY match is success
                    let matches = if use_multi_value_matching {
                        let mut found = false;
                        for dv in &doc_values {
                            if check_regex_match(dv)? {
                                found = true;
                                break;
                            }
                        }
                        found
                    } else {
                        match doc_value {
                            Some(v) => check_regex_match(v)?,
                            None => false,
                        }
                    };

                    if !matches {
                        return Ok(false);
                    }

                    // Process remaining operators (excluding $regex and $options)
                    for (op_name, op_value) in condition_obj {
                        if op_name == "$regex" || op_name == "$options" {
                            continue; // Already handled
                        }
                        if op_name.starts_with('$') {
                            if let Some(operator) = OPERATOR_REGISTRY.get(op_name.as_str()) {
                                if !check_operator_match(
                                    &doc_values,
                                    doc_value,
                                    operator.as_ref(),
                                    op_value,
                                    Some(document),
                                )? {
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
                    // Standard operator processing
                    // Field has operators like { age: { $gt: 18 } }
                    for (op_name, op_value) in condition_obj {
                        if op_name.starts_with('$') {
                            if let Some(operator) = OPERATOR_REGISTRY.get(op_name.as_str()) {
                                if !check_operator_match(
                                    &doc_values,
                                    doc_value,
                                    operator.as_ref(),
                                    op_value,
                                    Some(document),
                                )? {
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
                }
            } else {
                // Direct equality check like { name: "Alice" }
                // Use EqOperator for array element matching support
                if !check_operator_match(
                    &doc_values,
                    doc_value,
                    &EqOperator,
                    value,
                    Some(document),
                )? {
                    return Ok(false);
                }
            }
        }
    }

    Ok(true)
}
