// src/query/operators/text_search.rs
// Text search operators: $regex, $fuzzy with caching

use crate::document::Document;
use crate::error::{IronBaseError, Result};
use lazy_static::lazy_static;
use lru::LruCache;
use regex::Regex;
use serde_json::Value;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};
use strsim::{damerau_levenshtein, jaro_winkler, normalized_levenshtein};

use super::traits::OperatorMatcher;

// ============================================================================
// REGEX WITH OPTIONS SUPPORT (Full regex crate implementation)
// ============================================================================

// Re-export from central limits module
use crate::limits::REGEX_CACHE_CAPACITY;

lazy_static! {
    /// Global cache for compiled regex patterns
    /// Key format: "pattern:options"
    /// Uses Arc<Regex> for cheap cloning (reference count increment only)
    pub(crate) static ref REGEX_CACHE: Mutex<LruCache<String, Arc<Regex>>> =
        Mutex::new(LruCache::new(NonZeroUsize::new(REGEX_CACHE_CAPACITY).unwrap()));
}

/// Build regex pattern string with MongoDB-style options
///
/// Converts MongoDB options (i, m, s, x) to Rust regex inline flags
pub(crate) fn build_regex_pattern(pattern: &str, options: &str) -> Result<String> {
    let mut regex_str = String::new();

    // Handle options - only add prefix if there are valid options
    let mut valid_options = String::new();
    for option in options.chars() {
        if matches!(option, 'i' | 'm' | 's' | 'x') {
            valid_options.push(option);
        } else {
            return Err(IronBaseError::InvalidQuery(format!(
                "Invalid regex option '{}'",
                option
            )));
        }
    }

    if !valid_options.is_empty() {
        regex_str.push_str("(?");
        regex_str.push_str(&valid_options);
        regex_str.push(')');
    }

    regex_str.push_str(pattern);
    Ok(regex_str)
}

