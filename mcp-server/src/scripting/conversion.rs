//! JSON ↔ Rhai Dynamic conversion utilities.
//!
//! Provides bidirectional conversion between `serde_json::Value` and Rhai `Dynamic` types.

use rhai::{Dynamic, Map};
use serde_json::Value;

/// Convert serde_json::Value to Rhai Dynamic.
///
/// Performs recursive conversion of JSON values to their Rhai equivalents:
/// - `null` → `Dynamic::UNIT`
/// - `bool` → Rhai boolean
/// - `number` → Rhai i64 or f64
/// - `string` → Rhai String
/// - `array` → Rhai Array (Vec<Dynamic>)
/// - `object` → Rhai Map
///
/// # Arguments
///
/// * `value` - The JSON value to convert
///
/// # Returns
///
/// The equivalent Rhai Dynamic value
///
/// # Example
///
/// ```rust,ignore
/// use serde_json::json;
///
/// let json = json!({"name": "Alice", "age": 30});
/// let dynamic = json_to_dynamic(&json);
/// ```
pub fn json_to_dynamic(value: &Value) -> Dynamic {
    match value {
        Value::Null => Dynamic::UNIT,
        Value::Bool(b) => Dynamic::from(*b),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Dynamic::from(i)
            } else if let Some(f) = n.as_f64() {
                Dynamic::from(f)
            } else {
                Dynamic::UNIT
            }
        }
        Value::String(s) => Dynamic::from(s.clone()),
        Value::Array(arr) => {
            let vec: Vec<Dynamic> = arr.iter().map(json_to_dynamic).collect();
            Dynamic::from(vec)
        }
        Value::Object(obj) => {
            let mut map = Map::new();
            for (k, v) in obj {
                map.insert(k.clone().into(), json_to_dynamic(v));
            }
            Dynamic::from(map)
        }
    }
}

/// Convert Rhai Dynamic to serde_json::Value.
///
/// Performs recursive conversion of Rhai values to their JSON equivalents:
/// - `Dynamic::UNIT` → `null`
/// - Rhai boolean → JSON boolean
/// - Rhai i64 → JSON number
/// - Rhai f64 → JSON number
/// - Rhai String → JSON string
/// - Rhai Array → JSON array
/// - Rhai Map → JSON object
///
/// # Arguments
///
/// * `value` - The Rhai Dynamic value to convert
///
/// # Returns
///
/// The equivalent JSON value
///
/// # Example
///
/// ```rust,ignore
/// let dynamic = Dynamic::from(42);
/// let json = dynamic_to_json(&dynamic);
/// assert_eq!(json, serde_json::json!(42));
/// ```
pub fn dynamic_to_json(value: &Dynamic) -> Value {
    if value.is_unit() {
        Value::Null
    } else if value.is_bool() {
        Value::Bool(value.as_bool().unwrap_or(false))
    } else if value.is_int() {
        Value::Number(value.as_int().unwrap_or(0).into())
    } else if value.is_float() {
        if let Ok(f) = value.as_float() {
            serde_json::Number::from_f64(f)
                .map(Value::Number)
                .unwrap_or(Value::Null)
        } else {
            Value::Null
        }
    } else if value.is_string() {
        Value::String(value.clone().into_string().unwrap_or_default())
    } else if value.is_array() {
        let arr: Vec<Dynamic> = value.clone().into_array().unwrap_or_default();
        Value::Array(arr.iter().map(dynamic_to_json).collect())
    } else if value.is_map() {
        let map: Map = value.clone().cast::<Map>();
        let mut obj = serde_json::Map::new();
        for (k, v) in map {
            obj.insert(k.to_string(), dynamic_to_json(&v));
        }
        Value::Object(obj)
    } else {
        // Try to convert to string as fallback
        Value::String(value.to_string())
    }
}

/// Convert Rhai Map to serde_json::Value (always an object).
///
/// # Arguments
///
/// * `map` - The Rhai Map to convert
///
/// # Returns
///
/// A JSON object
pub fn map_to_json(map: &Map) -> Value {
    let mut obj = serde_json::Map::new();
    for (k, v) in map {
        obj.insert(k.to_string(), dynamic_to_json(v));
    }
    Value::Object(obj)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_json_to_dynamic_primitives() {
        assert!(json_to_dynamic(&json!(null)).is_unit());
        assert!(json_to_dynamic(&json!(true)).as_bool().unwrap());
        assert_eq!(json_to_dynamic(&json!(42)).as_int().unwrap(), 42);
        assert_eq!(json_to_dynamic(&json!(2.5)).as_float().unwrap(), 2.5);
        assert_eq!(
            json_to_dynamic(&json!("hello")).into_string().unwrap(),
            "hello"
        );
    }

    #[test]
    fn test_json_to_dynamic_array() {
        let json = json!([1, 2, 3]);
        let dyn_val = json_to_dynamic(&json);
        let arr = dyn_val.into_array().unwrap();
        assert_eq!(arr.len(), 3);
    }

    #[test]
    fn test_json_to_dynamic_object() {
        let json = json!({"name": "Alice"});
        let dyn_val = json_to_dynamic(&json);
        assert!(dyn_val.is_map());
    }

    #[test]
    fn test_dynamic_to_json_primitives() {
        assert_eq!(dynamic_to_json(&Dynamic::UNIT), json!(null));
        assert_eq!(dynamic_to_json(&Dynamic::from(true)), json!(true));
        assert_eq!(dynamic_to_json(&Dynamic::from(42_i64)), json!(42));
        assert_eq!(
            dynamic_to_json(&Dynamic::from("hello".to_string())),
            json!("hello")
        );
    }

    #[test]
    fn test_roundtrip() {
        let original = json!({
            "name": "Test",
            "values": [1, 2, 3],
            "nested": {"a": true}
        });
        let dynamic = json_to_dynamic(&original);
        let back = dynamic_to_json(&dynamic);
        assert_eq!(original, back);
    }
}
