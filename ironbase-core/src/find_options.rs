// ironbase-core/src/find_options.rs
// Find query options: projection, sort, limit, skip

use crate::error::{IronBaseError, Result};
use crate::value_utils::{
    compare_values as compare_values_core, delete_nested_value, get_nested_value, set_nested_value,
};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

/// Options for find queries
#[derive(Debug, Clone, Default)]
pub struct FindOptions {
    /// Projection: field → 1 (include) or 0 (exclude)
    /// Special case: _id can be excluded in include mode
    pub projection: Option<HashMap<String, i32>>,

    /// Sort: [(field, direction)], direction: 1 (asc) or -1 (desc)
    pub sort: Option<Vec<(String, i32)>>,

    /// Limit: maximum number of documents to return
    pub limit: Option<usize>,

    /// Skip: number of documents to skip (for pagination)
    pub skip: Option<usize>,

    /// Include total count of matching documents (ignoring limit/skip)
    /// Useful for pagination where you need to know total pages
    pub include_total: bool,

    /// Maximum total response size in bytes (OOM protection)
    /// When set, documents are loaded until this limit would be exceeded,
    /// then an error is returned with guidance to use limit/projection.
    /// Default: None (no limit)
    ///
    /// Use `FindOptions::with_safe_defaults()` for automatic RAM-based limits.
    pub max_response_bytes: Option<usize>,

    /// Cancellation flag for cooperative timeout support
    /// When set to true, the find operation will abort and return an error.
    /// Check is performed every 100 documents during loading.
    /// Default: None (no cancellation support)
    pub cancel_flag: Option<Arc<AtomicBool>>,

    /// Deadline for timeout enforcement
    /// When set, the find operation will check this deadline every 100 documents
    /// and abort with a timeout error if the deadline has passed.
    /// Default: None (no timeout)
    pub deadline: Option<Instant>,
}

/// Result of find_with_options when include_total is true
#[derive(Debug, Clone, Default)]
pub struct FindResult {
    /// Documents matching the query (respecting limit/skip)
    pub documents: Vec<Value>,

    /// Total count of documents matching the filter (ignoring limit/skip)
    /// Only populated when FindOptions.include_total is true
    pub total: Option<u64>,
}

impl FindOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_projection(mut self, projection: HashMap<String, i32>) -> Self {
        self.projection = Some(projection);
        self
    }

    pub fn with_sort(mut self, sort: Vec<(String, i32)>) -> Self {
        self.sort = Some(sort);
        self
    }

    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }

    pub fn with_skip(mut self, skip: usize) -> Self {
        self.skip = Some(skip);
        self
    }

    /// Include total count of matching documents in the result
    pub fn with_include_total(mut self, include: bool) -> Self {
        self.include_total = include;
        self
    }

    /// Set maximum response size in bytes (OOM protection)
    pub fn with_max_response_bytes(mut self, max_bytes: usize) -> Self {
        self.max_response_bytes = Some(max_bytes);
        self
    }

    /// Set cancellation flag for cooperative timeout support
    ///
    /// When the flag is set to true (externally), the find operation will
    /// check it every 100 documents and abort with a cancellation error.
    pub fn with_cancel_flag(mut self, flag: Arc<AtomicBool>) -> Self {
        self.cancel_flag = Some(flag);
        self
    }

    /// Set deadline for timeout enforcement
    ///
    /// The find operation will check this deadline every 100 documents
    /// and abort with a timeout error if the deadline has passed.
    pub fn with_deadline(mut self, deadline: Instant) -> Self {
        self.deadline = Some(deadline);
        self
    }

    /// Create FindOptions with safe defaults based on system RAM
    ///
    /// Automatically sets `max_response_bytes` to ~10% of available RAM,
    /// capped between 10 MB and 500 MB.
    ///
    /// # Scaling table
    /// | Available RAM | max_response_bytes |
    /// |---------------|-------------------|
    /// | < 512 MB      | 10 MB             |
    /// | 512MB - 2GB   | 50 MB             |
    /// | 2GB - 8GB     | 100 MB            |
    /// | 8GB - 32GB    | 200 MB            |
    /// | > 32GB        | 500 MB            |
    pub fn with_safe_defaults() -> Self {
        Self {
            max_response_bytes: Some(calculate_safe_response_limit()),
            ..Default::default()
        }
    }
}

