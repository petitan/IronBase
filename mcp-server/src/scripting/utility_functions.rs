//! Utility function registrations for Rhai scripts.
//!
//! Provides helper functions for scripts:
//! - Error/null checking
//! - Base64 encoding/decoding
//! - Type checking
//! - UUID and timestamp generation

use base64::Engine as Base64Engine;
use rhai::{Dynamic, Engine};

/// Register all utility functions into a Rhai engine.
///
/// # Functions Registered
///
/// ## Error/Null Checking
/// - `is_error(value)` - Check if value is an error string
/// - `is_null(value)` - Check if value is null/unit
/// - `get_error(value)` - Extract error message (without "Error: " prefix)
/// - `unwrap_or(value, default)` - Return value if not error/null, else default
///
/// ## Base64
/// - `base64_encode(string)` - Encode string to base64
/// - `base64_decode(base64_string)` - Decode base64 to string (or error)
///
/// ## Type Checking
/// - `type_name(value)` - Get type name as string
///
/// ## UUID and Timestamp
/// - `uuid()` - Generate random UUID v4
/// - `timestamp()` - Current Unix timestamp in seconds
/// - `timestamp_ms()` - Current Unix timestamp in milliseconds
/// - `now_iso()` - Current time as ISO 8601 string
///
/// # Arguments
///
/// * `engine` - The Rhai engine to register functions into
pub fn register_utility_functions(engine: &mut Engine) {
    register_error_functions(engine);
    register_base64_functions(engine);
    register_type_functions(engine);
    register_time_functions(engine);
}

/// Register error/null checking helper functions.
fn register_error_functions(engine: &mut Engine) {
    // is_error(value) -> bool
    // Check if value is an error string (starts with "Error:")
    engine.register_fn("is_error", |value: Dynamic| -> bool {
        if let Some(s) = value.clone().try_cast::<String>() {
            s.starts_with("Error:")
        } else {
            false
        }
    });

    // is_null(value) -> bool
    // Check if value is null/unit
    engine.register_fn("is_null", |value: Dynamic| -> bool {
        value.is_unit()
    });

    // get_error(value) -> string
    // Extract error message (without "Error: " prefix)
    engine.register_fn("get_error", |value: Dynamic| -> String {
        if let Some(s) = value.clone().try_cast::<String>() {
            if let Some(msg) = s.strip_prefix("Error: ") {
                msg.to_string()
            } else {
                s
            }
        } else {
            String::new()
        }
    });

    // unwrap_or(value, default) -> value if not error/null, else default
    engine.register_fn("unwrap_or", |value: Dynamic, default: Dynamic| -> Dynamic {
        if value.is_unit() {
            return default;
        }
        if let Some(s) = value.clone().try_cast::<String>() {
            if s.starts_with("Error:") {
                return default;
            }
        }
        value
    });
}

/// Register base64 encoding/decoding functions.
fn register_base64_functions(engine: &mut Engine) {
    // base64_encode(string) -> base64 encoded string
    engine.register_fn("base64_encode", |s: &str| -> String {
        base64::engine::general_purpose::STANDARD.encode(s.as_bytes())
    });

    // base64_decode(base64_string) -> decoded string or error
    engine.register_fn("base64_decode", |s: &str| -> Dynamic {
        match base64::engine::general_purpose::STANDARD.decode(s) {
            Ok(bytes) => match String::from_utf8(bytes) {
                Ok(decoded) => Dynamic::from(decoded),
                Err(e) => Dynamic::from(format!("Error: Invalid UTF-8: {}", e)),
            },
            Err(e) => Dynamic::from(format!("Error: Invalid base64: {}", e)),
        }
    });
}

/// Register type checking helper functions.
fn register_type_functions(engine: &mut Engine) {
    // type_name(value) -> string describing the type
    engine.register_fn("type_name", |value: Dynamic| -> String {
        if value.is_unit() {
            "null".to_string()
        } else if value.is_bool() {
            "bool".to_string()
        } else if value.is_int() {
            "int".to_string()
        } else if value.is_float() {
            "float".to_string()
        } else if value.is_string() {
            "string".to_string()
        } else if value.is_array() {
            "array".to_string()
        } else if value.is_map() {
            "map".to_string()
        } else {
            "unknown".to_string()
        }
    });
}

/// Register UUID and timestamp functions.
fn register_time_functions(engine: &mut Engine) {
    // uuid() -> random UUID v4 string
    engine.register_fn("uuid", || -> String {
        uuid::Uuid::new_v4().to_string()
    });

    // timestamp() -> current Unix timestamp in seconds (i64)
    engine.register_fn("timestamp", || -> i64 {
        chrono::Utc::now().timestamp()
    });

    // timestamp_ms() -> current Unix timestamp in milliseconds (i64)
    engine.register_fn("timestamp_ms", || -> i64 {
        chrono::Utc::now().timestamp_millis()
    });

    // now_iso() -> current time as ISO 8601 string
    engine.register_fn("now_iso", || -> String {
        chrono::Utc::now().to_rfc3339()
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_error() {
        let mut engine = Engine::new();
        register_utility_functions(&mut engine);

        let result: bool = engine.eval(r#"is_error("Error: something")"#).unwrap();
        assert!(result);

        let result: bool = engine.eval(r#"is_error("Not an error")"#).unwrap();
        assert!(!result);
    }

    #[test]
    fn test_is_null() {
        let mut engine = Engine::new();
        register_utility_functions(&mut engine);

        let result: bool = engine.eval("is_null(())").unwrap();
        assert!(result);

        let result: bool = engine.eval("is_null(42)").unwrap();
        assert!(!result);
    }

    #[test]
    fn test_base64() {
        let mut engine = Engine::new();
        register_utility_functions(&mut engine);

        let encoded: String = engine.eval(r#"base64_encode("hello")"#).unwrap();
        assert_eq!(encoded, "aGVsbG8=");

        // Roundtrip
        let result: String = engine
            .eval(r#"base64_decode(base64_encode("test"))"#)
            .unwrap();
        assert_eq!(result, "test");
    }

    #[test]
    fn test_type_name() {
        let mut engine = Engine::new();
        register_utility_functions(&mut engine);

        let result: String = engine.eval("type_name(42)").unwrap();
        assert_eq!(result, "int");

        let result: String = engine.eval(r#"type_name("hello")"#).unwrap();
        assert_eq!(result, "string");
    }

    #[test]
    fn test_uuid() {
        let mut engine = Engine::new();
        register_utility_functions(&mut engine);

        let result: String = engine.eval("uuid()").unwrap();
        assert!(result.len() == 36); // UUID v4 format
    }
}
