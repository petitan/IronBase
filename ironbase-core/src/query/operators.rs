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
use crate::value_utils::compare_values;
use lazy_static::lazy_static;
use lru::LruCache;
use regex::Regex;
use serde_json::Value;
use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};
use strsim::{damerau_levenshtein, jaro_winkler, normalized_levenshtein};

// ============================================================================
// REGEX WITH OPTIONS SUPPORT (Full regex crate implementation)
// ============================================================================

/// Maximum number of compiled regex patterns to cache (prevents memory bloat)
const REGEX_CACHE_CAPACITY: usize = 100;

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

lazy_static! {
    /// Global cache for compiled regex patterns
    /// Key format: "pattern:options"
    /// Uses Arc<Regex> for cheap cloning (reference count increment only)
    static ref REGEX_CACHE: Mutex<LruCache<String, Arc<Regex>>> =
        Mutex::new(LruCache::new(NonZeroUsize::new(REGEX_CACHE_CAPACITY).unwrap()));
}

/// Build regex pattern string with MongoDB-style options
///
/// Converts MongoDB options (i, m, s, x) to Rust regex inline flags
fn build_regex_pattern(pattern: &str, options: &str) -> String {
    let mut regex_str = String::new();

    // Handle options - only add prefix if there are valid options
    let valid_options: String = options
        .chars()
        .filter(|c| matches!(c, 'i' | 'm' | 's' | 'x'))
        .collect();

    if !valid_options.is_empty() {
        regex_str.push_str("(?");
        regex_str.push_str(&valid_options);
        regex_str.push(')');
    }

    regex_str.push_str(pattern);
    regex_str
}

/// Get or compile a regex pattern with caching
///
/// Uses an LRU cache to avoid recompiling the same patterns repeatedly.
/// Regex::new() is expensive, so caching provides significant performance benefits.
/// Returns Arc<Regex> for cheap cloning (only increments reference count).
fn get_or_compile_regex(pattern: &str, options: &str) -> Result<Arc<Regex>> {
    let cache_key = format!("{}:{}", pattern, options);

    // Try cache first
    {
        let mut cache = REGEX_CACHE.lock().unwrap();
        if let Some(regex) = cache.get(&cache_key) {
            return Ok(Arc::clone(regex)); // Cheap: just increments ref count
        }
    }

    // Build and compile regex with options
    let regex_pattern = build_regex_pattern(pattern, options);
    let regex = Arc::new(Regex::new(&regex_pattern).map_err(|e| {
        MongoLiteError::InvalidQuery(format!("Invalid regex pattern '{}': {}", pattern, e))
    })?);

    // Store in cache
    {
        let mut cache = REGEX_CACHE.lock().unwrap();
        cache.put(cache_key, Arc::clone(&regex));
    }

    Ok(regex)
}

/// Helper function for regex matching with MongoDB-style options
///
/// Supports:
/// - `i` - Case insensitive matching
/// - `m` - Multiline mode (^ and $ match line boundaries)
/// - `s` - Dotall mode (. matches newlines)
/// - `x` - Extended mode (whitespace ignored, # comments)
///
/// Uses the `regex` crate for full regex support including:
/// - Anchors: ^, $
/// - Character classes: [a-z], [0-9], \d, \w, \s
/// - Quantifiers: +, *, ?, {n}, {n,m}
/// - Alternation: |
/// - Grouping: ()
/// - Word boundaries: \b
fn regex_match_with_options(text: &str, pattern: &str, options: &str) -> Result<bool> {
    let regex = get_or_compile_regex(pattern, options)?;
    Ok(regex.is_match(text))
}

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
                    Err(MongoLiteError::InvalidQuery(
                        "$size operator requires an integer".to_string(),
                    ))
                }
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
/// # Supported features (via regex crate):
/// - Anchors: ^, $
/// - Character classes: [a-z], [0-9], \d, \w, \s
/// - Quantifiers: +, *, ?, {n}, {n,m}
/// - Alternation: |
/// - Grouping: ()
/// - Word boundaries: \b
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
                    // Full regex matching with compiled & cached regex
                    regex_match_with_options(s, pattern, "")
                } else {
                    Err(MongoLiteError::InvalidQuery(
                        "$regex operator requires a string pattern".to_string(),
                    ))
                }
            }
            Some(Value::Array(arr)) => {
                // Check if any string element in the array matches
                if let Value::String(pattern) = filter_value {
                    for elem in arr {
                        if let Value::String(s) = elem {
                            if regex_match_with_options(s, pattern, "")? {
                                return Ok(true);
                            }
                        }
                    }
                    Ok(false)
                } else {
                    Err(MongoLiteError::InvalidQuery(
                        "$regex operator requires a string pattern".to_string(),
                    ))
                }
            }
            Some(_) => Ok(false), // Not a string or array
        }
    }
}

// ============================================================================
// FUZZY TEXT SEARCH OPERATORS
// ============================================================================

/// Default threshold for fuzzy matching (0.0-1.0 scale)
const DEFAULT_FUZZY_THRESHOLD: f64 = 0.8;

/// Supported fuzzy matching algorithms
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FuzzyAlgorithm {
    /// Jaro-Winkler similarity - fast, good for names and short strings
    JaroWinkler,
    /// Levenshtein distance (normalized) - most accurate for edit distance
    Levenshtein,
    /// Damerau-Levenshtein - like Levenshtein but treats transpositions as single edits
    DamerauLevenshtein,
}

impl FuzzyAlgorithm {
    /// Calculate similarity between two strings (returns 0.0-1.0)
    fn similarity(&self, a: &str, b: &str) -> f64 {
        match self {
            FuzzyAlgorithm::JaroWinkler => jaro_winkler(a, b),
            FuzzyAlgorithm::Levenshtein => normalized_levenshtein(a, b),
            FuzzyAlgorithm::DamerauLevenshtein => {
                // Normalize Damerau-Levenshtein distance to 0.0-1.0
                let max_len = a.len().max(b.len());
                if max_len == 0 {
                    return 1.0; // Both empty strings are identical
                }
                let distance = damerau_levenshtein(a, b);
                1.0 - (distance as f64 / max_len as f64)
            }
        }
    }

    /// Parse algorithm name from string
    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "jaro_winkler" | "jarowinkler" | "jaro-winkler" => Ok(FuzzyAlgorithm::JaroWinkler),
            "levenshtein" => Ok(FuzzyAlgorithm::Levenshtein),
            "damerau_levenshtein" | "damerau-levenshtein" | "dameraulevenshtein" => {
                Ok(FuzzyAlgorithm::DamerauLevenshtein)
            }
            _ => Err(MongoLiteError::InvalidQuery(format!(
                "Unknown fuzzy algorithm: '{}'. Supported: jaro_winkler, levenshtein, damerau_levenshtein",
                s
            ))),
        }
    }
}

