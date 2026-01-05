//! Common helper functions for MCP tool handlers

use crate::error::{McpError, Result};
use serde_json::Value;
use std::collections::HashMap;

/// Default limit for queries when no ScriptLimits provided
pub const DEFAULT_QUERY_LIMIT: usize = 10_000;

/// Maximum collection name length
pub const MAX_COLLECTION_NAME_LEN: usize = 128;

/// Constant-time string comparison to prevent timing attacks
/// SECURITY FIX: Length comparison is now also constant-time to prevent
/// attackers from determining the key length via timing analysis.
pub fn constant_time_compare(a: &[u8], b: &[u8]) -> bool {
    // Use the longer length to prevent length-based timing leak
    let max_len = a.len().max(b.len());

    // Track length mismatch (will be combined at the end)
    let len_mismatch = if a.len() != b.len() { 1u8 } else { 0u8 };

    let mut result = 0u8;
    for i in 0..max_len {
        // Use 0 as default for out-of-bounds (constant-time)
        let a_byte = if i < a.len() { a[i] } else { 0 };
        let b_byte = if i < b.len() { b[i] } else { 0 };
        result |= a_byte ^ b_byte;
    }

    // Combine XOR result with length mismatch
    (result | len_mismatch) == 0
}

/// Verify admin key from params against IRONBASE_ADMIN_KEY env var
/// SECURITY FIX: Use generic error message to prevent enumeration attacks.
/// Attacker cannot determine if IRONBASE_ADMIN_KEY is set or not.
pub fn verify_admin_key(params: &Value) -> Result<()> {
    // Generic error message for all admin auth failures
    const ADMIN_AUTH_ERROR: &str = "Admin authentication failed";

    let expected = match std::env::var("IRONBASE_ADMIN_KEY") {
        Ok(key) if !key.is_empty() => key,
        _ => return Err(McpError::invalid_params(ADMIN_AUTH_ERROR)),
    };

    let provided = params
        .get("admin_key")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    // Use constant-time comparison to prevent timing attacks
    // Even if provided is empty, we still do the comparison
    if provided.is_empty() || !constant_time_compare(provided.as_bytes(), expected.as_bytes()) {
        return Err(McpError::invalid_params(ADMIN_AUTH_ERROR));
    }
    Ok(())
}

/// Verify admin key from typed Option<String> param
/// SECURITY FIX: Use generic error message to prevent enumeration attacks.
pub fn verify_admin_key_opt(admin_key: Option<&str>) -> Result<()> {
    const ADMIN_AUTH_ERROR: &str = "Admin authentication failed";

    let expected = match std::env::var("IRONBASE_ADMIN_KEY") {
        Ok(key) if !key.is_empty() => key,
        _ => return Err(McpError::invalid_params(ADMIN_AUTH_ERROR)),
    };

    let provided = admin_key.unwrap_or("");

    if provided.is_empty() || !constant_time_compare(provided.as_bytes(), expected.as_bytes()) {
        return Err(McpError::invalid_params(ADMIN_AUTH_ERROR));
    }
    Ok(())
}

/// Validate and parse limit, capping at the provided max
pub fn parse_limit_with_max(params: &Value, max_limit: usize) -> Option<usize> {
    params
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|v| (v as usize).min(max_limit))
}

/// Validate and parse limit, capping at DEFAULT_QUERY_LIMIT
pub fn parse_limit(params: &Value) -> Option<usize> {
    parse_limit_with_max(params, DEFAULT_QUERY_LIMIT)
}

/// Validate and parse skip
pub fn parse_skip(params: &Value) -> Option<usize> {
    params
        .get("skip")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
}

/// Parse sort parameter - returns None if missing, null, or empty
/// Accepts two formats:
/// - Array: [["field", 1], ["field2", -1]]
/// - Object: {"field": 1, "field2": -1}
pub fn parse_sort(params: &Value) -> Option<Vec<(String, i32)>> {
    let sort_value = params.get("sort")?;

    // null → None
    if sort_value.is_null() {
        return None;
    }

    let sort_vec: Vec<(String, i32)> = if let Some(arr) = sort_value.as_array() {
        // Array format: [["field", 1], ["field2", -1]]
        arr.iter()
            .filter_map(|item| {
                let pair = item.as_array()?;
                if pair.len() == 2 {
                    let field = pair[0].as_str()?.to_string();
                    let dir = pair[1].as_i64()? as i32;
                    Some((field, dir))
                } else {
                    None
                }
            })
            .collect()
    } else if let Some(obj) = sort_value.as_object() {
        // Object format: {"field": 1, "field2": -1}
        obj.iter()
            .map(|(k, v)| (k.clone(), v.as_i64().unwrap_or(1) as i32))
            .collect()
    } else {
        return None;
    };

    // Empty → None (enables O(1) skip/limit)
    if sort_vec.is_empty() {
        None
    } else {
        Some(sort_vec)
    }
}

/// Validate threshold is in range [0.0, 1.0]
pub fn parse_threshold(params: &Value) -> Result<Option<f64>> {
    match params.get("threshold").and_then(|v| v.as_f64()) {
        Some(t) if !(0.0..=1.0).contains(&t) => Err(McpError::invalid_params(format!(
            "threshold must be between 0.0 and 1.0, got: {}",
            t
        ))),
        t => Ok(t),
    }
}