/// Calculate safe response size limit based on system RAM
///
/// Uses 10% of available RAM, capped between 10 MB and 500 MB.
/// Falls back to 50 MB if memory detection fails.
pub fn calculate_safe_response_limit() -> usize {
    use crate::aggregation::memory_info::get_available_memory_bytes;

    match get_available_memory_bytes() {
        Some(bytes) => {
            let available_mb = bytes / (1024 * 1024);
            // Use 10% of available RAM, bounded [10 MB, 500 MB]
            let max_mb = (available_mb as usize / 10).clamp(10, 500);
            max_mb * 1024 * 1024
        }
        None => 50 * 1024 * 1024, // 50 MB fallback
    }
}

/// Estimate JSON serialized size of a Value (without actually serializing)
///
/// Fast approximation that avoids allocation. Used for response size tracking.
pub fn estimate_json_size(value: &Value) -> usize {
    match value {
        Value::Null => 4,
        Value::Bool(true) => 4,
        Value::Bool(false) => 5,
        Value::Number(n) => n.to_string().len(),
        Value::String(s) => s.len() + 2, // quotes
        Value::Array(arr) => {
            2 + arr.iter().map(estimate_json_size).sum::<usize>() + arr.len().saturating_sub(1)
            // commas
        }
        Value::Object(obj) => {
            2 + obj
                .iter()
                .map(|(k, v)| k.len() + 3 + estimate_json_size(v)) // "key": value
                .sum::<usize>()
                + obj.len().saturating_sub(1) // commas
        }
    }
}

impl FindResult {
    pub fn new(documents: Vec<Value>) -> Self {
        Self {
            documents,
            total: None,
        }
    }

    pub fn with_total(documents: Vec<Value>, total: u64) -> Self {
        Self {
            documents,
            total: Some(total),
        }
    }
}

/// Get nested value with array projection support
/// When path crosses an array, projects the remaining path into each array element
/// e.g., "to.email" where "to" is an array returns array of just email values
fn get_nested_value_with_array_projection(doc: &Value, path: &str) -> Option<Value> {
    // Fast path: no dots means simple field access
    if !path.contains('.') {
        return doc.get(path).cloned();
    }

    let parts: Vec<&str> = path.split('.').collect();
    get_nested_value_recursive(doc, &parts, 0)
}

/// Recursive helper for nested value retrieval with array support
fn get_nested_value_recursive(value: &Value, parts: &[&str], index: usize) -> Option<Value> {
    if index >= parts.len() {
        return Some(value.clone());
    }

    let part = parts[index];

    match value {
        Value::Object(map) => {
            if let Some(child) = map.get(part) {
                get_nested_value_recursive(child, parts, index + 1)
            } else {
                None
            }
        }
        Value::Array(arr) => {
            // Check if part is a numeric index
            if let Ok(arr_index) = part.parse::<usize>() {
                if let Some(element) = arr.get(arr_index) {
                    get_nested_value_recursive(element, parts, index + 1)
                } else {
                    None
                }
            } else {
                // MongoDB-style: project the remaining path into each array element
                // e.g., for "to.email" where "to" is an array of {email, name} objects
                // returns an array of just the nested values
                let projected: Vec<Value> = arr
                    .iter()
                    .filter_map(|element| get_nested_value_recursive(element, parts, index))
                    .collect();

                if projected.is_empty() {
                    None
                } else {
                    Some(Value::Array(projected))
                }
            }
        }
        _ => None,
    }
}