/// $fuzzy operator: Matches strings using fuzzy similarity algorithms
///
/// # Syntax
///
/// Simple form (uses Jaro-Winkler with 0.8 threshold):
/// ```json
/// { "name": { "$fuzzy": "john" } }
/// ```
///
/// Extended form with options:
/// ```json
/// { "name": { "$fuzzy": { "value": "john", "algorithm": "levenshtein", "threshold": 0.7 } } }
/// ```
///
/// # Supported Algorithms
///
/// - `jaro_winkler` (default): Fast, good for names and short strings
/// - `levenshtein`: Most accurate for edit distance
/// - `damerau_levenshtein`: Treats transpositions (e.g., "teh" → "the") as single edits
///
/// # Threshold
///
/// A value between 0.0 and 1.0. Documents with similarity >= threshold match.
/// Default is 0.8 (80% similarity required).
///
/// # Complexity: CC = 6
pub struct FuzzyOperator;

impl OperatorMatcher for FuzzyOperator {
    fn name(&self) -> &'static str {
        "$fuzzy"
    }

    fn matches(
        &self,
        doc_value: Option<&Value>,
        filter_value: &Value,
        _document: Option<&Document>,
    ) -> Result<bool> {
        // Parse filter value to get search term, algorithm, and threshold
        let (search_term, algorithm, threshold) = parse_fuzzy_filter(filter_value)?;

        match doc_value {
            None => Ok(false),
            Some(Value::String(s)) => {
                let similarity = algorithm.similarity(s, &search_term);
                Ok(similarity >= threshold)
            }
            Some(Value::Array(arr)) => {
                // Check if any string element in the array matches
                for elem in arr {
                    if let Value::String(s) = elem {
                        let similarity = algorithm.similarity(s, &search_term);
                        if similarity >= threshold {
                            return Ok(true);
                        }
                    }
                }
                Ok(false)
            }
            Some(_) => Ok(false), // Not a string or array
        }
    }
}

/// Parse $fuzzy filter value into (search_term, algorithm, threshold)
fn parse_fuzzy_filter(filter_value: &Value) -> Result<(String, FuzzyAlgorithm, f64)> {
    match filter_value {
        // Simple form: { "$fuzzy": "search_term" }
        Value::String(s) => Ok((
            s.clone(),
            FuzzyAlgorithm::JaroWinkler,
            DEFAULT_FUZZY_THRESHOLD,
        )),

        // Extended form: { "$fuzzy": { "value": "search_term", "algorithm": "...", "threshold": 0.8 } }
        Value::Object(obj) => {
            let value = obj
                .get("value")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    MongoLiteError::InvalidQuery(
                        "$fuzzy object form requires 'value' field with string".to_string(),
                    )
                })?
                .to_string();

            let algorithm = match obj.get("algorithm").and_then(|v| v.as_str()) {
                Some(algo_str) => FuzzyAlgorithm::from_str(algo_str)?,
                None => FuzzyAlgorithm::JaroWinkler,
            };

            let threshold = obj
                .get("threshold")
                .and_then(|v| v.as_f64())
                .unwrap_or(DEFAULT_FUZZY_THRESHOLD);

            // Validate threshold
            if !(0.0..=1.0).contains(&threshold) {
                return Err(MongoLiteError::InvalidQuery(format!(
                    "$fuzzy threshold must be between 0.0 and 1.0, got {}",
                    threshold
                )));
            }

            Ok((value, algorithm, threshold))
        }

        _ => Err(MongoLiteError::InvalidQuery(
            "$fuzzy operator requires a string or object with 'value' field".to_string(),
        )),
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
// EXPRESSION OPERATOR ($expr)
// ============================================================================

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
        MongoLiteError::InvalidQuery("$expr expression must be an object".to_string())
    })?;

    // Expression should have exactly one operator
    if expr_obj.len() != 1 {
        return Err(MongoLiteError::InvalidQuery(
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
                MongoLiteError::InvalidQuery("$and in $expr requires an array".to_string())
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
                MongoLiteError::InvalidQuery("$or in $expr requires an array".to_string())
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
                MongoLiteError::InvalidQuery("$not in $expr requires an array".to_string())
            })?;
            if arr.len() != 1 {
                return Err(MongoLiteError::InvalidQuery(
                    "$not in $expr requires exactly one element".to_string(),
                ));
            }
            Ok(!evaluate_expr(&arr[0], document)?)
        }

        _ => Err(MongoLiteError::InvalidQuery(format!(
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
        MongoLiteError::InvalidQuery("Comparison in $expr requires an array".to_string())
    })?;

    if arr.len() != 2 {
        return Err(MongoLiteError::InvalidQuery(
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
            MongoLiteError::InvalidQuery("$expr operator requires document context".to_string())
        })?;

        evaluate_expr(filter_value, doc)
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
        registry.insert("$size", Box::new(SizeOperator));

        // Element operators
        registry.insert("$exists", Box::new(ExistsOperator));
        registry.insert("$type", Box::new(TypeOperator));

        // Regex operators
        registry.insert("$regex", Box::new(RegexOperator));

        // Fuzzy text search operators
        registry.insert("$fuzzy", Box::new(FuzzyOperator));

        // Logical operators
        registry.insert("$and", Box::new(AndOperator));
        registry.insert("$or", Box::new(OrOperator));
        registry.insert("$nor", Box::new(NorOperator));
        registry.insert("$not", Box::new(NotOperator));

        // Expression operators
        registry.insert("$expr", Box::new(ExprOperator));

        registry
    };
}

// ============================================================================
// HELPER FUNCTIONS
// ============================================================================

