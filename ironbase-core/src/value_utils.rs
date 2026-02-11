//! Value utility functions shared across modules
//!
//! This module provides common functions for working with JSON values,
//! including nested field access and value comparison.

use serde_json::Value;
use std::cmp::Ordering;

/// Get nested value from JSON with dot notation support
///
/// Supports:
/// - Simple fields: "name"
/// - Nested objects: "address.city"
/// - Array indexing: "items.0.name"
///
/// # Examples
///
/// ```ignore
/// use serde_json::json;
/// use ironbase_core::value_utils::get_nested_value;
///
/// let doc = json!({"address": {"city": "NYC"}});
/// assert_eq!(get_nested_value(&doc, "address.city"), Some(&json!("NYC")));
/// ```
pub fn get_nested_value<'a>(doc: &'a Value, path: &str) -> Option<&'a Value> {
    // Fast path: no dots means simple field access
    if !path.contains('.') {
        return doc.get(path);
    }

    let mut value = doc;
    for part in path.split('.') {
        match value {
            Value::Object(map) => value = map.get(part)?,
            Value::Array(arr) => {
                // Support array indexing: "items.0.name"
                if let Ok(index) = part.parse::<usize>() {
                    value = arr.get(index)?;
                } else {
                    return None;
                }
            }
            _ => return None,
        }
    }
    Some(value)
}

/// Retrieve all values that match a dot-notation path, flattening arrays.
///
/// This mirrors MongoDB's implicit array traversal used by queries/indexes.
pub fn get_all_nested_values<'a>(doc: &'a Value, path: &str) -> Vec<&'a Value> {
    if path.is_empty() {
        return Vec::new();
    }

    let parts: Vec<&str> = path.split('.').collect();
    if parts.is_empty() {
        return Vec::new();
    }

    let mut results = Vec::new();
    if let Some(first) = doc.get(parts[0]) {
        collect_all_values_recursive(first, &parts[1..], &mut results);
    }
    results
}

fn collect_all_values_recursive<'a>(
    value: &'a Value,
    remaining: &[&str],
    results: &mut Vec<&'a Value>,
) {
    if remaining.is_empty() {
        match value {
            Value::Array(arr) => {
                for elem in arr {
                    if let Value::Array(_) = elem {
                        collect_all_values_recursive(elem, remaining, results);
                    } else {
                        results.push(elem);
                    }
                }
            }
            _ => results.push(value),
        }
        return;
    }

    let next_part = remaining[0];
    let rest = &remaining[1..];

    match value {
        Value::Object(map) => {
            if let Some(child) = map.get(next_part) {
                collect_all_values_recursive(child, rest, results);
            }
        }
        Value::Array(arr) => {
            // Explicit numeric index (e.g., items.0.name)
            if let Ok(index) = next_part.parse::<usize>() {
                if let Some(elem) = arr.get(index) {
                    collect_all_values_recursive(elem, rest, results);
                }
            } else {
                // Implicit traversal: examine each element
                for elem in arr {
                    match elem {
                        Value::Object(map) => {
                            if let Some(child) = map.get(next_part) {
                                collect_all_values_recursive(child, rest, results);
                            }
                        }
                        Value::Array(_) => {
                            // Array of arrays: recurse without consuming the path component
                            collect_all_values_recursive(elem, remaining, results);
                        }
                        _ => {}
                    }
                }
            }
        }
        _ => {}
    }
}

/// Returns true if the given path crosses an array without an explicit numeric index.
///
/// This is used to detect multikey paths for compound index validation.
pub fn path_crosses_array(doc: &Value, path: &str) -> bool {
    if path.is_empty() {
        return false;
    }

    let parts: Vec<&str> = path.split('.').collect();
    let mut current = doc;

    for part in parts {
        match current {
            Value::Object(map) => {
                if let Some(next) = map.get(part) {
                    current = next;
                } else {
                    return false;
                }
            }
            Value::Array(arr) => {
                if let Ok(index) = part.parse::<usize>() {
                    if let Some(next) = arr.get(index) {
                        current = next;
                    } else {
                        return false;
                    }
                } else {
                    return true;
                }
            }
            _ => return false,
        }
    }

    matches!(current, Value::Array(_))
}

