//! Common helper functions for MCP tool handlers

use crate::adapter::{FindOptions, IronBaseAdapter};
use crate::error::{ErrorCode, McpError, Result};
use serde_json::{json, Value};
use std::collections::HashMap;

/// Default limit for queries when no ScriptLimits provided
pub const DEFAULT_QUERY_LIMIT: usize = 10_000;

/// Check if the current request has been cancelled
///
/// Call this at the beginning of potentially long operations
/// to allow early exit when the client sends `notifications/cancelled`.
///
/// Returns Ok(()) if not cancelled, Err(RequestCancelled) if cancelled.
#[inline]
pub fn check_cancelled() -> Result<()> {
    if crate::execution::is_cancelled() {
        return Err(McpError::cancelled("Request was cancelled by client"));
    }
    Ok(())
}

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
///
/// NOTE: This is the legacy version that doesn't validate direction values.
/// For new code, use `parse_sort_value` which returns Result and validates that
/// direction is exactly 1 or -1.
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

/// Parse sort specification from Value to Vec<(String, i32)> with validation.
/// Supports both array format [["field", 1]] and object format {"field": 1}.
/// Returns error if direction is not exactly 1 or -1.
///
/// This is the canonical implementation - use this in all tool handlers.
pub fn parse_sort_value(sort: Option<Value>) -> Result<Option<Vec<(String, i32)>>> {
    match sort {
        None => Ok(None),
        Some(Value::Null) => Ok(None),
        Some(Value::Array(arr)) => {
            // Array format: [["field", 1], ["field2", -1]]
            let mut result = Vec::new();
            for item in arr {
                let pair = item.as_array().ok_or_else(|| {
                    McpError::invalid_params("Sort array items must be [field, direction] pairs")
                })?;
                if pair.len() != 2 {
                    return Err(McpError::invalid_params(
                        "Sort array items must have exactly 2 elements: [field, direction]",
                    ));
                }
                let field = pair[0]
                    .as_str()
                    .ok_or_else(|| McpError::invalid_params("Sort field name must be a string"))?;
                let direction = pair[1]
                    .as_i64()
                    .ok_or_else(|| McpError::invalid_params("Sort direction must be 1 or -1"))?;
                if direction != 1 && direction != -1 {
                    return Err(McpError::invalid_params(format!(
                        "Sort direction for '{}' must be 1 or -1, got {}",
                        field, direction
                    )));
                }
                result.push((field.to_string(), direction as i32));
            }
            if result.is_empty() {
                Ok(None)
            } else {
                Ok(Some(result))
            }
        }
        Some(Value::Object(map)) => {
            // Object format: {"field": 1, "field2": -1}
            let mut result = Vec::new();
            for (key, value) in map {
                let direction = value.as_i64().ok_or_else(|| {
                    McpError::invalid_params(format!(
                        "Sort direction for '{}' must be 1 or -1",
                        key
                    ))
                })?;
                if direction != 1 && direction != -1 {
                    return Err(McpError::invalid_params(format!(
                        "Sort direction for '{}' must be 1 or -1, got {}",
                        key, direction
                    )));
                }
                result.push((key, direction as i32));
            }
            if result.is_empty() {
                Ok(None)
            } else {
                Ok(Some(result))
            }
        }
        Some(_) => Err(McpError::invalid_params(
            "Sort must be an array [[\"field\", 1]] or object {\"field\": 1}",
        )),
    }
}