/// Set nested value with array projection support
/// When path crosses an array, creates projected structure for each element
/// Merges with existing array elements if the array already exists
fn set_nested_value_with_array(doc: &mut Value, path: &str, value: Value) {
    let parts: Vec<&str> = path.split('.').collect();
    set_nested_value_recursive(doc, &parts, 0, &value);
}

/// Recursive helper for setting nested values with array support
fn set_nested_value_recursive(doc: &mut Value, parts: &[&str], index: usize, value: &Value) {
    if index >= parts.len() {
        return;
    }

    let part = parts[index];
    let is_last = index == parts.len() - 1;

    if let Value::Object(map) = doc {
        if is_last {
            // Last part: set the value
            map.insert(part.to_string(), value.clone());
        } else {
            // Check if the value we're setting is an array (from array projection)
            if let Value::Array(arr) = value {
                // Check if we already have an array at this position
                if let Some(Value::Array(existing_arr)) = map.get_mut(part) {
                    // MERGE: Add fields to existing array elements
                    for (i, element) in arr.iter().enumerate() {
                        if let Some(existing_element) = existing_arr.get_mut(i) {
                            // Merge the new field into the existing element
                            set_nested_value_recursive(existing_element, parts, index + 1, element);
                        } else {
                            // Array grew - create new element
                            let mut obj = Value::Object(serde_json::Map::new());
                            set_nested_value_recursive(&mut obj, parts, index + 1, element);
                            existing_arr.push(obj);
                        }
                    }
                    return;
                }

                // No existing array - create new array structure
                let projected_array: Vec<Value> = arr
                    .iter()
                    .map(|element| {
                        let mut obj = Value::Object(serde_json::Map::new());
                        set_nested_value_recursive(&mut obj, parts, index + 1, element);
                        obj
                    })
                    .collect();
                map.insert(part.to_string(), Value::Array(projected_array));
                return;
            }

            // Regular object case
            let entry = map
                .entry(part.to_string())
                .or_insert_with(|| Value::Object(serde_json::Map::new()));
            set_nested_value_recursive(entry, parts, index + 1, value);
        }
    }
}

/// Apply projection to a document
/// Supports dot notation for nested fields (e.g., "address.city")
/// Supports array element projection (e.g., "to.email" where "to" is an array)
///
/// # Errors
/// Validate a projection map without applying it to a document.
///
/// Checks:
/// - All values are 0 or 1
/// - No mixing of inclusion (1) and exclusion (0) fields (except `_id: 0` in include mode)
///
/// Call this early (before query execution) to fail fast on invalid projections.
pub fn validate_projection(projection: &HashMap<String, i32>) -> Result<()> {
    if projection.is_empty() {
        return Ok(());
    }

    if let Some((field, value)) = projection.iter().find(|(_, &v)| v != 0 && v != 1) {
        return Err(IronBaseError::InvalidQuery(format!(
            "Invalid projection value for '{}': expected 0 or 1, got {}",
            field, value
        )));
    }

    let has_inclusions = projection.values().any(|&v| v == 1);
    let has_non_id_exclusions = projection
        .iter()
        .any(|(field, &action)| action == 0 && field != "_id");

    if has_inclusions && has_non_id_exclusions {
        return Err(IronBaseError::InvalidQuery(
            "Cannot mix inclusion and exclusion in projection (except _id: 0)".to_string(),
        ));
    }

    Ok(())
}