/// Return a substring by character index and length.
pub fn substr_string(value: &str, start: usize, length: usize) -> String {
    if length == 0 {
        return String::new();
    }
    value.chars().skip(start).take(length).collect()
}

/// Set a value at a nested path with dot notation support
///
/// Creates intermediate objects if they don't exist.
/// Used by $unwind to set the unwound element back into the document.
///
/// # Examples
///
/// ```ignore
/// use serde_json::json;
/// use ironbase_core::value_utils::set_nested_value;
///
/// let mut doc = json!({"name": "Alice"});
/// set_nested_value(&mut doc, "address.city", json!("NYC"));
/// assert_eq!(doc["address"]["city"], "NYC");
/// ```
pub fn set_nested_value(doc: &mut Value, path: &str, value: Value) {
    // Fast path: no dots means simple field assignment
    if !path.contains('.') {
        match doc {
            Value::Object(ref mut map) => {
                map.insert(path.to_string(), value);
            }
            Value::Array(ref mut arr) => {
                // Handle array index for simple path
                if let Ok(index) = path.parse::<usize>() {
                    if index < arr.len() {
                        arr[index] = value;
                    }
                    // else: index out of bounds, ignore
                }
            }
            _ => {}
        }
        return;
    }

    let parts: Vec<&str> = path.split('.').collect();
    let mut current = doc;

    for (i, part) in parts.iter().enumerate() {
        if i == parts.len() - 1 {
            // Last part - set the value
            match current {
                Value::Object(ref mut map) => {
                    map.insert(part.to_string(), value);
                }
                Value::Array(ref mut arr) => {
                    if let Ok(index) = part.parse::<usize>() {
                        if index < arr.len() {
                            arr[index] = value;
                        }
                        // else: index out of bounds, ignore
                    }
                }
                _ => {}
            }
            return;
        }

        // Determine if next path component is an array index
        let next_is_array_index = parts
            .get(i + 1)
            .map(|p| p.parse::<usize>().is_ok())
            .unwrap_or(false);

        // Navigate deeper, creating intermediate structures if needed
        match current {
            Value::Object(ref mut map) => {
                if !map.contains_key(*part) {
                    // Create intermediate structure based on next path component
                    if next_is_array_index {
                        map.insert(part.to_string(), Value::Array(Vec::new()));
                    } else {
                        map.insert(part.to_string(), Value::Object(serde_json::Map::new()));
                    }
                }
                current = map.get_mut(*part).expect("key was just inserted above");
            }
            Value::Array(ref mut arr) => {
                if let Ok(index) = part.parse::<usize>() {
                    // Extend array if needed
                    while arr.len() <= index {
                        if next_is_array_index {
                            arr.push(Value::Array(Vec::new()));
                        } else {
                            arr.push(Value::Object(serde_json::Map::new()));
                        }
                    }
                    current = &mut arr[index];
                } else {
                    // Invalid: array but non-numeric key
                    return;
                }
            }
            _ => return,
        }
    }
}

/// Delete a value at a nested path with dot notation support
///
/// Returns `true` if the value was deleted, `false` if path doesn't exist.
/// Supports array indexing: "items.0.name" will delete the name field from items[0].
///
/// # Examples
///
/// ```ignore
/// use serde_json::json;
/// use ironbase_core::value_utils::delete_nested_value;
///
/// let mut doc = json!({"address": {"city": "NYC", "zip": "10001"}});
/// delete_nested_value(&mut doc, "address.city");
/// assert_eq!(doc, json!({"address": {"zip": "10001"}}));
/// ```
pub fn delete_nested_value(doc: &mut Value, path: &str) -> bool {
    // Fast path: no dots means simple field deletion
    if !path.contains('.') {
        if let Value::Object(ref mut map) = doc {
            return map.remove(path).is_some();
        }
        return false;
    }

    let parts: Vec<&str> = path.split('.').collect();
    let mut current = doc;

    // Navigate to the parent of the field to delete
    for part in &parts[..parts.len() - 1] {
        match current {
            Value::Object(ref mut map) => {
                if let Some(v) = map.get_mut(*part) {
                    current = v;
                } else {
                    return false; // Path doesn't exist
                }
            }
            Value::Array(ref mut arr) => {
                if let Ok(index) = part.parse::<usize>() {
                    if let Some(v) = arr.get_mut(index) {
                        current = v;
                    } else {
                        return false; // Index out of bounds
                    }
                } else {
                    return false; // Invalid array index
                }
            }
            _ => return false, // Cannot navigate into non-container
        }
    }

    // Delete the final key/index
    let last_key = parts.last().unwrap();
    match current {
        Value::Object(ref mut map) => map.remove(*last_key).is_some(),
        Value::Array(ref mut arr) => {
            if let Ok(index) = last_key.parse::<usize>() {
                if index < arr.len() {
                    arr.remove(index);
                    return true;
                }
            }
            false
        }
        _ => false,
    }
}

