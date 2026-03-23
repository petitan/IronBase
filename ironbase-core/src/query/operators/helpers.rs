// src/query/operators/helpers.rs
// Helper functions for operators

use crate::error::{IronBaseError, Result};
use crate::value_utils::compare_values;
use serde_json::Value;

use super::text_search::regex_match_with_options;

// ============================================================================
// $regex + $options helpers (shared by filter.rs and array.rs)
// ============================================================================

/// Parsed $regex + $options pair from a filter object.
pub(crate) struct ParsedRegexFilter<'a> {
    pub pattern: &'a str,
    pub options: &'a str,
}

/// Parse `$regex` + `$options` from a filter/condition object.
///
/// Returns:
/// - `Ok(Some(parsed))` if `$regex` is present (with or without `$options`)
/// - `Ok(None)` if neither `$regex` nor `$options` is present
/// - `Err(...)` if `$options` is present without `$regex`, or `$regex` is not a string
pub(crate) fn parse_regex_filter(
    obj: &serde_json::Map<String, Value>,
) -> Result<Option<ParsedRegexFilter<'_>>> {
    let has_regex = obj.contains_key("$regex");
    let has_options = obj.contains_key("$options");

    if !has_regex && !has_options {
        return Ok(None);
    }

    if has_options && !has_regex {
        return Err(IronBaseError::InvalidQuery(
            "$options requires $regex".to_string(),
        ));
    }

    let pattern = obj.get("$regex").and_then(|v| v.as_str()).ok_or_else(|| {
        IronBaseError::InvalidQuery("$regex requires a string pattern".to_string())
    })?;
    let options = obj.get("$options").and_then(|v| v.as_str()).unwrap_or("");

    Ok(Some(ParsedRegexFilter { pattern, options }))
}

/// Check if a single value matches a regex pattern with options.
///
/// Handles `String`, `Array` (any string element match), and other types (always false).
pub(crate) fn regex_matches_value(
    value: Option<&Value>,
    pattern: &str,
    options: &str,
) -> Result<bool> {
    match value {
        Some(Value::String(s)) => regex_match_with_options(s, pattern, options),
        Some(Value::Array(arr)) => {
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
}

/// Generic comparison helper for $gt, $gte, $lt, $lte operators
///
/// Handles both direct comparison and MongoDB array element matching.
/// The predicate function determines which orderings are considered a match.
pub fn compare_with_predicate<F>(
    doc_value: Option<&Value>,
    filter_value: &Value,
    predicate: F,
) -> Result<bool>
where
    F: Fn(std::cmp::Ordering) -> bool,
{
    match doc_value {
        None => Ok(false),
        Some(v) => {
            // Direct comparison
            if let Some(ordering) = compare_values(v, filter_value) {
                if predicate(ordering) {
                    return Ok(true);
                }
            }
            // MongoDB array element matching
            if let Value::Array(arr) = v {
                Ok(arr.iter().any(|elem| {
                    compare_values(elem, filter_value)
                        .map(&predicate)
                        .unwrap_or(false)
                }))
            } else {
                Ok(false)
            }
        }
    }
}