/// Parse projection Value to HashMap<String, i32> with validation.
/// Accepts object format like {"field": 1} or {"field": 0}.
/// Returns error if values are not 0 or 1.
///
/// This is the canonical implementation - use this in all tool handlers.
pub fn parse_projection_value(proj: Option<Value>) -> Result<Option<HashMap<String, i32>>> {
    match proj {
        None => Ok(None),
        Some(Value::Null) => Ok(None),
        Some(Value::Object(map)) => {
            let mut result = HashMap::new();
            for (key, value) in map {
                let v = if let Some(i) = value.as_i64() {
                    if i != 0 && i != 1 {
                        return Err(McpError::invalid_params(format!(
                            "Invalid projection value for '{}': expected 0 or 1, got {}",
                            key, i
                        )));
                    }
                    i as i32
                } else if let Some(f) = value.as_f64() {
                    // Use epsilon-based comparison for floating point values
                    if f.abs() < f64::EPSILON {
                        0
                    } else if (f - 1.0).abs() < f64::EPSILON {
                        1
                    } else {
                        return Err(McpError::invalid_params(format!(
                            "Invalid projection value for '{}': expected 0 or 1, got {}",
                            key, f
                        )));
                    }
                } else {
                    return Err(McpError::invalid_params(format!(
                        "Invalid projection value for '{}': expected 0 or 1, got {:?}",
                        key, value
                    )));
                };
                result.insert(key, v);
            }
            if result.is_empty() {
                Ok(None)
            } else {
                Ok(Some(result))
            }
        }
        Some(_) => Err(McpError::invalid_params(
            "Projection must be an object like {\"field\": 1} or {\"field\": 0}",
        )),
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
/// This wraps the params and calls parse_projection_value.
pub fn parse_projection(params: &Value) -> Result<Option<HashMap<String, i32>>> {
    parse_projection_value(params.get("projection").cloned())
}

/// Validate collection name
pub fn validate_collection_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(McpError::invalid_params("Collection name cannot be empty"));
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
        return Err(McpError::invalid_params("Script name cannot be empty"));
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
        s.parse::<u64>().map_err(|_| {
            McpError::invalid_params(format!("Invalid transaction_id format: '{}'", s))
        })
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

// ============================================================================
// Idempotent chunk import (shared by rag_document_import, embed_document,
// Rhai db_rag_import — see issue #67)
// ============================================================================

/// Policy for an existing `doc_id` when importing chunks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IfExists {
    /// Delete the previously-existing chunks for this doc_id, keep the new set (default).
    Replace,
    /// If chunks already exist for this doc_id, do nothing.
    Skip,
    /// If chunks already exist for this doc_id, return an error.
    Error,
    /// Always insert, leaving any existing chunks in place (legacy behavior).
    Append,
}

impl IfExists {
    /// Parse from the `if_exists` parameter. Unknown/missing → `Replace` (idempotent default).
    pub fn parse(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "skip" => Self::Skip,
            "error" => Self::Error,
            "append" => Self::Append,
            _ => Self::Replace,
        }
    }
}

/// Default for serde-derived params.
pub fn default_if_exists() -> String {
    "replace".to_string()
}

/// Outcome of an idempotent chunk insert.
pub struct IdempotentInsert {
    /// Newly inserted chunk ids (empty when skipped).
    pub inserted_ids: Vec<String>,
    /// Number of previously-existing chunks removed (Replace only).
    pub replaced: u64,
    /// True when the Skip policy short-circuited because chunks already existed.
    pub skipped: bool,
}

/// Insert chunk documents for `doc_id` with an idempotency policy.
///
/// Safe ordering (issue #67): the previously-existing chunk ids are captured
/// first (projection: `_id` only, so embeddings are never loaded), then the new
/// chunks are inserted, and only afterwards are the old chunks deleted. A failure
/// during insert leaves the old chunks intact (no data loss); a failure during
/// the final delete leaves a temporary duplicate that the next import self-heals.
///
/// `documents` must be non-empty (callers guard the empty-chunk case earlier).
pub fn insert_chunks_idempotent(
    adapter: &IronBaseAdapter,
    collection: &str,
    doc_id: &str,
    documents: Vec<Value>,
    if_exists: IfExists,
) -> Result<IdempotentInsert> {
    // Capture existing chunk ids for this doc_id (skip the lookup for Append).
    let existing_ids: Vec<Value> = if if_exists == IfExists::Append {
        Vec::new()
    } else {
        // Project only `_id` so the (potentially large) embedding fields are never loaded.
        let opts = FindOptions {
            projection: Some(json!({ "_id": 1 })),
            sort: None,
            limit: None,
            skip: None,
            include_total: false,
            max_response_bytes: None,
            cancel_flag: None,
        };
        match adapter.find(collection, json!({ "doc_id": doc_id }), opts) {
            Ok(res) => res
                .documents
                .into_iter()
                .filter_map(|mut d| d.get_mut("_id").map(std::mem::take))
                .collect(),
            // A missing collection simply means there is nothing to replace yet;
            // insert_many below will create it. Any other error propagates.
            Err(e) if e.code == ErrorCode::CollectionNotFound => Vec::new(),
            Err(e) => return Err(e),
        }
    };

    if !existing_ids.is_empty() {
        match if_exists {
            IfExists::Error => {
                return Err(McpError::invalid_params(format!(
                    "doc_id '{}' already exists ({} chunks). Use if_exists=replace to overwrite, \
                     or if_exists=append to add alongside.",
                    doc_id,
                    existing_ids.len()
                )));
            }
            IfExists::Skip => {
                return Ok(IdempotentInsert {
                    inserted_ids: Vec::new(),
                    replaced: 0,
                    skipped: true,
                });
            }
            _ => {}
        }
    }

    // Insert the new chunks first — old chunks stay intact if this fails.
    let inserted_ids = adapter.insert_many(collection, documents)?;

    // Replace: remove the previously-existing chunks now that the new set is in.
    let replaced = if if_exists == IfExists::Replace && !existing_ids.is_empty() {
        adapter.delete_many(collection, json!({ "_id": { "$in": existing_ids } }))?
    } else {
        0
    };

    Ok(IdempotentInsert {
        inserted_ids,
        replaced,
        skipped: false,
    })
}