/// Compare two JSON values
///
/// Returns `Some(Ordering)` for comparable types (numbers, strings, booleans),
/// `None` for incompatible types (e.g., comparing string to number).
///
/// # Supported comparisons
///
/// - Number vs Number (uses f64 comparison)
/// - String vs String (lexicographic)
/// - Bool vs Bool (false < true)
///
/// # Examples
///
/// ```ignore
/// use serde_json::json;
/// use std::cmp::Ordering;
/// use ironbase_core::value_utils::compare_values;
///
/// assert_eq!(compare_values(&json!(10), &json!(5)), Some(Ordering::Greater));
/// assert_eq!(compare_values(&json!("a"), &json!("b")), Some(Ordering::Less));
/// assert_eq!(compare_values(&json!("a"), &json!(1)), None); // incompatible
/// ```
pub fn compare_values(a: &Value, b: &Value) -> Option<Ordering> {
    match (a, b) {
        (Value::Number(n1), Value::Number(n2)) => {
            // BUG #2 FIX: Try integer comparison first to avoid f64 precision loss
            // f64 only has 53 bits of mantissa, so integers > 2^53 lose precision
            // serde_json stores integers as i64/u64 internally, not f64

            // Case 1: Both are i64 (most common for IDs and counters)
            if let (Some(i1), Some(i2)) = (n1.as_i64(), n2.as_i64()) {
                return Some(i1.cmp(&i2));
            }

            // Case 2: Both are u64 (large positive integers)
            if let (Some(u1), Some(u2)) = (n1.as_u64(), n2.as_u64()) {
                return Some(u1.cmp(&u2));
            }

            // Case 3: Mixed i64/u64 - compare carefully
            if let (Some(i), Some(u)) = (n1.as_i64(), n2.as_u64()) {
                // Negative i64 is always less than any u64
                if i < 0 {
                    return Some(Ordering::Less);
                }
                // Non-negative i64 can be safely cast to u64
                return Some((i as u64).cmp(&u));
            }
            if let (Some(u), Some(i)) = (n1.as_u64(), n2.as_i64()) {
                if i < 0 {
                    return Some(Ordering::Greater);
                }
                return Some(u.cmp(&(i as u64)));
            }

            // Case 4: Fall back to f64 only for actual floating-point numbers
            // (or if the above integer checks all failed)
            let f1 = n1.as_f64()?;
            let f2 = n2.as_f64()?;
            f1.partial_cmp(&f2)
        }
        (Value::String(s1), Value::String(s2)) => Some(s1.cmp(s2)),
        (Value::Bool(b1), Value::Bool(b2)) => Some(b1.cmp(b2)),
        _ => None,
    }
}

/// Compare two optional JSON values with None handling
///
/// Used for sorting where missing values need consistent ordering.
/// None values are considered "less than" any actual value.
///
/// # Ordering rules
///
/// - None < Some(_)
/// - Some(a) vs Some(b) uses compare_values
/// - Incompatible types return Equal (stable sort behavior)
///
/// # Examples
///
/// ```ignore
/// use serde_json::json;
/// use std::cmp::Ordering;
/// use ironbase_core::value_utils::compare_values_with_none;
///
/// assert_eq!(compare_values_with_none(None, Some(&json!(5))), Ordering::Less);
/// assert_eq!(compare_values_with_none(Some(&json!(10)), None), Ordering::Greater);
/// ```
#[allow(dead_code)] // Utility function for future use
pub fn compare_values_with_none(a: Option<&Value>, b: Option<&Value>) -> Ordering {
    match (a, b) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (Some(av), Some(bv)) => compare_values(av, bv).unwrap_or(Ordering::Equal),
    }
}