/// Parse projection: {"field": 1} or {"field": 0}
pub fn parse_projection(params: &Value) -> Result<Option<HashMap<String, i32>>> {
    if let Some(proj_value) = params.get("projection") {
        if proj_value.is_null() {
            Ok(None)
        } else if let Some(obj) = proj_value.as_object() {
            let mut map = HashMap::new();
            for (k, v) in obj {
                let int_val = if let Some(i) = v.as_i64() {
                    if i != 0 && i != 1 {
                        return Err(McpError::invalid_params(format!(
                            "Invalid projection value for '{}': expected 0 or 1, got {}",
                            k, i
                        )));
                    }
                    i as i32
                } else if let Some(f) = v.as_f64() {
                    if f == 0.0 {
                        0
                    } else if f == 1.0 {
                        1
                    } else {
                        return Err(McpError::invalid_params(format!(
                            "Invalid projection value for '{}': expected 0 or 1, got {}",
                            k, f
                        )));
                    }
                } else {
                    return Err(McpError::invalid_params(format!(
                        "Invalid projection value for '{}': expected 0 or 1, got {:?}",
                        k, v
                    )));
                };
                map.insert(k.clone(), int_val);
            }
            Ok(Some(map))
        } else {
            Err(McpError::invalid_params(
                "projection must be an object like {\"field\": 1} or {\"field\": 0}",
            ))
        }
    } else {
        Ok(None)
    }
}

/// Validate collection name
pub fn validate_collection_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(McpError::invalid_params(
            "Collection name cannot be empty",
        ));
    }
    if name.len() > MAX_COLLECTION_NAME_LEN {
        return Err(McpError::invalid_params(format!(
            "Collection name too long (max {} chars)",
            MAX_COLLECTION_NAME_LEN
        )));
    }

    // SECURITY FIX: Prevent users from creating collections that look like system collections
    // The '.' character is NOT allowed in user collection names to prevent:
    // - Creating fake system collections like "_system.custom"
    // - Path-like obfuscation attacks
    // System collections (_system.*) can only be created via admin_create_system_collection
    if name.contains('.') {
        return Err(McpError::invalid_params(
            "Collection name cannot contain dots. System collections can only be created via admin tools."
        ));
    }

    // Check for invalid characters (allow alphanumeric, underscore, hyphen)
    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
    {
        return Err(McpError::invalid_params(
            "Collection name can only contain alphanumeric characters, underscores, and hyphens",
        ));
    }
    Ok(())
}

/// BUG #12 fix: Validate script name (same rules as collection name)
pub fn validate_script_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(McpError::invalid_params(
            "Script name cannot be empty",
        ));
    }
    if name.len() > MAX_COLLECTION_NAME_LEN {
        return Err(McpError::invalid_params(format!(
            "Script name too long (max {} chars)",
            MAX_COLLECTION_NAME_LEN
        )));
    }
    // Check for invalid characters (allow alphanumeric, underscore, dot, hyphen)
    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '.' || c == '-')
    {
        return Err(McpError::invalid_params(
            "Script name can only contain alphanumeric characters, underscores, dots, and hyphens",
        ));
    }
    Ok(())
}

/// Get required string parameter
pub fn get_string(params: &Value, key: &str) -> Result<String> {
    params
        .get(key)
        .and_then(|v| v.as_str())
        .map(String::from)
        .ok_or_else(|| McpError::invalid_params(format!("Missing or invalid '{}' parameter", key)))
}

/// Get required object parameter
pub fn get_object(params: &Value, key: &str) -> Result<Value> {
    params
        .get(key)
        .filter(|v| v.is_object())
        .cloned()
        .ok_or_else(|| {
            McpError::invalid_params(format!(
                "Missing or invalid '{}' parameter (expected object)",
                key
            ))
        })
}

/// Get required array parameter
pub fn get_array(params: &Value, key: &str) -> Result<Vec<Value>> {
    params
        .get(key)
        .and_then(|v| v.as_array())
        .cloned()
        .ok_or_else(|| {
            McpError::invalid_params(format!(
                "Missing or invalid '{}' parameter (expected array)",
                key
            ))
        })
}

/// Parse transaction_id parameter (accepts both string and number)
pub fn parse_transaction_id(params: &Value) -> Result<u64> {
    let tx_param = params
        .get("transaction_id")
        .ok_or_else(|| McpError::invalid_params("transaction_id parameter is required"))?;

    // Accept both string and number formats
    if let Some(s) = tx_param.as_str() {
        s.parse::<u64>()
            .map_err(|_| McpError::invalid_params(format!("Invalid transaction_id format: '{}'", s)))
    } else if let Some(n) = tx_param.as_u64() {
        Ok(n)
    } else if let Some(n) = tx_param.as_i64() {
        if n >= 0 {
            Ok(n as u64)
        } else {
            Err(McpError::invalid_params(
                "transaction_id cannot be negative",
            ))
        }
    } else {
        Err(McpError::invalid_params(
            "transaction_id must be a number or string",
        ))
    }
}

/// Parse transaction ID from a String (for typed params)
pub fn parse_transaction_id_str(tx_id: &str) -> Result<u64> {
    tx_id.parse::<u64>().map_err(|_| {
        McpError::invalid_params(format!("Invalid transaction_id format: '{}'", tx_id))
    })
}