/// Generic comparison helper for $gt, $gte, $lt, $lte operators
///
/// Handles both direct comparison and MongoDB array element matching.
/// The predicate function determines which orderings are considered a match.
fn compare_with_predicate<F>(
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
                                // MongoDB-style: if we have multiple values, ANY match is success
                                if use_multi_value_matching {
                                    let mut any_match = false;
                                    for dv in &doc_values {
                                        if operator.matches(Some(*dv), op_value, Some(document))? {
                                            any_match = true;
                                            break;
                                        }
                                    }
                                    if !any_match {
                                        return Ok(false);
                                    }
                                } else if !operator.matches(doc_value, op_value, Some(document))? {
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
                                // MongoDB-style: if we have multiple values, ANY match is success
                                if use_multi_value_matching {
                                    let mut any_match = false;
                                    for dv in &doc_values {
                                        if operator.matches(Some(*dv), op_value, Some(document))? {
                                            any_match = true;
                                            break;
                                        }
                                    }
                                    if !any_match {
                                        return Ok(false);
                                    }
                                } else if !operator.matches(doc_value, op_value, Some(document))? {
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
                // MongoDB-style: if we have multiple values, ANY match is success
                if use_multi_value_matching {
                    let mut any_match = false;
                    for dv in &doc_values {
                        if EqOperator.matches(Some(*dv), value, Some(document))? {
                            any_match = true;
                            break;
                        }
                    }
                    if !any_match {
                        return Ok(false);
                    }
                } else if !EqOperator.matches(doc_value, value, Some(document))? {
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

    // ========== Additional comparison operator tests ==========

    #[test]
    fn test_gte_operator() {
        let op = GteOperator;
        assert!(op.matches(Some(&json!(10)), &json!(5), None).unwrap());
        assert!(op.matches(Some(&json!(5)), &json!(5), None).unwrap()); // Equal
        assert!(!op.matches(Some(&json!(3)), &json!(5), None).unwrap());
        assert!(!op.matches(None, &json!(5), None).unwrap()); // Missing field
    }

    #[test]
    fn test_lt_operator() {
        let op = LtOperator;
        assert!(op.matches(Some(&json!(3)), &json!(5), None).unwrap());
        assert!(!op.matches(Some(&json!(5)), &json!(5), None).unwrap()); // Equal
        assert!(!op.matches(Some(&json!(10)), &json!(5), None).unwrap());
        assert!(!op.matches(None, &json!(5), None).unwrap()); // Missing field
    }

    #[test]
    fn test_lte_operator() {
        let op = LteOperator;
        assert!(op.matches(Some(&json!(3)), &json!(5), None).unwrap());
        assert!(op.matches(Some(&json!(5)), &json!(5), None).unwrap()); // Equal
        assert!(!op.matches(Some(&json!(10)), &json!(5), None).unwrap());
        assert!(!op.matches(None, &json!(5), None).unwrap()); // Missing field
    }

    #[test]
    fn test_gt_missing_field() {
        let op = GtOperator;
        assert!(!op.matches(None, &json!(5), None).unwrap());
    }

    #[test]
    fn test_comparison_strings() {
        let op = GtOperator;
        assert!(op.matches(Some(&json!("b")), &json!("a"), None).unwrap());
        assert!(!op.matches(Some(&json!("a")), &json!("b"), None).unwrap());
    }

    #[test]
    fn test_comparison_booleans() {
        let op = GtOperator;
        assert!(op.matches(Some(&json!(true)), &json!(false), None).unwrap());
        assert!(!op.matches(Some(&json!(false)), &json!(true), None).unwrap());
    }

    #[test]
    fn test_comparison_incompatible_types() {
        let op = GtOperator;
        // String vs number - incompatible
        assert!(!op.matches(Some(&json!("10")), &json!(5), None).unwrap());
    }

    // ========== Array operator tests ==========

    #[test]
    fn test_nin_operator() {
        let op = NinOperator;
        let array = json!(["NYC", "LA", "SF"]);
        assert!(op.matches(Some(&json!("Chicago")), &array, None).unwrap());
        assert!(!op.matches(Some(&json!("NYC")), &array, None).unwrap());
        assert!(op.matches(None, &array, None).unwrap()); // Missing field returns true
    }

    #[test]
    fn test_in_missing_field() {
        let op = InOperator;
        let array = json!(["NYC", "LA"]);
        assert!(!op.matches(None, &array, None).unwrap());
    }

    #[test]
    fn test_in_not_array_error() {
        let op = InOperator;
        let result = op.matches(Some(&json!("NYC")), &json!("not an array"), None);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("requires an array"));
    }

    #[test]
    fn test_nin_not_array_error() {
        let op = NinOperator;
        let result = op.matches(Some(&json!("NYC")), &json!("not an array"), None);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("requires an array"));
    }

    #[test]
    fn test_all_missing_field() {
        let op = AllOperator;
        assert!(!op.matches(None, &json!(["a"]), None).unwrap());
    }

    #[test]
    fn test_all_not_array_doc() {
        let op = AllOperator;
        // Doc value is not an array
        assert!(!op
            .matches(Some(&json!("not an array")), &json!(["a"]), None)
            .unwrap());
    }

    #[test]
    fn test_all_not_array_filter_error() {
        let op = AllOperator;
        let result = op.matches(Some(&json!(["a", "b"])), &json!("not an array"), None);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("requires an array"));
    }

    // ========== Element operator tests ==========

    #[test]
    fn test_exists_not_boolean_error() {
        let op = ExistsOperator;
        let result = op.matches(Some(&json!("value")), &json!("not a boolean"), None);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("requires a boolean"));
    }

    #[test]
    fn test_regex_missing_field() {
        let op = RegexOperator;
        assert!(!op.matches(None, &json!("pattern"), None).unwrap());
    }

    #[test]
    fn test_regex_not_string_doc() {
        let op = RegexOperator;
        // Doc value is not a string
        assert!(!op
            .matches(Some(&json!(123)), &json!("pattern"), None)
            .unwrap());
    }

    #[test]
    fn test_regex_not_string_filter_error() {
        let op = RegexOperator;
        let result = op.matches(Some(&json!("hello")), &json!(123), None);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("requires a string pattern"));
    }

    #[test]
    fn test_type_bson_numbers() {
        let op = TypeOperator;
        // BSON type 1 = double
        assert!(op.matches(Some(&json!(1.5)), &json!(1), None).unwrap());
        // BSON type 2 = string
        assert!(op.matches(Some(&json!("hello")), &json!(2), None).unwrap());
        // BSON type 3 = object
        assert!(op.matches(Some(&json!({"a": 1})), &json!(3), None).unwrap());
        // BSON type 4 = array
        assert!(op.matches(Some(&json!([1, 2])), &json!(4), None).unwrap());
        // BSON type 8 = bool
        assert!(op.matches(Some(&json!(true)), &json!(8), None).unwrap());
        // BSON type 10 = null
        assert!(op.matches(Some(&json!(null)), &json!(10), None).unwrap());
        // BSON type 16 = int
        assert!(op.matches(Some(&json!(42)), &json!(16), None).unwrap());
        // BSON type 18 = long
        assert!(op
            .matches(Some(&json!(9223372036854775807_i64)), &json!(18), None)
            .unwrap());
    }

    #[test]
    fn test_type_unknown_bson_number() {
        let op = TypeOperator;
        let result = op.matches(Some(&json!("hello")), &json!(999), None);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Unknown BSON type number"));
    }

    #[test]
    fn test_type_unknown_type_name() {
        let op = TypeOperator;
        let result = op.matches(Some(&json!("hello")), &json!("unknown_type"), None);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Unknown type name"));
    }

    #[test]
    fn test_type_invalid_filter_error() {
        let op = TypeOperator;
        let result = op.matches(Some(&json!("hello")), &json!([1, 2]), None);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("requires a string or number"));
    }

    #[test]
    fn test_type_missing_field() {
        let op = TypeOperator;
        assert!(!op.matches(None, &json!("string"), None).unwrap());
    }

    #[test]
    fn test_type_boolean_alias() {
        let op = TypeOperator;
        assert!(op
            .matches(Some(&json!(true)), &json!("boolean"), None)
            .unwrap());
        assert!(op
            .matches(Some(&json!(false)), &json!("bool"), None)
            .unwrap());
    }

    #[test]
    fn test_type_int_long() {
        let op = TypeOperator;
        assert!(op.matches(Some(&json!(42)), &json!("int"), None).unwrap());
        assert!(op.matches(Some(&json!(42)), &json!("long"), None).unwrap());
    }

    // ========== Logical operator tests ==========

    #[test]
    fn test_nor_operator() {
        let doc = create_test_document(1, vec![("age", json!(25))]);
        // age is not < 18 AND age is not > 65, so $nor should return true
        let filter = json!([{"age": {"$lt": 18}}, {"age": {"$gt": 65}}]);
        let op = NorOperator;
        assert!(op.matches(None, &filter, Some(&doc)).unwrap());
    }

    #[test]
    fn test_nor_operator_fails() {
        let doc = create_test_document(1, vec![("age", json!(15))]);
        // age < 18 is TRUE, so $nor should return false
        let filter = json!([{"age": {"$lt": 18}}, {"age": {"$gt": 65}}]);
        let op = NorOperator;
        assert!(!op.matches(None, &filter, Some(&doc)).unwrap());
    }

    #[test]
    fn test_nor_not_array_error() {
        let doc = create_test_document(1, vec![("age", json!(25))]);
        let op = NorOperator;
        let result = op.matches(None, &json!({"age": 25}), Some(&doc));
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("requires an array"));
    }

    #[test]
    fn test_nor_no_document_error() {
        let op = NorOperator;
        let result = op.matches(None, &json!([{"age": 25}]), None);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("requires document context"));
    }

    #[test]
    fn test_and_not_array_error() {
        let doc = create_test_document(1, vec![("age", json!(25))]);
        let op = AndOperator;
        let result = op.matches(None, &json!({"age": 25}), Some(&doc));
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("requires an array"));
    }

    #[test]
    fn test_and_no_document_error() {
        let op = AndOperator;
        let result = op.matches(None, &json!([{"age": 25}]), None);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("requires document context"));
    }

    #[test]
    fn test_or_not_array_error() {
        let doc = create_test_document(1, vec![("age", json!(25))]);
        let op = OrOperator;
        let result = op.matches(None, &json!({"age": 25}), Some(&doc));
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("requires an array"));
    }

    #[test]
    fn test_or_no_document_error() {
        let op = OrOperator;
        let result = op.matches(None, &json!([{"age": 25}]), None);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("requires document context"));
    }

    #[test]
    fn test_or_no_match() {
        let doc = create_test_document(1, vec![("age", json!(30))]);
        let filter = json!([{"age": {"$lt": 18}}, {"age": {"$gt": 65}}]);
        let op = OrOperator;
        assert!(!op.matches(None, &filter, Some(&doc)).unwrap());
    }

    #[test]
    fn test_and_fails() {
        let doc = create_test_document(1, vec![("age", json!(25)), ("city", json!("LA"))]);
        let filter = json!([{"age": {"$gt": 18}}, {"city": "NYC"}]); // city doesn't match
        let op = AndOperator;
        assert!(!op.matches(None, &filter, Some(&doc)).unwrap());
    }

    #[test]
    fn test_not_operator() {
        let doc = create_test_document(1, vec![("age", json!(25))]);
        let op = NotOperator;
        // $not: { $gt: 30 } should return true for age=25
        let filter = json!({"$gt": 30});
        assert!(op.matches(Some(&json!(25)), &filter, Some(&doc)).unwrap());
    }

    #[test]
    fn test_not_no_document_error() {
        let op = NotOperator;
        let result = op.matches(Some(&json!(25)), &json!({"$gt": 30}), None);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("requires document context"));
    }

    // ========== matches_filter tests ==========

    #[test]
    fn test_matches_filter_empty() {
        let doc = create_test_document(1, vec![("name", json!("Alice"))]);
        let filter = json!({});
        assert!(matches_filter(&doc, &filter).unwrap());
    }

    #[test]
    fn test_matches_filter_unknown_operator() {
        let doc = create_test_document(1, vec![("age", json!(25))]);
        let filter = json!({"age": {"$unknown": 25}});
        let result = matches_filter(&doc, &filter);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Unknown operator"));
    }

    #[test]
    fn test_matches_filter_top_level_unknown_operator() {
        let doc = create_test_document(1, vec![("age", json!(25))]);
        let filter = json!({"$unknown": [{"age": 25}]});
        let result = matches_filter(&doc, &filter);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Unknown operator"));
    }

    #[test]
    fn test_matches_filter_not_object_error() {
        let doc = create_test_document(1, vec![("name", json!("Alice"))]);
        let filter = json!("not an object");
        let result = matches_filter(&doc, &filter);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Filter must be an object"));
    }

    #[test]
    fn test_matches_filter_direct_mismatch() {
        let doc = create_test_document(1, vec![("name", json!("Alice"))]);
        let filter = json!({"name": "Bob"});
        assert!(!matches_filter(&doc, &filter).unwrap());
    }

    // ========== $elemMatch tests ==========

    #[test]
    fn test_elemmatch_operator() {
        let op = ElemMatchOperator;
        let doc_value = json!([
            {"name": "Alice", "age": 25},
            {"name": "Bob", "age": 30}
        ]);
        let filter_value = json!({"name": "Alice", "age": {"$gte": 20}});
        assert!(op.matches(Some(&doc_value), &filter_value, None).unwrap());
    }

    #[test]
    fn test_elemmatch_no_match() {
        let op = ElemMatchOperator;
        let doc_value = json!([
            {"name": "Alice", "age": 15},
            {"name": "Bob", "age": 18}
        ]);
        let filter_value = json!({"name": "Alice", "age": {"$gte": 20}});
        assert!(!op.matches(Some(&doc_value), &filter_value, None).unwrap());
    }

    #[test]
    fn test_elemmatch_missing_field() {
        let op = ElemMatchOperator;
        assert!(!op.matches(None, &json!({"name": "Alice"}), None).unwrap());
    }

    #[test]
    fn test_elemmatch_not_array() {
        let op = ElemMatchOperator;
        assert!(!op
            .matches(
                Some(&json!("not an array")),
                &json!({"name": "Alice"}),
                None
            )
            .unwrap());
    }

    #[test]
    fn test_elemmatch_non_object_elements() {
        let op = ElemMatchOperator;
        let doc_value = json!([1, 2, 3]); // Array of non-objects
        let filter_value = json!({"name": "Alice"});
        assert!(!op.matches(Some(&doc_value), &filter_value, None).unwrap());
    }

    // ========== matches_filter_value tests ==========

    #[test]
    fn test_matches_filter_value_unknown_operator() {
        let result = matches_filter_value(Some(&json!(25)), &json!({"$unknown": 25}), None);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Unknown operator"));
    }

    #[test]
    fn test_matches_filter_value_direct() {
        assert!(matches_filter_value(Some(&json!(25)), &json!(25), None).unwrap());
        assert!(!matches_filter_value(Some(&json!(25)), &json!(30), None).unwrap());
        assert!(!matches_filter_value(None, &json!(25), None).unwrap());
    }

    // ========== Existing tests ==========

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
        assert!(OPERATOR_REGISTRY.contains_key("$fuzzy"));
        assert!(OPERATOR_REGISTRY.contains_key("$expr"));
        assert_eq!(OPERATOR_REGISTRY.len(), 20); // Total operators implemented (19 + $fuzzy)
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

    #[test]
    fn test_size_operator() {
        let op = SizeOperator;

        // Array with 3 elements
        let arr3 = json!(["a", "b", "c"]);
        assert!(op.matches(Some(&arr3), &json!(3), None).unwrap());
        assert!(!op.matches(Some(&arr3), &json!(2), None).unwrap());
        assert!(!op.matches(Some(&arr3), &json!(4), None).unwrap());

        // Empty array
        let empty = json!([]);
        assert!(op.matches(Some(&empty), &json!(0), None).unwrap());
        assert!(!op.matches(Some(&empty), &json!(1), None).unwrap());

        // Non-array value should not match
        let str_val = json!("hello");
        assert!(!op.matches(Some(&str_val), &json!(5), None).unwrap());

        // Missing field should not match
        assert!(!op.matches(None, &json!(0), None).unwrap());
    }

    #[test]
    fn test_size_operator_in_query() {
        // Test matches_filter with $size
        let doc = create_test_document(1, vec![("tags", json!(["a", "b", "c"]))]);
        let filter = json!({"tags": {"$size": 3}});
        assert!(matches_filter(&doc, &filter).unwrap());

        let filter_fail = json!({"tags": {"$size": 2}});
        assert!(!matches_filter(&doc, &filter_fail).unwrap());
    }

    #[test]
    fn test_regex_with_options_case_insensitive() {
        // Test case-insensitive regex matching
        let doc_upper = create_test_document(1, vec![("name", json!("ALICE"))]);
        let doc_lower = create_test_document(2, vec![("name", json!("alice"))]);
        let doc_mixed = create_test_document(3, vec![("name", json!("Alice"))]);
        let doc_other = create_test_document(4, vec![("name", json!("Bob"))]);

        // Case-insensitive query: { name: { $regex: "alice", $options: "i" } }
        let filter_ci = json!({"name": {"$regex": "alice", "$options": "i"}});

        // All "alice" variants should match
        assert!(matches_filter(&doc_upper, &filter_ci).unwrap());
        assert!(matches_filter(&doc_lower, &filter_ci).unwrap());
        assert!(matches_filter(&doc_mixed, &filter_ci).unwrap());

        // "Bob" should not match
        assert!(!matches_filter(&doc_other, &filter_ci).unwrap());
    }

    #[test]
    fn test_regex_with_options_case_sensitive() {
        // Test case-sensitive regex matching (default / no options)
        let doc_lower = create_test_document(1, vec![("name", json!("alice"))]);
        let doc_upper = create_test_document(2, vec![("name", json!("ALICE"))]);

        // Case-sensitive query: { name: { $regex: "alice", $options: "" } }
        let filter_cs = json!({"name": {"$regex": "alice", "$options": ""}});

        // Only lowercase "alice" should match
        assert!(matches_filter(&doc_lower, &filter_cs).unwrap());
        assert!(!matches_filter(&doc_upper, &filter_cs).unwrap());
    }

    #[test]
    fn test_regex_without_options() {
        // Test that $regex alone still works (case-sensitive by default)
        let doc_lower = create_test_document(1, vec![("name", json!("alice"))]);
        let doc_upper = create_test_document(2, vec![("name", json!("ALICE"))]);

        let filter = json!({"name": {"$regex": "alice"}});

        // Should be case-sensitive
        assert!(matches_filter(&doc_lower, &filter).unwrap());
        assert!(!matches_filter(&doc_upper, &filter).unwrap());
    }

    #[test]
    fn test_regex_with_options_on_array() {
        // Test case-insensitive regex on array field
        let doc = create_test_document(1, vec![("tags", json!(["Rust", "PYTHON", "javascript"]))]);

        let filter_rust = json!({"tags": {"$regex": "rust", "$options": "i"}});
        let filter_python = json!({"tags": {"$regex": "python", "$options": "i"}});
        let filter_java = json!({"tags": {"$regex": "java", "$options": "i"}});
        let filter_go = json!({"tags": {"$regex": "go", "$options": "i"}});

        assert!(matches_filter(&doc, &filter_rust).unwrap());
        assert!(matches_filter(&doc, &filter_python).unwrap());
        assert!(matches_filter(&doc, &filter_java).unwrap()); // "javascript" contains "java"
        assert!(!matches_filter(&doc, &filter_go).unwrap());
    }

    // ========================================================================
    // $expr OPERATOR TESTS
    // ========================================================================

    #[test]
    fn test_expr_compare_two_fields() {
        // Test comparing two fields within a document
        // { "$expr": { "$gt": ["$quantity", "$reorderLevel"] } }
        let doc_above = create_test_document(
            1,
            vec![("quantity", json!(100)), ("reorderLevel", json!(50))],
        );
        let doc_below = create_test_document(
            2,
            vec![("quantity", json!(30)), ("reorderLevel", json!(50))],
        );
        let doc_equal = create_test_document(
            3,
            vec![("quantity", json!(50)), ("reorderLevel", json!(50))],
        );

        let filter = json!({"$expr": {"$gt": ["$quantity", "$reorderLevel"]}});

        assert!(matches_filter(&doc_above, &filter).unwrap()); // 100 > 50
        assert!(!matches_filter(&doc_below, &filter).unwrap()); // 30 > 50 is false
        assert!(!matches_filter(&doc_equal, &filter).unwrap()); // 50 > 50 is false
    }

    #[test]
    fn test_expr_compare_field_with_literal() {
        // Test comparing a field with a literal value
        // { "$expr": { "$gte": ["$age", 18] } }
        let doc_adult = create_test_document(1, vec![("age", json!(25))]);
        let doc_teen = create_test_document(2, vec![("age", json!(16))]);
        let doc_exact = create_test_document(3, vec![("age", json!(18))]);

        let filter = json!({"$expr": {"$gte": ["$age", 18]}});

        assert!(matches_filter(&doc_adult, &filter).unwrap()); // 25 >= 18
        assert!(!matches_filter(&doc_teen, &filter).unwrap()); // 16 >= 18 is false
        assert!(matches_filter(&doc_exact, &filter).unwrap()); // 18 >= 18
    }

    #[test]
    fn test_expr_eq_and_ne() {
        // Test $eq and $ne in $expr
        let doc = create_test_document(
            1,
            vec![("a", json!(10)), ("b", json!(10)), ("c", json!(20))],
        );

        let filter_eq = json!({"$expr": {"$eq": ["$a", "$b"]}});
        let filter_ne = json!({"$expr": {"$ne": ["$a", "$c"]}});
        let filter_eq_fail = json!({"$expr": {"$eq": ["$a", "$c"]}});

        assert!(matches_filter(&doc, &filter_eq).unwrap()); // a == b (10 == 10)
        assert!(matches_filter(&doc, &filter_ne).unwrap()); // a != c (10 != 20)
        assert!(!matches_filter(&doc, &filter_eq_fail).unwrap()); // a == c (10 == 20 is false)
    }

    #[test]
    fn test_expr_with_strings() {
        // Test $expr with string comparisons
        let doc = create_test_document(
            1,
            vec![("firstName", json!("Alice")), ("lastName", json!("Smith"))],
        );

        let filter_lt = json!({"$expr": {"$lt": ["$firstName", "$lastName"]}});
        let filter_gt = json!({"$expr": {"$gt": ["$firstName", "$lastName"]}});

        assert!(matches_filter(&doc, &filter_lt).unwrap()); // "Alice" < "Smith" alphabetically
        assert!(!matches_filter(&doc, &filter_gt).unwrap()); // "Alice" > "Smith" is false
    }

    #[test]
    fn test_expr_missing_field() {
        // Test $expr when a field is missing
        let doc = create_test_document(1, vec![("quantity", json!(100))]);

        let filter = json!({"$expr": {"$gt": ["$quantity", "$reorderLevel"]}});

        // Should return false when a field is missing
        assert!(!matches_filter(&doc, &filter).unwrap());
    }

    #[test]
    fn test_expr_nested_logical_operators() {
        // Test nested logical operators in $expr
        // { "$expr": { "$and": [{ "$gt": ["$a", 5] }, { "$lt": ["$a", 10] }] } }
        let doc_in_range = create_test_document(1, vec![("a", json!(7))]);
        let doc_too_low = create_test_document(2, vec![("a", json!(3))]);
        let doc_too_high = create_test_document(3, vec![("a", json!(15))]);

        let filter = json!({
            "$expr": {
                "$and": [
                    {"$gt": ["$a", 5]},
                    {"$lt": ["$a", 10]}
                ]
            }
        });

        assert!(matches_filter(&doc_in_range, &filter).unwrap()); // 7 > 5 AND 7 < 10
        assert!(!matches_filter(&doc_too_low, &filter).unwrap()); // 3 > 5 is false
        assert!(!matches_filter(&doc_too_high, &filter).unwrap()); // 15 < 10 is false
    }

    #[test]
    fn test_expr_or_operator() {
        // Test $or in $expr
        let doc_low = create_test_document(1, vec![("score", json!(20))]);
        let doc_high = create_test_document(2, vec![("score", json!(90))]);
        let doc_mid = create_test_document(3, vec![("score", json!(50))]);

        let filter = json!({
            "$expr": {
                "$or": [
                    {"$lt": ["$score", 30]},
                    {"$gt": ["$score", 80]}
                ]
            }
        });

        assert!(matches_filter(&doc_low, &filter).unwrap()); // 20 < 30
        assert!(matches_filter(&doc_high, &filter).unwrap()); // 90 > 80
        assert!(!matches_filter(&doc_mid, &filter).unwrap()); // 50 is neither
    }

    // ========================================================================
    // FULL REGEX TESTS (regex crate)
    // ========================================================================

    #[test]
    fn test_regex_anchor_start() {
        let doc = create_test_document(1, vec![("name", json!("Alice Smith"))]);
        let filter = json!({"name": {"$regex": "^Alice"}});
        assert!(matches_filter(&doc, &filter).unwrap());

        let filter_fail = json!({"name": {"$regex": "^Smith"}});
        assert!(!matches_filter(&doc, &filter_fail).unwrap());
    }

    #[test]
    fn test_regex_anchor_end() {
        let doc = create_test_document(1, vec![("name", json!("Alice Smith"))]);
        let filter = json!({"name": {"$regex": "Smith$"}});
        assert!(matches_filter(&doc, &filter).unwrap());

        let filter_fail = json!({"name": {"$regex": "Alice$"}});
        assert!(!matches_filter(&doc, &filter_fail).unwrap());
    }

    #[test]
    fn test_regex_anchor_full() {
        let doc = create_test_document(1, vec![("name", json!("Alice"))]);
        let filter = json!({"name": {"$regex": "^Alice$"}});
        assert!(matches_filter(&doc, &filter).unwrap());

        let doc_partial = create_test_document(2, vec![("name", json!("Alice Smith"))]);
        assert!(!matches_filter(&doc_partial, &filter).unwrap());
    }

    #[test]
    fn test_regex_character_class() {
        let doc = create_test_document(1, vec![("email", json!("test@example.com"))]);
        let filter = json!({"email": {"$regex": "[a-z]+@[a-z]+\\.[a-z]+"}});
        assert!(matches_filter(&doc, &filter).unwrap());

        let doc_invalid = create_test_document(2, vec![("email", json!("123@456.789"))]);
        assert!(!matches_filter(&doc_invalid, &filter).unwrap());
    }

    #[test]
    fn test_regex_digit_class() {
        let doc = create_test_document(1, vec![("code", json!("ABC123"))]);
        let filter = json!({"code": {"$regex": "[A-Z]+\\d+"}});
        assert!(matches_filter(&doc, &filter).unwrap());
    }

    #[test]
    fn test_regex_quantifiers() {
        let doc = create_test_document(1, vec![("phone", json!("123-456-7890"))]);
        let filter = json!({"phone": {"$regex": "^\\d{3}-\\d{3}-\\d{4}$"}});
        assert!(matches_filter(&doc, &filter).unwrap());

        let doc_invalid = create_test_document(2, vec![("phone", json!("12-34-5678"))]);
        assert!(!matches_filter(&doc_invalid, &filter).unwrap());
    }

    #[test]
    fn test_regex_alternation() {
        let doc = create_test_document(1, vec![("lang", json!("rust"))]);
        let filter = json!({"lang": {"$regex": "^(python|javascript|rust)$"}});
        assert!(matches_filter(&doc, &filter).unwrap());

        let doc_go = create_test_document(2, vec![("lang", json!("go"))]);
        assert!(!matches_filter(&doc_go, &filter).unwrap());
    }

    #[test]
    fn test_regex_optional_quantifier() {
        let doc1 = create_test_document(1, vec![("color", json!("color"))]);
        let doc2 = create_test_document(2, vec![("color", json!("colour"))]);
        let filter = json!({"color": {"$regex": "colou?r"}});
        assert!(matches_filter(&doc1, &filter).unwrap());
        assert!(matches_filter(&doc2, &filter).unwrap());
    }

    #[test]
    fn test_regex_plus_quantifier() {
        let doc = create_test_document(1, vec![("text", json!("aaaabc"))]);
        let filter = json!({"text": {"$regex": "a+bc"}});
        assert!(matches_filter(&doc, &filter).unwrap());

        let doc_no_a = create_test_document(2, vec![("text", json!("bc"))]);
        assert!(!matches_filter(&doc_no_a, &filter).unwrap());
    }

    #[test]
    fn test_regex_star_quantifier() {
        let doc1 = create_test_document(1, vec![("text", json!("bc"))]);
        let doc2 = create_test_document(2, vec![("text", json!("aaabc"))]);
        let filter = json!({"text": {"$regex": "a*bc"}});
        assert!(matches_filter(&doc1, &filter).unwrap());
        assert!(matches_filter(&doc2, &filter).unwrap());
    }

    #[test]
    fn test_regex_multiline_option() {
        let doc = create_test_document(1, vec![("text", json!("line1\nline2\nline3"))]);
        let filter = json!({"text": {"$regex": "^line2$", "$options": "m"}});
        assert!(matches_filter(&doc, &filter).unwrap());
    }

    #[test]
    fn test_regex_dotall_option() {
        let doc = create_test_document(1, vec![("text", json!("hello\nworld"))]);
        let filter = json!({"text": {"$regex": "hello.world", "$options": "s"}});
        assert!(matches_filter(&doc, &filter).unwrap());

        // Without 's' option, '.' doesn't match newline
        let filter_no_s = json!({"text": {"$regex": "hello.world"}});
        assert!(!matches_filter(&doc, &filter_no_s).unwrap());
    }

    #[test]
    fn test_regex_invalid_pattern_error() {
        let doc = create_test_document(1, vec![("name", json!("Alice"))]);
        let filter = json!({"name": {"$regex": "[unclosed"}});
        let result = matches_filter(&doc, &filter);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid regex"));
    }

    #[test]
    fn test_regex_word_boundary() {
        let doc = create_test_document(1, vec![("text", json!("test testing tested"))]);
        let filter = json!({"text": {"$regex": "\\btest\\b"}});
        assert!(matches_filter(&doc, &filter).unwrap());

        // Should not match "testing" when looking for whole word "test"
        let doc2 = create_test_document(2, vec![("text", json!("testing"))]);
        assert!(!matches_filter(&doc2, &filter).unwrap());
    }

    #[test]
    fn test_regex_whitespace_class() {
        let doc = create_test_document(1, vec![("text", json!("hello world"))]);
        let filter = json!({"text": {"$regex": "hello\\s+world"}});
        assert!(matches_filter(&doc, &filter).unwrap());

        let doc_tabs = create_test_document(2, vec![("text", json!("hello\t\tworld"))]);
        assert!(matches_filter(&doc_tabs, &filter).unwrap());
    }

    #[test]
    fn test_regex_combined_options() {
        let doc = create_test_document(1, vec![("text", json!("HELLO\nworld"))]);
        // Case insensitive + multiline
        let filter = json!({"text": {"$regex": "^hello$", "$options": "im"}});
        assert!(matches_filter(&doc, &filter).unwrap());
    }

    #[test]
    fn test_regex_on_array_with_anchors() {
        let doc = create_test_document(1, vec![("tags", json!(["rust", "python", "javascript"]))]);
        let filter = json!({"tags": {"$regex": "^rust$"}});
        assert!(matches_filter(&doc, &filter).unwrap());

        let filter_not_found = json!({"tags": {"$regex": "^go$"}});
        assert!(!matches_filter(&doc, &filter_not_found).unwrap());
    }

    #[test]
    fn test_regex_cache_reuse() {
        // This test verifies that the same pattern is used multiple times
        // and should benefit from caching (can't directly test cache hit,
        // but we verify correct behavior with repeated use)
        let doc1 = create_test_document(1, vec![("name", json!("Alice"))]);
        let doc2 = create_test_document(2, vec![("name", json!("Bob"))]);
        let doc3 = create_test_document(3, vec![("name", json!("Charlie"))]);

        let filter = json!({"name": {"$regex": "^[A-C]"}});

        assert!(matches_filter(&doc1, &filter).unwrap()); // Alice starts with A
        assert!(matches_filter(&doc2, &filter).unwrap()); // Bob starts with B
        assert!(matches_filter(&doc3, &filter).unwrap()); // Charlie starts with C
    }

    // ========================================================================
    // FUZZY OPERATOR TESTS
    // ========================================================================

    #[test]
    fn test_fuzzy_operator_simple_form() {
        let op = FuzzyOperator;

        // Exact match should pass
        assert!(op
            .matches(Some(&json!("john")), &json!("john"), None)
            .unwrap());

        // Similar strings should pass (jaro_winkler default)
        assert!(op
            .matches(Some(&json!("john")), &json!("jon"), None)
            .unwrap());
        assert!(op
            .matches(Some(&json!("john")), &json!("johnn"), None)
            .unwrap());

        // Very different strings should fail
        assert!(!op
            .matches(Some(&json!("john")), &json!("xyz"), None)
            .unwrap());

        // None value should fail
        assert!(!op.matches(None, &json!("john"), None).unwrap());
    }

    #[test]
    fn test_fuzzy_operator_extended_form() {
        let op = FuzzyOperator;

        // Extended form with explicit algorithm
        let filter = json!({"value": "john", "algorithm": "jaro_winkler", "threshold": 0.8});
        assert!(op.matches(Some(&json!("john")), &filter, None).unwrap());
        assert!(op.matches(Some(&json!("jon")), &filter, None).unwrap());

        // Lower threshold allows more matches
        let filter_low = json!({"value": "john", "threshold": 0.5});
        assert!(op.matches(Some(&json!("jane")), &filter_low, None).unwrap());

        // Higher threshold is stricter
        let filter_high = json!({"value": "john", "threshold": 0.95});
        assert!(op
            .matches(Some(&json!("john")), &filter_high, None)
            .unwrap());
        assert!(!op.matches(Some(&json!("jon")), &filter_high, None).unwrap());
    }

    #[test]
    fn test_fuzzy_operator_algorithms() {
        let op = FuzzyOperator;

        // Levenshtein
        let filter_lev = json!({"value": "john", "algorithm": "levenshtein", "threshold": 0.7});
        assert!(op.matches(Some(&json!("john")), &filter_lev, None).unwrap());
        assert!(op.matches(Some(&json!("jon")), &filter_lev, None).unwrap());

        // Damerau-Levenshtein (good for transpositions)
        let filter_dl =
            json!({"value": "the", "algorithm": "damerau_levenshtein", "threshold": 0.6});
        assert!(op.matches(Some(&json!("teh")), &filter_dl, None).unwrap()); // transposition
    }

    #[test]
    fn test_fuzzy_operator_array_matching() {
        let op = FuzzyOperator;

        // Array should match if any element is similar
        let arr = json!(["alice", "bob", "charlie"]);
        assert!(op.matches(Some(&arr), &json!("bob"), None).unwrap());
        assert!(op.matches(Some(&arr), &json!("bobb"), None).unwrap()); // fuzzy match
        assert!(!op.matches(Some(&arr), &json!("xyz"), None).unwrap());
    }

    #[test]
    fn test_fuzzy_operator_invalid_filter() {
        let op = FuzzyOperator;

        // Invalid filter types
        assert!(op.matches(Some(&json!("john")), &json!(123), None).is_err());

        // Object without value field
        assert!(op
            .matches(Some(&json!("john")), &json!({"threshold": 0.8}), None)
            .is_err());

        // Invalid threshold
        assert!(op
            .matches(
                Some(&json!("john")),
                &json!({"value": "john", "threshold": 1.5}),
                None
            )
            .is_err());

        // Invalid algorithm
        assert!(op
            .matches(
                Some(&json!("john")),
                &json!({"value": "john", "algorithm": "unknown"}),
                None
            )
            .is_err());
    }

    #[test]
    fn test_fuzzy_algorithm_similarity() {
        // Test Jaro-Winkler
        let jw = FuzzyAlgorithm::JaroWinkler;
        assert!(jw.similarity("john", "john") > 0.99);
        assert!(jw.similarity("john", "jon") > 0.85);
        assert!(jw.similarity("john", "johnny") > 0.8);

        // Test Levenshtein
        let lev = FuzzyAlgorithm::Levenshtein;
        assert!(lev.similarity("john", "john") > 0.99);
        assert!(lev.similarity("john", "jon") > 0.7);

        // Test Damerau-Levenshtein
        let dl = FuzzyAlgorithm::DamerauLevenshtein;
        assert!(dl.similarity("the", "teh") > 0.6); // transposition
        assert!(dl.similarity("abc", "abc") > 0.99);
    }

    #[test]
    fn test_fuzzy_with_document_filter() {
        let doc = create_test_document(1, vec![("name", json!("John"))]);

        // Simple form - exact case should match
        let filter = json!({"name": {"$fuzzy": "john"}});
        assert!(matches_filter(&doc, &filter).unwrap());

        // Extended form with lower threshold
        let filter2 = json!({"name": {"$fuzzy": {"value": "jon", "threshold": 0.7}}});
        assert!(matches_filter(&doc, &filter2).unwrap());

        // No match with very different string
        let filter3 = json!({"name": {"$fuzzy": {"value": "xyz", "threshold": 0.5}}});
        assert!(!matches_filter(&doc, &filter3).unwrap());
    }

    // ========================================================================
    // FUZZY OPERATOR WITH NESTED DOCUMENTS TESTS
    // ========================================================================

    #[test]
    fn test_fuzzy_nested_field_simple() {
        // Document with nested structure
        let doc = create_test_document(
            1,
            vec![(
                "user",
                json!({
                    "name": "John Smith",
                    "profile": {
                        "bio": "Software engineer"
                    }
                }),
            )],
        );

        // Fuzzy search on nested field
        let filter = json!({"user.name": {"$fuzzy": "jon smith"}});
        assert!(matches_filter(&doc, &filter).unwrap());

        // Fuzzy search on deeply nested field
        let filter2 = json!({"user.profile.bio": {"$fuzzy": "software enginer"}}); // typo
        assert!(matches_filter(&doc, &filter2).unwrap());
    }

    #[test]
    fn test_fuzzy_nested_field_extended_form() {
        let doc = create_test_document(
            1,
            vec![(
                "contact",
                json!({
                    "email": "john.smith@example.com",
                    "address": {
                        "city": "New York"
                    }
                }),
            )],
        );

        // Extended form with algorithm on nested field
        // "New York" vs "new york" - case insensitive comparison
        let filter = json!({
            "contact.address.city": {
                "$fuzzy": {
                    "value": "new york",
                    "algorithm": "jaro_winkler",
                    "threshold": 0.8
                }
            }
        });
        assert!(matches_filter(&doc, &filter).unwrap());

        // Jaro-Winkler on email field
        let filter2 = json!({
            "contact.email": {
                "$fuzzy": {
                    "value": "john.smth@example.com",  // missing 'i'
                    "algorithm": "jaro_winkler",
                    "threshold": 0.85
                }
            }
        });
        assert!(matches_filter(&doc, &filter2).unwrap());
    }

    #[test]
    fn test_fuzzy_nested_field_no_match() {
        let doc = create_test_document(
            1,
            vec![(
                "company",
                json!({
                    "name": "Acme Corporation"
                }),
            )],
        );

        // Very different string should not match
        let filter = json!({"company.name": {"$fuzzy": "xyz technologies"}});
        assert!(!matches_filter(&doc, &filter).unwrap());

        // Non-existent nested path should not match
        let filter2 = json!({"company.location.city": {"$fuzzy": "test"}});
        assert!(!matches_filter(&doc, &filter2).unwrap());
    }

    #[test]
    fn test_fuzzy_nested_combined_with_other_operators() {
        let doc = create_test_document(
            1,
            vec![
                (
                    "user",
                    json!({
                        "name": "John Smith",
                        "age": 30
                    }),
                ),
                ("status", json!("active")),
            ],
        );

        // Combine fuzzy with $and and other operators
        let filter = json!({
            "$and": [
                {"user.name": {"$fuzzy": "jon smith"}},
                {"user.age": {"$gte": 25}},
                {"status": "active"}
            ]
        });
        assert!(matches_filter(&doc, &filter).unwrap());

        // Combine fuzzy with $or
        let filter2 = json!({
            "$or": [
                {"user.name": {"$fuzzy": "jane doe"}},  // no match
                {"user.name": {"$fuzzy": "john smth"}}  // match
            ]
        });
        assert!(matches_filter(&doc, &filter2).unwrap());
    }

    #[test]
    fn test_fuzzy_nested_three_levels_deep() {
        let doc = create_test_document(
            1,
            vec![(
                "organization",
                json!({
                    "department": {
                        "team": {
                            "leader": "Elizabeth Johnson"
                        }
                    }
                }),
            )],
        );

        // Fuzzy on 4-level deep field
        let filter = json!({
            "organization.department.team.leader": {
                "$fuzzy": {
                    "value": "elisabeth johnsen",  // typos
                    "algorithm": "levenshtein",
                    "threshold": 0.75
                }
            }
        });
        assert!(matches_filter(&doc, &filter).unwrap());
    }
}