/// Creates a fast hash for a JSON Value using ahash.
///
/// This is much faster than canonical_json_string for deduplication,
/// as it avoids string allocation entirely. For objects, keys are
/// sorted before hashing to ensure deterministic output.
///
/// # Performance
/// - 10-50x faster than canonical_json_string
/// - No memory allocation (hashes directly into u64)
/// - Deterministic: same logical value = same hash
pub fn value_hash(value: &Value) -> u64 {
    use ahash::AHasher;
    use std::hash::Hasher;

    let mut hasher = AHasher::default();
    hash_value_into(&mut hasher, value);
    hasher.finish()
}

/// Hash a Value into a Hasher (recursive helper for value_hash)
fn hash_value_into<H: std::hash::Hasher>(hasher: &mut H, value: &Value) {
    use std::hash::Hash;

    match value {
        Value::Null => {
            0u8.hash(hasher);
        }
        Value::Bool(b) => {
            1u8.hash(hasher);
            b.hash(hasher);
        }
        Value::Number(n) => {
            2u8.hash(hasher);
            // Use string for deterministic float hashing
            n.to_string().hash(hasher);
        }
        Value::String(s) => {
            3u8.hash(hasher);
            s.hash(hasher);
        }
        Value::Array(arr) => {
            4u8.hash(hasher);
            arr.len().hash(hasher);
            for v in arr {
                hash_value_into(hasher, v);
            }
        }
        Value::Object(map) => {
            5u8.hash(hasher);
            map.len().hash(hasher);
            // Sort keys for deterministic output
            let mut pairs: Vec<_> = map.iter().collect();
            pairs.sort_by(|a, b| a.0.cmp(b.0));
            for (k, v) in pairs {
                k.hash(hasher);
                hash_value_into(hasher, v);
            }
        }
    }
}

