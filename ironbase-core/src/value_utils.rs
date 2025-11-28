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
}