/// Returns `InvalidQuery` if projection mixes inclusion (1) and exclusion (0) fields.
/// Exception: `_id: 0` is allowed in include mode (MongoDB compatible).
pub fn apply_projection(doc: &Value, projection: &HashMap<String, i32>) -> Result<Value> {
    if projection.is_empty() {
        return Ok(doc.clone());
    }

    if let Some((field, value)) = projection.iter().find(|(_, &v)| v != 0 && v != 1) {
        return Err(IronBaseError::InvalidQuery(format!(
            "Invalid projection value for '{}': expected 0 or 1, got {}",
            field, value
        )));
    }

    // Detect mode
    let has_inclusions = projection.values().any(|&v| v == 1);
    let has_non_id_exclusions = projection
        .iter()
        .any(|(field, &action)| action == 0 && field != "_id");

    // Validate: Cannot mix inclusion and exclusion (except _id: 0 in include mode)
    if has_inclusions && has_non_id_exclusions {
        return Err(IronBaseError::InvalidQuery(
            "Cannot mix inclusion and exclusion in projection (except _id: 0)".to_string(),
        ));
    }

    let include_mode = has_inclusions;

    if let Value::Object(obj) = doc {
        let mut result = Value::Object(serde_json::Map::new());

        if include_mode {
            // Include specified fields
            for (field, &action) in projection {
                if action == 1 {
                    // Use array-aware nested value retrieval
                    if let Some(value) = get_nested_value_with_array_projection(doc, field) {
                        // Use array-aware nested value setting
                        set_nested_value_with_array(&mut result, field, value);
                    }
                }
            }

            // Include _id unless explicitly excluded
            if projection.get("_id") != Some(&0) {
                if let Some(id) = obj.get("_id") {
                    set_nested_value(&mut result, "_id", id.clone());
                }
            }
        } else {
            // Exclude mode: copy entire document, then delete excluded fields
            // BUG #3 FIX: Now supports dot notation for nested field exclusion
            // e.g., {"address.city": 0} will properly exclude only the city field
            result = doc.clone();

            // Delete each excluded field (supports dot notation)
            for (field, &action) in projection {
                if action == 0 {
                    delete_nested_value(&mut result, field);
                }
            }
        }

        Ok(result)
    } else {
        Ok(doc.clone())
    }
}

/// Apply sort to documents
/// Supports dot notation for nested fields (e.g., "address.city")
///
/// # Errors
/// Returns `InvalidQuery` if sort direction is not 1 (ascending) or -1 (descending).
pub fn apply_sort(docs: &mut [Value], sort: &[(String, i32)]) -> Result<()> {
    if sort.is_empty() {
        return Ok(());
    }

    // Validate sort directions (MongoDB only accepts 1 or -1)
    for (field, direction) in sort {
        if *direction != 1 && *direction != -1 {
            return Err(IronBaseError::InvalidQuery(format!(
                "Invalid sort direction {} for field '{}'. Must be 1 (ascending) or -1 (descending).",
                direction, field
            )));
        }
    }

    docs.sort_by(|a, b| {
        for (field, direction) in sort {
            // Use get_nested_value to support dot notation (e.g., "address.city")
            let val_a = get_nested_value(a, field);
            let val_b = get_nested_value(b, field);

            let cmp = compare_values(val_a, val_b);

            if cmp != std::cmp::Ordering::Equal {
                return if *direction == 1 { cmp } else { cmp.reverse() };
            }
        }
        std::cmp::Ordering::Equal
    });

    Ok(())
}

/// Compare two JSON values for sorting
/// BUG #2 FIX: Now uses compare_values_core which handles large integers correctly
fn compare_values(a: Option<&Value>, b: Option<&Value>) -> std::cmp::Ordering {
    use std::cmp::Ordering;

    match (a, b) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Less, // null < any value
        (Some(_), None) => Ordering::Greater,

        // BUG #2 FIX: Use the core compare_values which doesn't lose precision
        // for large integers (> 2^53)
        (Some(a_val), Some(b_val)) => {
            compare_values_core(a_val, b_val).unwrap_or_else(|| {
                // Type priority fallback for incompatible types
                type_priority(a_val).cmp(&type_priority(b_val))
            })
        }
    }
}