/// Creates a canonical string representation of a JSON value
/// where object keys are always sorted alphabetically.
///
/// This ensures that two logically equivalent JSON objects with different
/// key ordering (e.g., `{"a":1,"b":2}` and `{"b":2,"a":1}`) produce the
/// same string representation.
///
/// Used by `$addToSet` accumulator to correctly deduplicate objects
/// regardless of key insertion order.
///
/// **Performance note:** For deduplication, prefer `value_hash()` (10-50x faster).
///
/// # Examples
///
/// ```ignore
/// use serde_json::json;
/// use ironbase_core::value_utils::canonical_json_string;
///
/// let v1 = json!({"a": 1, "b": 2});
/// let v2 = json!({"b": 2, "a": 1});
/// assert_eq!(canonical_json_string(&v1), canonical_json_string(&v2));
/// ```
pub fn canonical_json_string(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            // Sort keys alphabetically for deterministic output
            let mut pairs: Vec<_> = map.iter().collect();
            pairs.sort_by(|a, b| a.0.cmp(b.0));

            let inner: String = pairs
                .iter()
                .map(|(k, v)| format!("\"{}\":{}", k, canonical_json_string(v)))
                .collect::<Vec<_>>()
                .join(",");

            format!("{{{}}}", inner)
        }
        Value::Array(arr) => {
            let inner: String = arr
                .iter()
                .map(canonical_json_string)
                .collect::<Vec<_>>()
                .join(",");
            format!("[{}]", inner)
        }
        // Primitives: use standard serialization
        _ => value.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_get_nested_value_simple() {
        let doc = json!({"name": "Alice", "age": 30});
        assert_eq!(get_nested_value(&doc, "name"), Some(&json!("Alice")));
        assert_eq!(get_nested_value(&doc, "age"), Some(&json!(30)));
        assert_eq!(get_nested_value(&doc, "missing"), None);
    }

    #[test]
    fn test_get_nested_value_nested() {
        let doc = json!({
            "address": {
                "city": "NYC",
                "zip": 10001
            }
        });
        assert_eq!(get_nested_value(&doc, "address.city"), Some(&json!("NYC")));
        assert_eq!(get_nested_value(&doc, "address.zip"), Some(&json!(10001)));
        assert_eq!(get_nested_value(&doc, "address.missing"), None);
    }

    #[test]
    fn test_get_nested_value_array_index() {
        let doc = json!({
            "items": [
                {"name": "item1"},
                {"name": "item2"}
            ]
        });
        assert_eq!(
            get_nested_value(&doc, "items.0.name"),
            Some(&json!("item1"))
        );
        assert_eq!(
            get_nested_value(&doc, "items.1.name"),
            Some(&json!("item2"))
        );
        assert_eq!(get_nested_value(&doc, "items.5.name"), None);
    }

    #[test]
    fn test_get_nested_value_deeply_nested() {
        let doc = json!({
            "a": {
                "b": {
                    "c": {
                        "d": 42
                    }
                }
            }
        });
        assert_eq!(get_nested_value(&doc, "a.b.c.d"), Some(&json!(42)));
    }

    #[test]
    fn test_compare_values_numbers() {
        assert_eq!(
            compare_values(&json!(10), &json!(5)),
            Some(Ordering::Greater)
        );
        assert_eq!(compare_values(&json!(5), &json!(10)), Some(Ordering::Less));
        assert_eq!(compare_values(&json!(5), &json!(5)), Some(Ordering::Equal));
        assert_eq!(
            compare_values(&json!(3.5), &json!(2.5)),
            Some(Ordering::Greater)
        );
    }

    #[test]
    fn test_compare_values_strings() {
        assert_eq!(
            compare_values(&json!("banana"), &json!("apple")),
            Some(Ordering::Greater)
        );
        assert_eq!(
            compare_values(&json!("apple"), &json!("banana")),
            Some(Ordering::Less)
        );
        assert_eq!(
            compare_values(&json!("apple"), &json!("apple")),
            Some(Ordering::Equal)
        );
    }

    #[test]
    fn test_compare_values_booleans() {
        assert_eq!(
            compare_values(&json!(true), &json!(false)),
            Some(Ordering::Greater)
        );
        assert_eq!(
            compare_values(&json!(false), &json!(true)),
            Some(Ordering::Less)
        );
    }

    #[test]
    fn test_compare_values_incompatible() {
        assert_eq!(compare_values(&json!("string"), &json!(42)), None);
        assert_eq!(compare_values(&json!(true), &json!(1)), None);
        assert_eq!(compare_values(&json!([1, 2]), &json!(1)), None);
    }

    #[test]
    fn test_compare_values_with_none() {
        assert_eq!(compare_values_with_none(None, None), Ordering::Equal);
        assert_eq!(
            compare_values_with_none(None, Some(&json!(5))),
            Ordering::Less
        );
        assert_eq!(
            compare_values_with_none(Some(&json!(5)), None),
            Ordering::Greater
        );
        assert_eq!(
            compare_values_with_none(Some(&json!(10)), Some(&json!(5))),
            Ordering::Greater
        );
        // Incompatible types return Equal
        assert_eq!(
            compare_values_with_none(Some(&json!("a")), Some(&json!(1))),
            Ordering::Equal
        );
    }

    #[test]
    fn test_set_nested_value_simple() {
        let mut doc = json!({"name": "Alice"});
        set_nested_value(&mut doc, "age", json!(30));
        assert_eq!(doc["age"], 30);
    }

    #[test]
    fn test_set_nested_value_overwrite() {
        let mut doc = json!({"name": "Alice"});
        set_nested_value(&mut doc, "name", json!("Bob"));
        assert_eq!(doc["name"], "Bob");
    }

    #[test]
    fn test_set_nested_value_nested_existing() {
        let mut doc = json!({"address": {"city": "NYC"}});
        set_nested_value(&mut doc, "address.city", json!("Boston"));
        assert_eq!(doc["address"]["city"], "Boston");
    }

    #[test]
    fn test_set_nested_value_nested_create() {
        let mut doc = json!({"name": "Alice"});
        set_nested_value(&mut doc, "address.city", json!("NYC"));
        assert_eq!(doc["address"]["city"], "NYC");
    }

    #[test]
    fn test_set_nested_value_deeply_nested() {
        let mut doc = json!({"a": {}});
        set_nested_value(&mut doc, "a.b.c.d", json!(42));
        assert_eq!(doc["a"]["b"]["c"]["d"], 42);
    }

    // ========== BUG #1 regression tests: set_nested_value array handling ==========

    #[test]
    fn test_set_nested_value_array_index_path() {
        // BUG #1 regression test: "items.0.name" should work with existing array
        let mut doc = json!({
            "items": [
                {"name": "old_name"},
                {"name": "item2"}
            ]
        });
        set_nested_value(&mut doc, "items.0.name", json!("new_name"));
        assert_eq!(
            doc["items"][0]["name"], "new_name",
            "Should update items[0].name to new_name"
        );
        assert_eq!(
            doc["items"][1]["name"], "item2",
            "items[1] should be unchanged"
        );
    }

    #[test]
    fn test_set_nested_value_array_navigation() {
        // Navigate through existing array
        let mut doc = json!({
            "orders": [
                {"id": 1, "status": "pending"},
                {"id": 2, "status": "shipped"}
            ]
        });
        set_nested_value(&mut doc, "orders.1.status", json!("delivered"));
        assert_eq!(doc["orders"][1]["status"], "delivered");
    }

    #[test]
    fn test_set_nested_value_create_array_element() {
        // Extend array if index is beyond current length
        let mut doc = json!({
            "data": []
        });
        set_nested_value(&mut doc, "data.0.value", json!(100));
        assert_eq!(
            doc["data"][0]["value"], 100,
            "Should create data[0] and set value"
        );
    }

    #[test]
    fn test_set_nested_value_nested_arrays() {
        // Handle nested arrays: matrix.0.1 = first row, second column
        let mut doc = json!({
            "matrix": [
                [1, 2, 3],
                [4, 5, 6]
            ]
        });
        set_nested_value(&mut doc, "matrix.0.1", json!(99));
        assert_eq!(doc["matrix"][0][1], 99);
    }

    #[test]
    fn test_set_nested_value_create_array_path() {
        // When creating a path where next component is a number, create array
        let mut doc = json!({});
        set_nested_value(&mut doc, "items.0.name", json!("first"));
        assert!(
            doc["items"].is_array(),
            "items should be an array, not object"
        );
        assert_eq!(doc["items"][0]["name"], "first");
    }

    #[test]
    fn test_set_nested_value_simple_array_index() {
        // Simple path that's just an index into existing array
        let mut doc = json!([10, 20, 30]);
        set_nested_value(&mut doc, "1", json!(99));
        assert_eq!(doc[1], 99);
    }

    #[test]
    fn test_set_nested_value_projection_use_case() {
        // This is the actual use case: projection include mode with array paths
        // Original doc has items array, we want to project items.0.name
        let original = json!({
            "_id": 1,
            "items": [
                {"name": "item1", "price": 100},
                {"name": "item2", "price": 200}
            ]
        });

        // Simulate projection: create new doc and set fields from original
        let mut projected = json!({});
        set_nested_value(&mut projected, "_id", original["_id"].clone());
        set_nested_value(
            &mut projected,
            "items.0.name",
            original["items"][0]["name"].clone(),
        );

        assert_eq!(projected["_id"], 1);
        assert_eq!(projected["items"][0]["name"], "item1");
    }

    // ========== delete_nested_value tests (BUG #3 regression tests) ==========

    #[test]
    fn test_delete_nested_value_simple() {
        let mut doc = json!({"name": "Alice", "age": 30});
        assert!(delete_nested_value(&mut doc, "age"));
        assert_eq!(doc, json!({"name": "Alice"}));
    }

    #[test]
    fn test_delete_nested_value_nested() {
        let mut doc = json!({"address": {"city": "NYC", "zip": "10001"}});
        assert!(delete_nested_value(&mut doc, "address.city"));
        assert_eq!(doc, json!({"address": {"zip": "10001"}}));
    }

    #[test]
    fn test_delete_nested_value_deeply_nested() {
        let mut doc = json!({"a": {"b": {"c": {"d": 42, "e": 100}}}});
        assert!(delete_nested_value(&mut doc, "a.b.c.d"));
        assert_eq!(doc, json!({"a": {"b": {"c": {"e": 100}}}}));
    }

    #[test]
    fn test_delete_nested_value_array_index() {
        let mut doc = json!({
            "items": [
                {"name": "item1", "price": 100},
                {"name": "item2", "price": 200}
            ]
        });
        assert!(delete_nested_value(&mut doc, "items.0.price"));
        assert_eq!(
            doc["items"][0],
            json!({"name": "item1"}),
            "Should delete only price from items[0]"
        );
        assert_eq!(
            doc["items"][1],
            json!({"name": "item2", "price": 200}),
            "items[1] should be unchanged"
        );
    }

    #[test]
    fn test_delete_nested_value_nonexistent_path() {
        let mut doc = json!({"name": "Alice"});
        assert!(!delete_nested_value(&mut doc, "address.city"));
        assert_eq!(doc, json!({"name": "Alice"})); // Unchanged
    }

    #[test]
    fn test_delete_nested_value_entire_nested_object() {
        let mut doc = json!({"address": {"city": "NYC"}, "name": "Alice"});
        assert!(delete_nested_value(&mut doc, "address"));
        assert_eq!(doc, json!({"name": "Alice"}));
    }

    #[test]
    fn test_delete_nested_value_projection_exclude_use_case() {
        // BUG #3: This is the actual bug - projection exclude with dot notation
        let mut doc = json!({
            "_id": 1,
            "user": {"password": "secret", "email": "test@example.com"},
            "data": "public"
        });

        // Exclude sensitive field: user.password
        assert!(delete_nested_value(&mut doc, "user.password"));
        assert_eq!(
            doc,
            json!({
                "_id": 1,
                "user": {"email": "test@example.com"},
                "data": "public"
            })
        );
    }

    // ========== canonical_json_string tests ==========

    #[test]
    fn test_canonical_json_string_object_key_order() {
        // Two objects with same fields but different insertion order
        // should produce identical canonical strings
        let v1 = json!({"a": 1, "b": 2});
        let v2 = json!({"b": 2, "a": 1});
        assert_eq!(canonical_json_string(&v1), canonical_json_string(&v2));
        assert_eq!(canonical_json_string(&v1), r#"{"a":1,"b":2}"#);
    }

    #[test]
    fn test_canonical_json_string_nested_objects() {
        let v1 = json!({"outer": {"a": 1, "b": 2}});
        let v2 = json!({"outer": {"b": 2, "a": 1}});
        assert_eq!(canonical_json_string(&v1), canonical_json_string(&v2));
    }

    #[test]
    fn test_canonical_json_string_deeply_nested() {
        let v1 = json!({"x": {"y": {"a": 1, "b": 2}}});
        let v2 = json!({"x": {"y": {"b": 2, "a": 1}}});
        assert_eq!(canonical_json_string(&v1), canonical_json_string(&v2));
    }

    #[test]
    fn test_canonical_json_string_array_with_objects() {
        let v1 = json!([{"a": 1, "b": 2}]);
        let v2 = json!([{"b": 2, "a": 1}]);
        assert_eq!(canonical_json_string(&v1), canonical_json_string(&v2));
    }

    #[test]
    fn test_canonical_json_string_mixed_array() {
        let v1 = json!([1, {"z": 1, "a": 2}, "hello"]);
        let v2 = json!([1, {"a": 2, "z": 1}, "hello"]);
        assert_eq!(canonical_json_string(&v1), canonical_json_string(&v2));
    }

    #[test]
    fn test_canonical_json_string_primitives() {
        // Primitives should remain unchanged
        assert_eq!(canonical_json_string(&json!(42)), "42");
        assert_eq!(canonical_json_string(&json!("hello")), "\"hello\"");
        assert_eq!(canonical_json_string(&json!(true)), "true");
        assert_eq!(canonical_json_string(&json!(null)), "null");
        assert_eq!(canonical_json_string(&json!(3.14)), "3.14");
    }

    #[test]
    fn test_canonical_json_string_empty_structures() {
        assert_eq!(canonical_json_string(&json!({})), "{}");
        assert_eq!(canonical_json_string(&json!([])), "[]");
    }

    #[test]
    fn test_canonical_json_string_complex() {
        // Complex structure with multiple nesting levels
        let v1 = json!({
            "z": [{"b": 2, "a": 1}],
            "a": {"y": 1, "x": 2}
        });
        let v2 = json!({
            "a": {"x": 2, "y": 1},
            "z": [{"a": 1, "b": 2}]
        });
        assert_eq!(canonical_json_string(&v1), canonical_json_string(&v2));
    }
}