/// Get or compile a regex pattern with caching
///
/// Uses an LRU cache to avoid recompiling the same patterns repeatedly.
/// Regex::new() is expensive, so caching provides significant performance benefits.
/// Returns Arc<Regex> for cheap cloning (only increments reference count).
pub(crate) fn get_or_compile_regex(pattern: &str, options: &str) -> Result<Arc<Regex>> {
    let cache_key = format!("{}:{}", pattern, options);

    // Try cache first
    // Note: unwrap_or_else recovers from poisoned mutex (another thread panicked while holding lock)
    {
        let mut cache = REGEX_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(regex) = cache.get(&cache_key) {
            return Ok(Arc::clone(regex)); // Cheap: just increments ref count
        }
    }

    // Build and compile regex with options
    let regex_pattern = build_regex_pattern(pattern, options)?;
    let regex = Arc::new(Regex::new(&regex_pattern).map_err(|e| {
        IronBaseError::InvalidQuery(format!("Invalid regex pattern '{}': {}", pattern, e))
    })?);

    // Store in cache
    {
        let mut cache = REGEX_CACHE.lock().unwrap_or_else(|e| e.into_inner());
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
pub fn regex_match_with_options(text: &str, pattern: &str, options: &str) -> Result<bool> {
    let regex = get_or_compile_regex(pattern, options)?;
    Ok(regex.is_match(text))
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
                    Err(IronBaseError::InvalidQuery(
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
                    Err(IronBaseError::InvalidQuery(
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
    pub fn similarity(&self, a: &str, b: &str) -> f64 {
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
    pub fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "jaro_winkler" | "jarowinkler" | "jaro-winkler" => Ok(FuzzyAlgorithm::JaroWinkler),
            "levenshtein" => Ok(FuzzyAlgorithm::Levenshtein),
            "damerau_levenshtein" | "damerau-levenshtein" | "dameraulevenshtein" => {
                Ok(FuzzyAlgorithm::DamerauLevenshtein)
            }
            _ => Err(IronBaseError::InvalidQuery(format!(
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
                    IronBaseError::InvalidQuery(
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
                return Err(IronBaseError::InvalidQuery(format!(
                    "$fuzzy threshold must be between 0.0 and 1.0, got {}",
                    threshold
                )));
            }

            Ok((value, algorithm, threshold))
        }

        _ => Err(IronBaseError::InvalidQuery(
            "$fuzzy operator requires a string or object with 'value' field".to_string(),
        )),
    }
}

// ============================================================================
// TEXT SEARCH OPERATOR ($text)
// ============================================================================

use crate::fulltext::{tokenize, FtsLanguage, FtsOptions};
use std::collections::HashSet;

/// $text operator: Matches strings using tokenization and optional stemming
///
/// # Syntax
///
/// Simple form (no stemming, accent folding ON):
/// ```json
/// { "content": { "$text": "király" } }
/// ```
///
/// Extended form with language support:
/// ```json
/// { "content": { "$text": { "$search": "király története", "$language": "hungarian", "$caseSensitive": false } } }
/// ```
///
/// # Supported Languages
///
/// - `"none"` (default): No stemming, just tokenization + accent folding
/// - `"hungarian"`: Hungarian stemming + stop words
/// - `"english"`: English stemming + stop words
/// - `"german"`: German stemming + stop words
///
/// # Matching Logic
///
/// ALL query tokens must be present in the document field tokens (AND logic).
/// Tokenization includes: lowercase, accent folding, stop word removal, optional stemming.
///
/// # Complexity: CC = 5
pub struct TextOperator;

impl OperatorMatcher for TextOperator {
    fn name(&self) -> &'static str {
        "$text"
    }

    fn matches(
        &self,
        doc_value: Option<&Value>,
        filter_value: &Value,
        _document: Option<&Document>,
    ) -> Result<bool> {
        let (search_text, language, case_sensitive) = parse_text_filter(filter_value)?;

        match doc_value {
            None => Ok(false),
            Some(Value::String(s)) => text_matches(s, &search_text, language, case_sensitive),
            Some(Value::Array(arr)) => {
                for elem in arr {
                    if let Value::String(s) = elem {
                        if text_matches(s, &search_text, language, case_sensitive)? {
                            return Ok(true);
                        }
                    }
                }
                Ok(false)
            }
            Some(_) => Ok(false),
        }
    }
}

/// Parse $text filter value into (search_text, language, case_sensitive)
fn parse_text_filter(filter_value: &Value) -> Result<(String, FtsLanguage, bool)> {
    match filter_value {
        // Simple form: { "$text": "search terms" }
        Value::String(s) => Ok((s.clone(), FtsLanguage::None, false)),

        // Extended form: { "$text": { "$search": "...", "$language": "...", "$caseSensitive": false } }
        Value::Object(obj) => {
            let search = obj
                .get("$search")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    IronBaseError::InvalidQuery(
                        "$text object form requires '$search' field with string".to_string(),
                    )
                })?
                .to_string();

            let language = obj
                .get("$language")
                .and_then(|v| v.as_str())
                .map(FtsLanguage::from_str)
                .unwrap_or(FtsLanguage::None);

            let case_sensitive = obj
                .get("$caseSensitive")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            Ok((search, language, case_sensitive))
        }

        _ => Err(IronBaseError::InvalidQuery(
            "$text requires a string or object with '$search' field".to_string(),
        )),
    }
}

/// Check if document text matches the search text using tokenization
fn text_matches(
    doc_text: &str,
    search_text: &str,
    language: FtsLanguage,
    case_sensitive: bool,
) -> Result<bool> {
    let options = FtsOptions::with_settings(language, 2, !case_sensitive);
    let doc_tokens: HashSet<String> = tokenize(doc_text, &options).into_iter().collect();
    let query_tokens = tokenize(search_text, &options);

    if query_tokens.is_empty() {
        return Ok(false);
    }
    Ok(query_tokens.iter().all(|qt| doc_tokens.contains(qt)))
}

// ============================================================================
// STRING PATTERN OPERATORS ($startsWith, $endsWith, $contains)
// ============================================================================

/// Parse string pattern filter value into (pattern, case_sensitive)
///
/// Supports two forms:
/// - Simple: `"pattern"` → case-insensitive
/// - Extended: `{ "$value": "pattern", "$caseSensitive": true }`
fn parse_string_pattern_filter(filter_value: &Value, op_name: &str) -> Result<(String, bool)> {
    match filter_value {
        // Simple form: case-insensitive by default
        Value::String(s) => Ok((s.clone(), false)),

        // Extended form with options
        Value::Object(obj) => {
            let value = obj
                .get("$value")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    IronBaseError::InvalidQuery(format!(
                        "{} object form requires '$value' field with string",
                        op_name
                    ))
                })?
                .to_string();

            let case_sensitive = obj
                .get("$caseSensitive")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            Ok((value, case_sensitive))
        }

        _ => Err(IronBaseError::InvalidQuery(format!(
            "{} requires a string or object with '$value' field",
            op_name
        ))),
    }
}

// Wrapper functions to avoid generic Pattern trait issues with fn pointers
fn str_starts_with(haystack: &str, needle: &str) -> bool {
    haystack.starts_with(needle)
}

fn str_ends_with(haystack: &str, needle: &str) -> bool {
    haystack.ends_with(needle)
}

fn str_contains_substr(haystack: &str, needle: &str) -> bool {
    haystack.contains(needle)
}

/// Generic string pattern matching for $startsWith, $endsWith, $contains
///
/// Applies the given `check` function to document value and pattern,
/// with optional case-insensitive matching (default).
fn string_pattern_matches(
    doc_value: Option<&Value>,
    filter_value: &Value,
    op_name: &str,
    check: fn(&str, &str) -> bool,
) -> Result<bool> {
    let (pattern, case_sensitive) = parse_string_pattern_filter(filter_value, op_name)?;

    // Pre-compute lowercase pattern once (avoid redundant alloc in array loop)
    let pattern_lower = if case_sensitive {
        String::new() // unused, no allocation
    } else {
        pattern.to_lowercase()
    };

    match doc_value {
        None => Ok(false),
        Some(Value::String(s)) => {
            if case_sensitive {
                Ok(check(s, &pattern))
            } else {
                Ok(check(&s.to_lowercase(), &pattern_lower))
            }
        }
        Some(Value::Array(arr)) => {
            for elem in arr {
                if let Value::String(s) = elem {
                    let matches = if case_sensitive {
                        check(s, &pattern)
                    } else {
                        check(&s.to_lowercase(), &pattern_lower)
                    };
                    if matches {
                        return Ok(true);
                    }
                }
            }
            Ok(false)
        }
        Some(_) => Ok(false),
    }
}

/// $startsWith operator: Matches strings that start with a given prefix
///
/// # Syntax
///
/// Simple form (case-insensitive):
/// ```json
/// { "name": { "$startsWith": "Al" } }
/// ```
///
/// Extended form:
/// ```json
/// { "name": { "$startsWith": { "$value": "Al", "$caseSensitive": true } } }
/// ```
///
/// # Complexity: CC = 3
pub struct StartsWithOperator;

impl OperatorMatcher for StartsWithOperator {
    fn name(&self) -> &'static str {
        "$startsWith"
    }

    fn matches(
        &self,
        doc_value: Option<&Value>,
        filter_value: &Value,
        _document: Option<&Document>,
    ) -> Result<bool> {
        string_pattern_matches(doc_value, filter_value, "$startsWith", str_starts_with)
    }
}

/// $endsWith operator: Matches strings that end with a given suffix
///
/// # Syntax
///
/// Simple form (case-insensitive):
/// ```json
/// { "email": { "$endsWith": ".hu" } }
/// ```
///
/// Extended form:
/// ```json
/// { "email": { "$endsWith": { "$value": ".hu", "$caseSensitive": true } } }
/// ```
///
/// # Complexity: CC = 3
pub struct EndsWithOperator;

impl OperatorMatcher for EndsWithOperator {
    fn name(&self) -> &'static str {
        "$endsWith"
    }

    fn matches(
        &self,
        doc_value: Option<&Value>,
        filter_value: &Value,
        _document: Option<&Document>,
    ) -> Result<bool> {
        string_pattern_matches(doc_value, filter_value, "$endsWith", str_ends_with)
    }
}

/// $contains operator: Matches strings that contain a given substring
///
/// # Syntax
///
/// Simple form (case-insensitive):
/// ```json
/// { "bio": { "$contains": "Rust" } }
/// ```
///
/// Extended form:
/// ```json
/// { "bio": { "$contains": { "$value": "Rust", "$caseSensitive": true } } }
/// ```
///
/// # Complexity: CC = 3
pub struct ContainsOperator;

impl OperatorMatcher for ContainsOperator {
    fn name(&self) -> &'static str {
        "$contains"
    }

    fn matches(
        &self,
        doc_value: Option<&Value>,
        filter_value: &Value,
        _document: Option<&Document>,
    ) -> Result<bool> {
        string_pattern_matches(doc_value, filter_value, "$contains", str_contains_substr)
    }
}