// ============================================================================
// Document/Filter Validation (centralized)
// ============================================================================

/// Validate that a value is a JSON object (for documents)
pub fn validate_document(doc: &Value) -> Result<()> {
    if !doc.is_object() {
        return Err(McpError::invalid_params("document must be a JSON object"));
    }
    Ok(())
}

/// Validate that a value is a JSON object (for filters/queries)
pub fn validate_filter(filter: &Value) -> Result<()> {
    if !filter.is_object() {
        return Err(McpError::invalid_params("filter must be a JSON object"));
    }
    Ok(())
}

/// Validate that a value is a JSON object (for update operations)
pub fn validate_update(update: &Value) -> Result<()> {
    if !update.is_object() {
        return Err(McpError::invalid_params("update must be a JSON object"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn temp_adapter() -> (IronBaseAdapter, TempDir) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("idem.mlite");
        let adapter = IronBaseAdapter::new(path.to_string_lossy().to_string()).unwrap();
        (adapter, dir)
    }

    fn seed(adapter: &IronBaseAdapter, doc_id: &str, n: usize) {
        let docs: Vec<Value> = (0..n)
            .map(|i| json!({ "doc_id": doc_id, "chunk_index": i, "content": format!("c{i}") }))
            .collect();
        adapter.insert_many("rdocs", docs).unwrap();
    }

    fn new_chunks(doc_id: &str, n: usize) -> Vec<Value> {
        (0..n)
            .map(|i| json!({ "doc_id": doc_id, "chunk_index": i, "content": format!("new{i}") }))
            .collect()
    }

    fn count(adapter: &IronBaseAdapter, doc_id: &str) -> u64 {
        adapter
            .count_documents("rdocs", json!({ "doc_id": doc_id }))
            .unwrap()
    }

    #[test]
    fn test_if_exists_parse_default_replace() {
        assert_eq!(IfExists::parse("replace"), IfExists::Replace);
        assert_eq!(IfExists::parse("SKIP"), IfExists::Skip);
        assert_eq!(IfExists::parse("error"), IfExists::Error);
        assert_eq!(IfExists::parse("append"), IfExists::Append);
        assert_eq!(IfExists::parse("nonsense"), IfExists::Replace);
        assert_eq!(IfExists::parse(""), IfExists::Replace);
    }

    #[test]
    fn test_replace_is_idempotent() {
        let (adapter, _d) = temp_adapter();
        seed(&adapter, "A", 3);
        // Re-import the same doc_id with a fresh 2-chunk set.
        let out = insert_chunks_idempotent(
            &adapter,
            "rdocs",
            "A",
            new_chunks("A", 2),
            IfExists::Replace,
        )
        .unwrap();
        assert_eq!(out.inserted_ids.len(), 2);
        assert_eq!(out.replaced, 3);
        assert!(!out.skipped);
        // Only the new set remains — no duplication (issue #67).
        assert_eq!(count(&adapter, "A"), 2);
    }

    #[test]
    fn test_replace_on_fresh_doc_id_just_inserts() {
        let (adapter, _d) = temp_adapter();
        let out = insert_chunks_idempotent(
            &adapter,
            "rdocs",
            "B",
            new_chunks("B", 4),
            IfExists::Replace,
        )
        .unwrap();
        assert_eq!(out.inserted_ids.len(), 4);
        assert_eq!(out.replaced, 0);
        assert_eq!(count(&adapter, "B"), 4);
    }

    #[test]
    fn test_skip_leaves_existing_untouched() {
        let (adapter, _d) = temp_adapter();
        seed(&adapter, "A", 3);
        let out =
            insert_chunks_idempotent(&adapter, "rdocs", "A", new_chunks("A", 2), IfExists::Skip)
                .unwrap();
        assert!(out.skipped);
        assert_eq!(out.inserted_ids.len(), 0);
        assert_eq!(count(&adapter, "A"), 3);
    }

    #[test]
    fn test_error_when_exists() {
        let (adapter, _d) = temp_adapter();
        seed(&adapter, "A", 3);
        let res =
            insert_chunks_idempotent(&adapter, "rdocs", "A", new_chunks("A", 2), IfExists::Error);
        assert!(res.is_err());
        assert_eq!(count(&adapter, "A"), 3);
    }

    #[test]
    fn test_append_adds_alongside() {
        let (adapter, _d) = temp_adapter();
        seed(&adapter, "A", 3);
        let out =
            insert_chunks_idempotent(&adapter, "rdocs", "A", new_chunks("A", 2), IfExists::Append)
                .unwrap();
        assert_eq!(out.inserted_ids.len(), 2);
        assert_eq!(out.replaced, 0);
        // Legacy behavior: old + new coexist.
        assert_eq!(count(&adapter, "A"), 5);
    }
}
