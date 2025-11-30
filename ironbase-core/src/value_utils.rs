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
/// ```
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

/// Set a value at a nested path with dot notation support
///
/// Creates intermediate objects if they don't exist.
/// Used by $unwind to set the unwound element back into the document.
///
/// # Examples
///
/// ```
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
        if let Value::Object(ref mut map) = doc {
            map.insert(path.to_string(), value);
        }
        return;
    }

    let parts: Vec<&str> = path.split('.').collect();
    let mut current = doc;

    for (i, part) in parts.iter().enumerate() {
        if i == parts.len() - 1 {
            // Last part - set the value
            if let Value::Object(ref mut map) = current {
                map.insert(part.to_string(), value);
            }
            return;
        }

        // Navigate deeper, creating intermediate objects if needed
        if let Value::Object(ref mut map) = current {
            if !map.contains_key(*part) {
                map.insert(part.to_string(), Value::Object(serde_json::Map::new()));
            }
            current = map.get_mut(*part).unwrap();
        } else {
            // Cannot navigate into non-object
            return;
        }
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
/// ```
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
/// ```
/// use serde_json::json;
/// use std::cmp::Ordering;
/// use ironbase_core::value_utils::compare_values_with_none;
///
/// assert_eq!(compare_values_with_none(None, Some(&json!(5))), Ordering::Less);
/// assert_eq!(compare_values_with_none(Some(&json!(10)), None), Ordering::Greater);
/// ```
pub fn compare_values_with_none(a: Option<&Value>, b: Option<&Value>) -> Ordering {
    match (a, b) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (Some(av), Some(bv)) => compare_values(av, bv).unwrap_or(Ordering::Equal),
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
/// # Examples
///
/// ```
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