/// Get type priority for mixed-type sorting
fn type_priority(val: &Value) -> u8 {
    match val {
        Value::Null => 0,
        Value::Number(_) => 1,
        Value::String(_) => 2,
        Value::Bool(_) => 3,
        Value::Object(_) => 4,
        Value::Array(_) => 5,
    }
}

/// Apply limit and skip to documents
///
/// 🚀 OPTIMIZED: Uses into_iter() to move elements instead of cloning.
/// The owned Vec is consumed and elements are transferred, not copied.
pub fn apply_limit_skip(docs: Vec<Value>, limit: Option<usize>, skip: Option<usize>) -> Vec<Value> {
    let skip_count = skip.unwrap_or(0);

    // Early return for edge case (avoids iterator overhead)
    if skip_count >= docs.len() {
        return Vec::new();
    }

    // Move elements via into_iter() - zero-copy for individual Values
    let iter = docs.into_iter().skip(skip_count);

    // MongoDB compatibility: limit(0) means "no limit"
    if let Some(limit_count) = limit {
        if limit_count > 0 {
            iter.take(limit_count).collect()
        } else {
            iter.collect()
        }
    } else {
        iter.collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_projection_include_mode() {
        let doc = json!({"name": "Alice", "age": 30, "city": "NYC", "_id": 1});
        let projection = HashMap::from([("name".to_string(), 1), ("age".to_string(), 1)]);

        let result = apply_projection(&doc, &projection).unwrap();
        assert!(result.get("name").is_some());
        assert!(result.get("age").is_some());
        assert!(result.get("_id").is_some()); // Included by default
        assert!(result.get("city").is_none());
    }

    #[test]
    fn test_projection_exclude_id() {
        let doc = json!({"name": "Alice", "age": 30, "_id": 1});
        let projection = HashMap::from([
            ("name".to_string(), 1),
            ("_id".to_string(), 0), // Explicit exclude
        ]);

        let result = apply_projection(&doc, &projection).unwrap();
        assert!(result.get("name").is_some());
        assert!(result.get("_id").is_none()); // Excluded
    }

    #[test]
    fn test_projection_exclude_mode() {
        let doc = json!({"name": "Alice", "age": 30, "city": "NYC", "_id": 1});
        let projection = HashMap::from([("city".to_string(), 0)]);

        let result = apply_projection(&doc, &projection).unwrap();
        assert!(result.get("name").is_some());
        assert!(result.get("age").is_some());
        assert!(result.get("_id").is_some());
        assert!(result.get("city").is_none()); // Excluded
    }

    #[test]
    fn test_projection_exclude_dot_notation() {
        // BUG #3 regression test: exclude mode with dot notation
        let doc = json!({
            "_id": 1,
            "user": {
                "password": "secret123",
                "email": "alice@example.com",
                "name": "Alice"
            },
            "data": "public info"
        });

        // Exclude sensitive field: user.password
        let projection = HashMap::from([("user.password".to_string(), 0)]);

        let result = apply_projection(&doc, &projection).unwrap();

        // _id and data should remain
        assert_eq!(result.get("_id"), Some(&json!(1)));
        assert_eq!(result.get("data"), Some(&json!("public info")));

        // user should exist but without password
        let user = result.get("user").unwrap();
        assert_eq!(user.get("email"), Some(&json!("alice@example.com")));
        assert_eq!(user.get("name"), Some(&json!("Alice")));
        assert!(
            user.get("password").is_none(),
            "user.password should be excluded"
        );
    }

    #[test]
    fn test_projection_exclude_deeply_nested() {
        // BUG #3: Test deeply nested exclusion
        let doc = json!({
            "_id": 1,
            "settings": {
                "security": {
                    "api_key": "secret-key",
                    "rate_limit": 100
                },
                "display": {
                    "theme": "dark"
                }
            }
        });

        // Exclude settings.security.api_key
        let projection = HashMap::from([("settings.security.api_key".to_string(), 0)]);

        let result = apply_projection(&doc, &projection).unwrap();

        // api_key should be excluded, rate_limit should remain
        let security = result.get("settings").unwrap().get("security").unwrap();
        assert!(
            security.get("api_key").is_none(),
            "api_key should be excluded"
        );
        assert_eq!(security.get("rate_limit"), Some(&json!(100)));

        // display should be unchanged
        assert_eq!(
            result.get("settings").unwrap().get("display"),
            Some(&json!({"theme": "dark"}))
        );
    }

    #[test]
    fn test_projection_exclude_multiple_dot_notation() {
        // BUG #3: Multiple nested exclusions
        let doc = json!({
            "_id": 1,
            "user": {"password": "secret", "token": "abc123", "name": "Alice"}
        });

        // Exclude both password and token
        let projection = HashMap::from([
            ("user.password".to_string(), 0),
            ("user.token".to_string(), 0),
        ]);

        let result = apply_projection(&doc, &projection).unwrap();

        let user = result.get("user").unwrap();
        assert!(user.get("password").is_none());
        assert!(user.get("token").is_none());
        assert_eq!(user.get("name"), Some(&json!("Alice")));
    }

    #[test]
    fn test_sort_single_field() {
        let mut docs = vec![json!({"age": 30}), json!({"age": 25}), json!({"age": 35})];

        let sort = vec![("age".to_string(), 1)]; // Ascending

        apply_sort(&mut docs, &sort).unwrap();

        assert_eq!(docs[0].get("age").unwrap(), 25);
        assert_eq!(docs[1].get("age").unwrap(), 30);
        assert_eq!(docs[2].get("age").unwrap(), 35);
    }

    #[test]
    fn test_sort_descending() {
        let mut docs = vec![json!({"age": 30}), json!({"age": 25}), json!({"age": 35})];

        let sort = vec![("age".to_string(), -1)]; // Descending

        apply_sort(&mut docs, &sort).unwrap();

        assert_eq!(docs[0].get("age").unwrap(), 35);
        assert_eq!(docs[1].get("age").unwrap(), 30);
        assert_eq!(docs[2].get("age").unwrap(), 25);
    }

    #[test]
    fn test_sort_multi_field() {
        let mut docs = vec![
            json!({"age": 30, "name": "Bob"}),
            json!({"age": 25, "name": "Alice"}),
            json!({"age": 30, "name": "Carol"}),
        ];

        let sort = vec![
            ("age".to_string(), 1),   // Age ascending
            ("name".to_string(), -1), // Name descending
        ];

        apply_sort(&mut docs, &sort).unwrap();

        assert_eq!(docs[0].get("name").unwrap(), "Alice"); // age=25
        assert_eq!(docs[1].get("name").unwrap(), "Carol"); // age=30, name=C
        assert_eq!(docs[2].get("name").unwrap(), "Bob"); // age=30, name=B
    }

    #[test]
    fn test_sort_string() {
        let mut docs = vec![
            json!({"name": "Charlie"}),
            json!({"name": "Alice"}),
            json!({"name": "Bob"}),
        ];

        let sort = vec![("name".to_string(), 1)];

        apply_sort(&mut docs, &sort).unwrap();

        assert_eq!(docs[0].get("name").unwrap(), "Alice");
        assert_eq!(docs[1].get("name").unwrap(), "Bob");
        assert_eq!(docs[2].get("name").unwrap(), "Charlie");
    }

    #[test]
    fn test_limit() {
        let docs = vec![
            json!({"n": 1}),
            json!({"n": 2}),
            json!({"n": 3}),
            json!({"n": 4}),
            json!({"n": 5}),
        ];

        let result = apply_limit_skip(docs, Some(3), None);

        assert_eq!(result.len(), 3);
        assert_eq!(result[0].get("n").unwrap(), 1);
        assert_eq!(result[1].get("n").unwrap(), 2);
        assert_eq!(result[2].get("n").unwrap(), 3);
    }

    #[test]
    fn test_skip() {
        let docs = vec![
            json!({"n": 1}),
            json!({"n": 2}),
            json!({"n": 3}),
            json!({"n": 4}),
            json!({"n": 5}),
        ];

        let result = apply_limit_skip(docs, None, Some(2));

        assert_eq!(result.len(), 3);
        assert_eq!(result[0].get("n").unwrap(), 3);
        assert_eq!(result[1].get("n").unwrap(), 4);
        assert_eq!(result[2].get("n").unwrap(), 5);
    }

    #[test]
    fn test_limit_skip() {
        let docs = vec![
            json!({"n": 1}),
            json!({"n": 2}),
            json!({"n": 3}),
            json!({"n": 4}),
            json!({"n": 5}),
        ];

        let result = apply_limit_skip(docs, Some(2), Some(1));

        assert_eq!(result.len(), 2);
        assert_eq!(result[0].get("n").unwrap(), 2);
        assert_eq!(result[1].get("n").unwrap(), 3);
    }

    #[test]
    fn test_skip_beyond_length() {
        let docs = vec![json!({"n": 1}), json!({"n": 2})];

        let result = apply_limit_skip(docs, None, Some(10));

        assert_eq!(result.len(), 0);
    }

    // ========== Dot notation tests ==========

    #[test]
    fn test_get_nested_value() {
        let doc = json!({
            "name": "Alice",
            "address": {
                "city": "NYC",
                "zip": {
                    "code": "10001"
                }
            }
        });

        assert_eq!(get_nested_value(&doc, "name"), Some(&json!("Alice")));
        assert_eq!(get_nested_value(&doc, "address.city"), Some(&json!("NYC")));
        assert_eq!(
            get_nested_value(&doc, "address.zip.code"),
            Some(&json!("10001"))
        );
        assert_eq!(get_nested_value(&doc, "nonexistent"), None);
        assert_eq!(get_nested_value(&doc, "address.nonexistent"), None);
    }

    #[test]
    fn test_projection_dot_notation() {
        let doc = json!({
            "_id": 1,
            "name": "Alice",
            "address": {
                "city": "NYC",
                "street": "123 Main St",
                "zip": "10001"
            }
        });

        // Include nested field with dot notation
        let projection = HashMap::from([("address.city".to_string(), 1), ("name".to_string(), 1)]);

        let result = apply_projection(&doc, &projection).unwrap();

        assert!(result.get("_id").is_some()); // _id included by default
        assert!(result.get("name").is_some());
        // MongoDB-compatible: returns nested structure, not flat key
        assert!(result.get("address").is_some());
        assert_eq!(result["address"]["city"], json!("NYC"));
        // Only the projected field is included, not other nested fields
        assert!(result["address"].get("street").is_none());
        assert!(result["address"].get("zip").is_none());
    }

    #[test]
    fn test_projection_deeply_nested() {
        let doc = json!({
            "_id": 1,
            "data": {
                "level1": {
                    "level2": {
                        "value": 42
                    }
                }
            }
        });

        let projection = HashMap::from([("data.level1.level2.value".to_string(), 1)]);

        let result = apply_projection(&doc, &projection).unwrap();
        // MongoDB-compatible: returns nested structure
        assert_eq!(result["data"]["level1"]["level2"]["value"], json!(42));
    }

    #[test]
    fn test_projection_mixed_mode_error() {
        let doc = json!({"name": "Alice", "age": 30, "city": "NYC"});
        // Mixed: include name (1) and exclude age (0) - MongoDB rejects this
        let projection = HashMap::from([("name".to_string(), 1), ("age".to_string(), 0)]);

        let result = apply_projection(&doc, &projection);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Cannot mix inclusion and exclusion"));
    }

    #[test]
    fn test_sort_dot_notation() {
        let mut docs = vec![
            json!({"name": "Charlie", "address": {"zip": 30000}}),
            json!({"name": "Alice", "address": {"zip": 10000}}),
            json!({"name": "Bob", "address": {"zip": 20000}}),
        ];

        let sort = vec![("address.zip".to_string(), 1)]; // Ascending by nested field

        apply_sort(&mut docs, &sort).unwrap();

        assert_eq!(docs[0].get("name").unwrap(), "Alice");
        assert_eq!(docs[1].get("name").unwrap(), "Bob");
        assert_eq!(docs[2].get("name").unwrap(), "Charlie");
    }

    #[test]
    fn test_sort_dot_notation_descending() {
        let mut docs = vec![
            json!({"name": "Alice", "stats": {"score": 85}}),
            json!({"name": "Bob", "stats": {"score": 92}}),
            json!({"name": "Charlie", "stats": {"score": 78}}),
        ];

        let sort = vec![("stats.score".to_string(), -1)]; // Descending by nested field

        apply_sort(&mut docs, &sort).unwrap();

        assert_eq!(docs[0].get("name").unwrap(), "Bob");
        assert_eq!(docs[1].get("name").unwrap(), "Alice");
        assert_eq!(docs[2].get("name").unwrap(), "Charlie");
    }

    #[test]
    fn test_sort_dot_notation_multi_field() {
        let mut docs = vec![
            json!({"name": "Alice", "location": {"city": "NYC"}, "stats": {"score": 80}}),
            json!({"name": "Bob", "location": {"city": "LA"}, "stats": {"score": 90}}),
            json!({"name": "Charlie", "location": {"city": "NYC"}, "stats": {"score": 70}}),
        ];

        // Sort by city ascending, then by score descending
        let sort = vec![
            ("location.city".to_string(), 1),
            ("stats.score".to_string(), -1),
        ];

        apply_sort(&mut docs, &sort).unwrap();

        // LA comes first (alphabetically), then NYC
        assert_eq!(docs[0].get("name").unwrap(), "Bob"); // LA
        assert_eq!(docs[1].get("name").unwrap(), "Alice"); // NYC, score=80
        assert_eq!(docs[2].get("name").unwrap(), "Charlie"); // NYC, score=70
    }

    #[test]
    fn test_sort_dot_notation_with_missing_field() {
        let mut docs = vec![
            json!({"name": "Alice", "address": {"zip": 10000}}),
            json!({"name": "Bob"}), // Missing address
            json!({"name": "Charlie", "address": {"zip": 30000}}),
        ];

        let sort = vec![("address.zip".to_string(), 1)]; // Ascending

        apply_sort(&mut docs, &sort).unwrap();

        // Missing field (null) should come first
        assert_eq!(docs[0].get("name").unwrap(), "Bob");
        assert_eq!(docs[1].get("name").unwrap(), "Alice");
        assert_eq!(docs[2].get("name").unwrap(), "Charlie");
    }

    #[test]
    fn test_sort_invalid_direction_zero() {
        let mut docs = vec![json!({"age": 30}), json!({"age": 25})];
        let sort = vec![("age".to_string(), 0)]; // Invalid: 0

        let result = apply_sort(&mut docs, &sort);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Invalid sort direction"));
    }

    #[test]
    fn test_sort_invalid_direction_two() {
        let mut docs = vec![json!({"age": 30}), json!({"age": 25})];
        let sort = vec![("age".to_string(), 2)]; // Invalid: 2

        let result = apply_sort(&mut docs, &sort);
        assert!(result.is_err());
    }

    #[test]
    fn test_sort_invalid_direction_negative_two() {
        let mut docs = vec![json!({"age": 30}), json!({"age": 25})];
        let sort = vec![("age".to_string(), -2)]; // Invalid: -2

        let result = apply_sort(&mut docs, &sort);
        assert!(result.is_err());
    }
}
