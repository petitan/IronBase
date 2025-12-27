//! MCP Tool definitions and handlers for IronBase

use crate::adapter::{FindOptions, FulltextSearchOptions, IronBaseAdapter};
use crate::error::{McpError, Result};
use crate::scripting::{RhaiEngine, ScriptManager, ScriptOptions};
use ironbase_core::find_options::{apply_projection, apply_sort};
use serde_json::{json, Value};
use std::sync::Arc;

/// Maximum limit for queries (DoS protection)
const MAX_QUERY_LIMIT: usize = 10_000;

/// Maximum collection name length
const MAX_COLLECTION_NAME_LEN: usize = 128;

/// Constant-time string comparison to prevent timing attacks
/// SECURITY FIX: Length comparison is now also constant-time to prevent
/// attackers from determining the key length via timing analysis.
fn constant_time_compare(a: &[u8], b: &[u8]) -> bool {
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
fn verify_admin_key(params: &Value) -> Result<()> {
    // Generic error message for all admin auth failures
    const ADMIN_AUTH_ERROR: &str = "Admin authentication failed";

    let expected = match std::env::var("IRONBASE_ADMIN_KEY") {
        Ok(key) if !key.is_empty() => key,
        _ => return Err(McpError::InvalidParams(ADMIN_AUTH_ERROR.into())),
    };

    let provided = params
        .get("admin_key")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    // Use constant-time comparison to prevent timing attacks
    // Even if provided is empty, we still do the comparison
    if provided.is_empty() || !constant_time_compare(provided.as_bytes(), expected.as_bytes()) {
        return Err(McpError::InvalidParams(ADMIN_AUTH_ERROR.into()));
    }
    Ok(())
}

/// Validate and parse limit, capping at MAX_QUERY_LIMIT
fn parse_limit(params: &Value) -> Option<usize> {
    params
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|v| (v as usize).min(MAX_QUERY_LIMIT))
}

/// Validate and parse skip
fn parse_skip(params: &Value) -> Option<usize> {
    params
        .get("skip")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
}

/// Parse sort parameter - returns None if missing, null, or empty
/// Accepts two formats:
/// - Array: [["field", 1], ["field2", -1]]
/// - Object: {"field": 1, "field2": -1}
fn parse_sort(params: &Value) -> Option<Vec<(String, i32)>> {
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
fn parse_threshold(params: &Value) -> Result<Option<f64>> {
    match params.get("threshold").and_then(|v| v.as_f64()) {
        Some(t) if !(0.0..=1.0).contains(&t) => Err(McpError::InvalidParams(format!(
            "threshold must be between 0.0 and 1.0, got: {}",
            t
        ))),
        t => Ok(t),
    }
}

/// Parse projection: {"field": 1} or {"field": 0}
fn parse_projection(params: &Value) -> Result<Option<std::collections::HashMap<String, i32>>> {
    if let Some(proj_value) = params.get("projection") {
        if proj_value.is_null() {
            Ok(None)
        } else if let Some(obj) = proj_value.as_object() {
            let mut map = std::collections::HashMap::new();
            for (k, v) in obj {
                let int_val = if let Some(i) = v.as_i64() {
                    if i != 0 && i != 1 {
                        return Err(McpError::InvalidParams(format!(
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
                        return Err(McpError::InvalidParams(format!(
                            "Invalid projection value for '{}': expected 0 or 1, got {}",
                            k, f
                        )));
                    }
                } else {
                    return Err(McpError::InvalidParams(format!(
                        "Invalid projection value for '{}': expected 0 or 1, got {:?}",
                        k, v
                    )));
                };
                map.insert(k.clone(), int_val);
            }
            Ok(Some(map))
        } else {
            Err(McpError::InvalidParams(
                "projection must be an object like {\"field\": 1} or {\"field\": 0}".into(),
            ))
        }
    } else {
        Ok(None)
    }
}

/// Validate collection name
fn validate_collection_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(McpError::InvalidParams(
            "Collection name cannot be empty".into(),
        ));
    }
    if name.len() > MAX_COLLECTION_NAME_LEN {
        return Err(McpError::InvalidParams(format!(
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
        return Err(McpError::InvalidParams(
            "Collection name cannot contain dots. System collections can only be created via admin tools.".into()
        ));
    }

    // Check for invalid characters (allow alphanumeric, underscore, hyphen)
    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
    {
        return Err(McpError::InvalidParams(
            "Collection name can only contain alphanumeric characters, underscores, and hyphens".into()
        ));
    }
    Ok(())
}

/// BUG #12 fix: Validate script name (same rules as collection name)
fn validate_script_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(McpError::InvalidParams(
            "Script name cannot be empty".into(),
        ));
    }
    if name.len() > MAX_COLLECTION_NAME_LEN {
        return Err(McpError::InvalidParams(format!(
            "Script name too long (max {} chars)",
            MAX_COLLECTION_NAME_LEN
        )));
    }
    // Check for invalid characters (allow alphanumeric, underscore, dot, hyphen)
    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '.' || c == '-')
    {
        return Err(McpError::InvalidParams(
            "Script name can only contain alphanumeric characters, underscores, dots, and hyphens"
                .into(),
        ));
    }
    Ok(())
}

/// Get the list of all available tools for MCP tools/list
pub fn get_tools_list() -> Value {
    get_tools_list_filtered(true) // Default: show all tools (localhost assumed)
}

/// Get filtered tools list based on caller context
/// SECURITY FIX #14: Non-localhost callers don't see admin_* tools
/// This prevents information disclosure about admin capabilities
pub fn get_tools_list_filtered(is_localhost: bool) -> Value {
    let all_tools = get_all_tools_json();

    if is_localhost {
        // Localhost sees everything
        all_tools
    } else {
        // Filter out admin_* tools for non-localhost callers
        if let Some(tools_array) = all_tools.get("tools").and_then(|t| t.as_array()) {
            let filtered: Vec<Value> = tools_array
                .iter()
                .filter(|tool| {
                    tool.get("name")
                        .and_then(|n| n.as_str())
                        .map(|name| !name.starts_with("admin_"))
                        .unwrap_or(true)
                })
                .cloned()
                .collect();
            json!({ "tools": filtered })
        } else {
            all_tools
        }
    }
}

/// Internal function that returns the full tools JSON
fn get_all_tools_json() -> Value {
    json!({
        "tools": [
            // Database Management
            {
                "name": "db_open",
                "title": "Open Database",
                "description": "Open or create a database file. Closes the current database and switches to the new one.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Path to the database file (.mlite)"
                        },
                        "create": {
                            "type": "boolean",
                            "description": "If true, creates new database (path must not exist). If false, opens existing (path must exist).",
                            "default": false
                        }
                    },
                    "required": ["path"]
                }
            },
            {
                "name": "db_stats",
                "title": "Database Statistics",
                "description": "Get database statistics including collection count and names",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            },
            {
                "name": "db_compact",
                "title": "Compact Database",
                "description": "Compact the database file, removing deleted documents and freeing space",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            },
            {
                "name": "db_checkpoint",
                "title": "Force Checkpoint",
                "description": "Force a checkpoint - flush all pending writes to disk",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            },
            // Collection Management
            {
                "name": "collection_list",
                "title": "List Collections",
                "description": "List all collections in the database",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            },
            {
                "name": "collection_create",
                "title": "Create Collection",
                "description": "Create a new collection (implicitly created on first insert if not exists)",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "Collection name"
                        }
                    },
                    "required": ["name"]
                }
            },
            {
                "name": "collection_drop",
                "title": "Drop Collection",
                "description": "Drop (delete) a collection and all its documents",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "Collection name to drop"
                        }
                    },
                    "required": ["name"]
                }
            },
            // Document CRUD
            {
                "name": "insert_one",
                "title": "Insert Document",
                "description": "Insert a single document into a collection",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": {
                            "type": "string",
                            "description": "Collection name"
                        },
                        "document": {
                            "type": "object",
                            "description": "Document to insert (JSON object)"
                        }
                    },
                    "required": ["collection", "document"]
                }
            },
            {
                "name": "insert_many",
                "title": "Insert Multiple Documents",
                "description": "Insert multiple documents into a collection",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": {
                            "type": "string",
                            "description": "Collection name"
                        },
                        "documents": {
                            "type": "array",
                            "items": { "type": "object" },
                            "description": "Array of documents to insert"
                        }
                    },
                    "required": ["collection", "documents"]
                }
            },
            {
                "name": "find",
                "title": "Find Documents",
                "description": "Find documents matching a query. ⚠️ ALWAYS use count_documents FIRST to check size, then use 'limit' (5-20) and 'projection' to avoid context overflow!",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": {
                            "type": "string",
                            "description": "Collection name"
                        },
                        "query": {
                            "type": "object",
                            "description": "MongoDB-style query filter. Examples: {\"name\": \"Alice\"}, {\"age\": {\"$gte\": 18}}, {\"$or\": [{\"city\": \"NYC\"}, {\"city\": \"LA\"}]}"
                        },
                        "projection": {
                            "type": "object",
                            "description": "Fields to include (1) or exclude (0). Example: {\"name\": 1, \"age\": 1, \"_id\": 0}"
                        },
                        "sort": {
                            "type": "array",
                            "description": "Sort order as array of [field, direction] pairs. Example: [[\"age\", -1], [\"name\", 1]]"
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Maximum number of documents to return"
                        },
                        "skip": {
                            "type": "integer",
                            "description": "Number of documents to skip (for pagination)"
                        },
                        "include_total": {
                            "type": "boolean",
                            "description": "If true, also return total count of matching documents (before limit/skip). Useful for pagination UI.",
                            "default": false
                        }
                    },
                    "required": ["collection", "query"]
                }
            },
            {
                "name": "find_one",
                "title": "Find One Document",
                "description": "Find a single document matching the query with optional projection",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": {
                            "type": "string",
                            "description": "Collection name"
                        },
                        "query": {
                            "type": "object",
                            "description": "MongoDB-style query filter"
                        },
                        "projection": {
                            "type": "object",
                            "description": "Fields to include (1) or exclude (0). Example: {\"name\": 1, \"age\": 1, \"_id\": 0}"
                        }
                    },
                    "required": ["collection", "query"]
                }
            },
            {
                "name": "fuzzy_search",
                "title": "Fuzzy Text Search",
                "description": "Find documents using fuzzy text index (REQUIRES index_create_fuzzy first!). Returns documents with similarity scores, sorted by relevance. Useful for typo-tolerant search, name matching, and approximate string matching.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": {
                            "type": "string",
                            "description": "Collection name"
                        },
                        "field": {
                            "type": "string",
                            "description": "Field name to search in (must have fuzzy index)"
                        },
                        "query": {
                            "type": "string",
                            "description": "Search term to match approximately"
                        },
                        "algorithm": {
                            "type": "string",
                            "description": "Override fuzzy algorithm: 'jaro_winkler' (default, fast), 'levenshtein' (accurate), 'damerau_levenshtein' (handles transpositions)",
                            "enum": ["jaro_winkler", "levenshtein", "damerau_levenshtein"]
                        },
                        "threshold": {
                            "type": "number",
                            "description": "Override similarity threshold 0.0-1.0. Default uses index threshold."
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Maximum number of documents to return"
                        },
                        "projection": {
                            "type": "object",
                            "description": "Fields to include (1) or exclude (0). Example: {\"name\": 1, \"age\": 1, \"_id\": 0}"
                        }
                    },
                    "required": ["collection", "field", "query"]
                }
            },
            {
                "name": "update_one",
                "title": "Update Document",
                "description": "Update a single document matching the filter",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": {
                            "type": "string",
                            "description": "Collection name"
                        },
                        "filter": {
                            "type": "object",
                            "description": "Query filter to match documents"
                        },
                        "update": {
                            "type": "object",
                            "description": "Update operations. Use $set, $inc, $unset, $push, $pull, $addToSet, $pop. Example: {\"$set\": {\"status\": \"active\"}, \"$inc\": {\"count\": 1}}"
                        }
                    },
                    "required": ["collection", "filter", "update"]
                }
            },
            {
                "name": "update_many",
                "title": "Update Multiple Documents",
                "description": "Update all documents matching the filter",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": {
                            "type": "string",
                            "description": "Collection name"
                        },
                        "filter": {
                            "type": "object",
                            "description": "Query filter to match documents"
                        },
                        "update": {
                            "type": "object",
                            "description": "Update operations"
                        }
                    },
                    "required": ["collection", "filter", "update"]
                }
            },
            {
                "name": "delete_one",
                "title": "Delete Document",
                "description": "Delete a single document matching the filter",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": {
                            "type": "string",
                            "description": "Collection name"
                        },
                        "filter": {
                            "type": "object",
                            "description": "Query filter to match document to delete"
                        }
                    },
                    "required": ["collection", "filter"]
                }
            },
            {
                "name": "delete_many",
                "title": "Delete Multiple Documents",
                "description": "Delete all documents matching the filter",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": {
                            "type": "string",
                            "description": "Collection name"
                        },
                        "filter": {
                            "type": "object",
                            "description": "Query filter to match documents to delete"
                        }
                    },
                    "required": ["collection", "filter"]
                }
            },
            // Query Features
            {
                "name": "count_documents",
                "title": "Count Documents",
                "description": "Count documents matching a query",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": {
                            "type": "string",
                            "description": "Collection name"
                        },
                        "query": {
                            "type": "object",
                            "description": "Query filter (empty {} counts all documents)"
                        }
                    },
                    "required": ["collection"]
                }
            },
            {
                "name": "distinct",
                "title": "Get Distinct Values",
                "description": "Get distinct values for a field",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": {
                            "type": "string",
                            "description": "Collection name"
                        },
                        "field": {
                            "type": "string",
                            "description": "Field name to get distinct values for"
                        },
                        "query": {
                            "type": "object",
                            "description": "Optional filter to apply before getting distinct values"
                        }
                    },
                    "required": ["collection", "field"]
                }
            },
            {
                "name": "aggregate",
                "title": "Aggregation Pipeline",
                "description": "Execute an aggregation pipeline. ⚠️ ALWAYS include {\"$limit\": 10-20} stage to avoid context overflow!",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": {
                            "type": "string",
                            "description": "Collection name"
                        },
                        "pipeline": {
                            "type": "array",
                            "description": "Aggregation pipeline stages. Supported: $match, $group, $project, $sort, $limit, $skip. Example: [{\"$match\": {\"status\": \"active\"}}, {\"$group\": {\"_id\": \"$city\", \"count\": {\"$sum\": 1}}}]"
                        }
                    },
                    "required": ["collection", "pipeline"]
                }
            },
            // Index Management
            {
                "name": "index_create",
                "title": "Create Index",
                "description": "Create an index on a collection field(s)",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": {
                            "type": "string",
                            "description": "Collection name"
                        },
                        "field": {
                            "type": "string",
                            "description": "Field name to index (for single-field index)"
                        },
                        "fields": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Field names for compound index (use instead of 'field')"
                        },
                        "unique": {
                            "type": "boolean",
                            "description": "Whether the index should enforce uniqueness",
                            "default": false
                        }
                    },
                    "required": ["collection"]
                }
            },
            {
                "name": "index_list",
                "title": "List Indexes",
                "description": "List all indexes on a collection",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": {
                            "type": "string",
                            "description": "Collection name"
                        }
                    },
                    "required": ["collection"]
                }
            },
            {
                "name": "index_create_fuzzy",
                "title": "Create Fuzzy Index",
                "description": "Create a fuzzy text index for similarity-based search",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": {
                            "type": "string",
                            "description": "Collection name"
                        },
                        "field": {
                            "type": "string",
                            "description": "Field name to index for fuzzy search"
                        },
                        "algorithm": {
                            "type": "string",
                            "description": "Similarity algorithm: jaro_winkler (default, fast), levenshtein (accurate), damerau_levenshtein (good for typos)",
                            "enum": ["jaro_winkler", "levenshtein", "damerau_levenshtein"],
                            "default": "jaro_winkler"
                        },
                        "threshold": {
                            "type": "number",
                            "description": "Minimum similarity threshold 0.0-1.0 (default: 0.8)",
                            "default": 0.8
                        }
                    },
                    "required": ["collection", "field"]
                }
            },
            {
                "name": "index_create_fulltext",
                "title": "Create Full-Text Index",
                "description": "Create a full-text search index with language-aware stemming, stop words, and TF-IDF scoring. Supports Hungarian, English, German, and language-neutral modes.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": {
                            "type": "string",
                            "description": "Collection name"
                        },
                        "field": {
                            "type": "string",
                            "description": "Field name to index for full-text search"
                        },
                        "language": {
                            "type": "string",
                            "description": "Language for stemming and stop words: 'hungarian', 'english', 'german', 'none' (default: 'none')",
                            "enum": ["hungarian", "english", "german", "none"],
                            "default": "none"
                        },
                        "min_word_length": {
                            "type": "integer",
                            "description": "Minimum word length to index (default: 2)",
                            "default": 2
                        },
                        "accent_folding": {
                            "type": "boolean",
                            "description": "Whether to apply accent folding (á→a, ő→o, etc.) (default: true)",
                            "default": true
                        }
                    },
                    "required": ["collection", "field"]
                }
            },
            {
                "name": "fulltext_search",
                "title": "Full-Text Search",
                "description": "Search documents using full-text index with TF-IDF relevance scoring. Returns documents sorted by relevance score.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": {
                            "type": "string",
                            "description": "Collection name"
                        },
                        "field": {
                            "type": "string",
                            "description": "Field name with full-text index"
                        },
                        "query": {
                            "type": "string",
                            "description": "Search query text (will be tokenized and searched)"
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Maximum number of results (default: 10)",
                            "default": 10
                        },
                        "skip": {
                            "type": "integer",
                            "description": "Number of results to skip for pagination (default: 0)",
                            "default": 0
                        },
                        "min_score": {
                            "type": "number",
                            "description": "Minimum TF-IDF score threshold (default: no threshold)"
                        },
                        "projection": {
                            "type": "object",
                            "description": "Fields to include (1) or exclude (0). Example: {\"full_text\": 0} to exclude, or {\"title\": 1, \"_id\": 1} to include only specific fields"
                        }
                    },
                    "required": ["collection", "field", "query"]
                }
            },
            {
                "name": "index_list_fulltext",
                "title": "List Full-Text Indexes",
                "description": "List all full-text search indexes for a collection with their metadata (field, language, document count, token count)",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": {
                            "type": "string",
                            "description": "Collection name"
                        }
                    },
                    "required": ["collection"]
                }
            },
            {
                "name": "index_drop",
                "title": "Drop Index",
                "description": "Drop (delete) an index from a collection",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": {
                            "type": "string",
                            "description": "Collection name"
                        },
                        "index_name": {
                            "type": "string",
                            "description": "Name of the index to drop (use index_list to see available indexes)"
                        }
                    },
                    "required": ["collection", "index_name"]
                }
            },
            // Query Analysis
            {
                "name": "explain",
                "title": "Explain Query",
                "description": "Explain query execution plan. Shows whether an index is used and the query strategy.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": {
                            "type": "string",
                            "description": "Collection name"
                        },
                        "query": {
                            "type": "object",
                            "description": "Query to analyze"
                        }
                    },
                    "required": ["collection", "query"]
                }
            },
            {
                "name": "find_with_hint",
                "title": "Find with Index Hint",
                "description": "Find documents using a specific index (forces index usage), with full FindOptions support",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": {
                            "type": "string",
                            "description": "Collection name"
                        },
                        "query": {
                            "type": "object",
                            "description": "Query filter"
                        },
                        "hint": {
                            "type": "string",
                            "description": "Index name to use (from index_list)"
                        },
                        "projection": {
                            "type": "object",
                            "description": "Fields to include (1) or exclude (0). Example: {\"name\": 1, \"age\": 1, \"_id\": 0}"
                        },
                        "sort": {
                            "type": "array",
                            "description": "Sort order as array of [field, direction] pairs. Example: [[\"age\", -1], [\"name\", 1]]"
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Maximum number of documents to return"
                        },
                        "skip": {
                            "type": "integer",
                            "description": "Number of documents to skip (for pagination)"
                        }
                    },
                    "required": ["collection", "query", "hint"]
                }
            },
            // Schema Management
            {
                "name": "schema_set",
                "title": "Set JSON Schema",
                "description": "Set or clear a JSON schema for a collection. Schema is used to validate documents on insert/update.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": {
                            "type": "string",
                            "description": "Collection name"
                        },
                        "schema": {
                            "type": "object",
                            "description": "JSON Schema object. Must have type: 'object'. Supports 'required' array and 'properties' with types: string, number, integer, boolean, object, array. Pass null to clear schema."
                        }
                    },
                    "required": ["collection"]
                }
            },
            {
                "name": "schema_get",
                "title": "Get JSON Schema",
                "description": "Get the JSON schema for a collection",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": {
                            "type": "string",
                            "description": "Collection name"
                        }
                    },
                    "required": ["collection"]
                }
            },
            // Script Management
            {
                "name": "script_save",
                "title": "Save Script",
                "description": "Save a script to the database. Scripts are stored in _system.scripts collection. Supports versioning - each save creates a new version.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "Script name (used as identifier)"
                        },
                        "code": {
                            "type": "string",
                            "description": "Rhai script code"
                        },
                        "description": {
                            "type": "string",
                            "description": "Optional description of what the script does"
                        },
                        "tags": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Optional tags for categorization (e.g. ['utility', 'report'])"
                        },
                        "dependencies": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Optional list of script names this script depends on"
                        }
                    },
                    "required": ["name", "code"]
                }
            },
            {
                "name": "script_list",
                "title": "List Scripts",
                "description": "List all saved scripts (without code). Supports filtering by tags.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "tags": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Optional tags to filter by"
                        },
                        "match_all": {
                            "type": "boolean",
                            "description": "If true, match ALL tags (AND). If false or omitted, match ANY tag (OR)."
                        }
                    },
                    "required": []
                }
            },
            {
                "name": "script_get",
                "title": "Get Script",
                "description": "Get a script by name (with code)",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "Script name"
                        }
                    },
                    "required": ["name"]
                }
            },
            {
                "name": "script_delete",
                "title": "Delete Script",
                "description": "Delete a script by name",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "Script name to delete"
                        }
                    },
                    "required": ["name"]
                }
            },
            {
                "name": "script_run",
                "title": "Run Script",
                "description": "Run a saved script by name with optional parameters. Available functions: DB: db_find, db_find_one, db_find_one_result (returns {found, doc, error}), db_insert_one, db_insert_many, db_update_one, db_update_many, db_delete_one, db_delete_many, db_count, db_aggregate. Helpers: is_error(v), is_null(v), get_error(v), unwrap_or(v, default). Utils: base64_encode, base64_decode, print. Returns script result and captured logs.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "Script name to run"
                        },
                        "params": {
                            "type": "object",
                            "description": "Optional parameters passed to the script (accessible as 'params' variable)"
                        },
                        "max_operations": {
                            "type": "integer",
                            "description": "Maximum number of operations allowed (default: 1000000, for DoS protection)"
                        }
                    },
                    "required": ["name"]
                }
            },
            {
                "name": "script_exec",
                "title": "Execute Script",
                "description": "Execute inline Rhai code without saving. Useful for one-off operations. Available functions: DB: db_find, db_find_one, db_find_one_result (returns {found, doc, error}), db_insert_one, db_insert_many, db_update_one, db_update_many, db_delete_one, db_delete_many, db_count, db_aggregate. Helpers: is_error(v), is_null(v), get_error(v), unwrap_or(v, default). Utils: base64_encode, base64_decode, print.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "code": {
                            "type": "string",
                            "description": "Rhai script code to execute"
                        },
                        "params": {
                            "type": "object",
                            "description": "Optional parameters passed to the script (accessible as 'params' variable)"
                        },
                        "max_operations": {
                            "type": "integer",
                            "description": "Maximum number of operations allowed (default: 1000000, for DoS protection)"
                        }
                    },
                    "required": ["code"]
                }
            },
            {
                "name": "script_history",
                "title": "Script History",
                "description": "Get version history of a script",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "Script name"
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Maximum number of versions to return (optional)"
                        }
                    },
                    "required": ["name"]
                }
            },
            {
                "name": "script_rollback",
                "title": "Rollback Script",
                "description": "Rollback a script to a previous version (creates a new version with the old code)",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "Script name"
                        },
                        "version": {
                            "type": "integer",
                            "description": "Version number to rollback to"
                        }
                    },
                    "required": ["name", "version"]
                }
            },
            {
                "name": "script_version_get",
                "title": "Get Script Version",
                "description": "Get a specific version of a script",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "Script name"
                        },
                        "version": {
                            "type": "integer",
                            "description": "Version number to retrieve"
                        }
                    },
                    "required": ["name", "version"]
                }
            },
            {
                "name": "script_tags_add",
                "title": "Add Script Tags",
                "description": "Add tags to a script",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "Script name"
                        },
                        "tags": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Tags to add"
                        }
                    },
                    "required": ["name", "tags"]
                }
            },
            {
                "name": "script_tags_remove",
                "title": "Remove Script Tags",
                "description": "Remove tags from a script",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "Script name"
                        },
                        "tags": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Tags to remove"
                        }
                    },
                    "required": ["name", "tags"]
                }
            },
            {
                "name": "script_stats",
                "title": "Script Statistics",
                "description": "Get execution statistics for a script",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "Script name"
                        }
                    },
                    "required": ["name"]
                }
            },
            // Transaction Management (Read Committed Isolation)
            {
                "name": "begin_transaction",
                "title": "Begin Transaction",
                "description": "Start a new transaction with Read Committed isolation. Only one write transaction can be active at a time. Returns transaction_id to use with other _tx operations.",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            },
            {
                "name": "commit_transaction",
                "title": "Commit Transaction",
                "description": "Commit an active transaction, making all changes permanent. Releases the write lock.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "transaction_id": {
                            "type": "string",
                            "description": "Transaction ID returned by begin_transaction"
                        }
                    },
                    "required": ["transaction_id"]
                }
            },
            {
                "name": "rollback_transaction",
                "title": "Rollback Transaction",
                "description": "Rollback an active transaction, discarding all buffered changes. Releases the write lock.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "transaction_id": {
                            "type": "string",
                            "description": "Transaction ID returned by begin_transaction"
                        }
                    },
                    "required": ["transaction_id"]
                }
            },
            {
                "name": "insert_one_tx",
                "title": "Insert Document (Transaction)",
                "description": "Insert a document within a transaction. Changes are buffered until commit.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "transaction_id": {
                            "type": "string",
                            "description": "Transaction ID returned by begin_transaction"
                        },
                        "collection": {
                            "type": "string",
                            "description": "Collection name"
                        },
                        "document": {
                            "type": "object",
                            "description": "Document to insert"
                        }
                    },
                    "required": ["transaction_id", "collection", "document"]
                }
            },
            {
                "name": "update_one_tx",
                "title": "Update Document (Transaction)",
                "description": "Update a document within a transaction. Changes are buffered until commit.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "transaction_id": {
                            "type": "string",
                            "description": "Transaction ID returned by begin_transaction"
                        },
                        "collection": {
                            "type": "string",
                            "description": "Collection name"
                        },
                        "filter": {
                            "type": "object",
                            "description": "Query filter to match the document"
                        },
                        "update": {
                            "type": "object",
                            "description": "Update operators (e.g. {\"$set\": {\"field\": \"value\"}})"
                        }
                    },
                    "required": ["transaction_id", "collection", "filter", "update"]
                }
            },
            {
                "name": "delete_one_tx",
                "title": "Delete Document (Transaction)",
                "description": "Delete a document within a transaction. Changes are buffered until commit.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "transaction_id": {
                            "type": "string",
                            "description": "Transaction ID returned by begin_transaction"
                        },
                        "collection": {
                            "type": "string",
                            "description": "Collection name"
                        },
                        "filter": {
                            "type": "object",
                            "description": "Query filter to match the document"
                        }
                    },
                    "required": ["transaction_id", "collection", "filter"]
                }
            },
            {
                "name": "transaction_status",
                "title": "Transaction Status",
                "description": "Check if there's an active write transaction and get its ID",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            },
            // Admin Operations (require IRONBASE_ADMIN_KEY)
            {
                "name": "admin_list_all_collections",
                "title": "Admin: List All Collections",
                "description": "List ALL collections including hidden/system collections. Only accessible from localhost. Requires IRONBASE_ADMIN_KEY.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "admin_key": {
                            "type": "string",
                            "description": "Admin key (must match IRONBASE_ADMIN_KEY env var)"
                        }
                    },
                    "required": ["admin_key"]
                }
            },
            {
                "name": "admin_create_system_collection",
                "title": "Admin: Create System Collection",
                "description": "Create a system collection with protected/hidden flags. Only accessible from localhost. Requires IRONBASE_ADMIN_KEY.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "admin_key": {
                            "type": "string",
                            "description": "Admin key (must match IRONBASE_ADMIN_KEY env var)"
                        },
                        "name": {
                            "type": "string",
                            "description": "Collection name (convention: _system.xxx)"
                        }
                    },
                    "required": ["admin_key", "name"]
                }
            },
            {
                "name": "admin_set_collection_flags",
                "title": "Admin: Set Collection Flags",
                "description": "Set collection flags (is_system, protected, hidden). Only accessible from localhost. Requires IRONBASE_ADMIN_KEY.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "admin_key": {
                            "type": "string",
                            "description": "Admin key (must match IRONBASE_ADMIN_KEY env var)"
                        },
                        "collection": {
                            "type": "string",
                            "description": "Collection name"
                        },
                        "is_system": {
                            "type": "boolean",
                            "description": "Mark as system collection"
                        },
                        "protected": {
                            "type": "boolean",
                            "description": "Prevent deletion via drop_collection"
                        },
                        "hidden": {
                            "type": "boolean",
                            "description": "Hide from list_collections (still visible via admin_list_all_collections)"
                        }
                    },
                    "required": ["admin_key", "collection"]
                }
            },
            {
                "name": "admin_drop_protected",
                "title": "Admin: Drop Protected Collection",
                "description": "Force drop a protected collection. Only accessible from localhost. Requires IRONBASE_ADMIN_KEY. USE WITH CAUTION!",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "admin_key": {
                            "type": "string",
                            "description": "Admin key (must match IRONBASE_ADMIN_KEY env var)"
                        },
                        "name": {
                            "type": "string",
                            "description": "Collection name to drop"
                        }
                    },
                    "required": ["admin_key", "name"]
                }
            },
            // API Key Management (require IRONBASE_ADMIN_KEY)
            {
                "name": "admin_apikey_create",
                "title": "Admin: Create API Key",
                "description": "Create a new API key for accessing the MCP server. Only accessible from localhost. Requires IRONBASE_ADMIN_KEY.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "admin_key": {
                            "type": "string",
                            "description": "Admin key (must match IRONBASE_ADMIN_KEY env var)"
                        },
                        "name": {
                            "type": "string",
                            "description": "Name/description for this API key"
                        }
                    },
                    "required": ["admin_key", "name"]
                }
            },
            {
                "name": "admin_apikey_list",
                "title": "Admin: List API Keys",
                "description": "List all API keys (key values are masked). Only accessible from localhost. Requires IRONBASE_ADMIN_KEY.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "admin_key": {
                            "type": "string",
                            "description": "Admin key (must match IRONBASE_ADMIN_KEY env var)"
                        }
                    },
                    "required": ["admin_key"]
                }
            },
            {
                "name": "admin_apikey_revoke",
                "title": "Admin: Revoke API Key",
                "description": "Revoke (disable) an API key by ID. Only accessible from localhost. Requires IRONBASE_ADMIN_KEY.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "admin_key": {
                            "type": "string",
                            "description": "Admin key (must match IRONBASE_ADMIN_KEY env var)"
                        },
                        "id": {
                            "type": "integer",
                            "description": "API key ID to revoke"
                        }
                    },
                    "required": ["admin_key", "id"]
                }
            },
            {
                "name": "admin_apikey_delete",
                "title": "Admin: Delete API Key",
                "description": "Permanently delete an API key by ID. Only accessible from localhost. Requires IRONBASE_ADMIN_KEY.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "admin_key": {
                            "type": "string",
                            "description": "Admin key (must match IRONBASE_ADMIN_KEY env var)"
                        },
                        "id": {
                            "type": "integer",
                            "description": "API key ID to delete"
                        }
                    },
                    "required": ["admin_key", "id"]
                }
            },

            // ACL (Access Control List) Management
            {
                "name": "acl_list",
                "title": "List ACL Rules",
                "description": "List all ACL rules for collection-level access control. Shows both built-in and custom rules. Requires read permission on _system.acl.",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            },
            {
                "name": "acl_get",
                "title": "Get Collection ACL",
                "description": "Get ACL rules for a specific collection. Requires read permission on _system.acl.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": {
                            "type": "string",
                            "description": "Collection name to get ACL for"
                        }
                    },
                    "required": ["collection"]
                }
            },
            {
                "name": "acl_set",
                "title": "Set Collection ACL",
                "description": "Set ACL rules for a collection. Only accessible from localhost. Rules format: [{\"principal\": \"interface:internal\", \"permissions\": \"read,write\"}]",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": {
                            "type": "string",
                            "description": "Collection name to set ACL for (use '*' for default rules)"
                        },
                        "rules": {
                            "type": "array",
                            "description": "Array of ACL rules. Each rule has 'principal' (e.g., 'interface:internal', 'apikey:mykey') and 'permissions' (e.g., 'read', 'read,write', 'all', 'deny')",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "principal": {
                                        "type": "string",
                                        "description": "Who: interface:localhost|internal|external, apikey:keyname, ip:1.2.3.4, iprange:192.168.0.0/24, anyone"
                                    },
                                    "permissions": {
                                        "type": "string",
                                        "description": "What: read, write, admin, all, deny (comma-separated)"
                                    }
                                },
                                "required": ["principal", "permissions"]
                            }
                        }
                    },
                    "required": ["collection", "rules"]
                }
            },
            {
                "name": "acl_delete",
                "title": "Delete Collection ACL",
                "description": "Delete custom ACL rules for a collection, reverting to defaults. Only accessible from localhost.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": {
                            "type": "string",
                            "description": "Collection name to delete ACL for"
                        }
                    },
                    "required": ["collection"]
                }
            },
            {
                "name": "acl_cleanup",
                "title": "Cleanup Orphan ACLs",
                "description": "Remove ACL rules for collections that no longer exist. Only accessible from localhost.",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            },

            // Listener Management
            {
                "name": "listener_list",
                "title": "List Listeners",
                "description": "List all configured listeners (HTTP/HTTPS endpoints). Requires read permission on _system.listeners.",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            },
            {
                "name": "listener_get",
                "title": "Get Listener",
                "description": "Get configuration for a specific listener by ID. Requires read permission on _system.listeners.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "id": {
                            "type": "string",
                            "description": "Listener ID"
                        }
                    },
                    "required": ["id"]
                }
            },
            {
                "name": "listener_add",
                "title": "Add Listener",
                "description": "Add a new HTTP/HTTPS listener. Only accessible from localhost. Requires server restart to take effect.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "id": {
                            "type": "string",
                            "description": "Unique identifier for the listener"
                        },
                        "bind": {
                            "type": "string",
                            "description": "Bind address (e.g., '0.0.0.0:8080', '192.168.1.100:443')"
                        },
                        "tls": {
                            "type": "boolean",
                            "description": "Enable TLS/HTTPS (default: false)"
                        },
                        "cert_path": {
                            "type": "string",
                            "description": "Path to TLS certificate file (required if tls=true)"
                        },
                        "key_path": {
                            "type": "string",
                            "description": "Path to TLS private key file (required if tls=true)"
                        },
                        "description": {
                            "type": "string",
                            "description": "Optional description for this listener"
                        }
                    },
                    "required": ["id", "bind"]
                }
            },
            {
                "name": "listener_delete",
                "title": "Delete Listener",
                "description": "Delete a listener configuration. Only accessible from localhost. Requires server restart to take effect.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "id": {
                            "type": "string",
                            "description": "Listener ID to delete"
                        }
                    },
                    "required": ["id"]
                }
            },
            {
                "name": "listener_enable",
                "title": "Enable Listener",
                "description": "Enable a disabled listener. Only accessible from localhost. Requires server restart to take effect.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "id": {
                            "type": "string",
                            "description": "Listener ID to enable"
                        }
                    },
                    "required": ["id"]
                }
            },
            {
                "name": "listener_disable",
                "title": "Disable Listener",
                "description": "Disable a listener without deleting it. Only accessible from localhost. Requires server restart to take effect.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "id": {
                            "type": "string",
                            "description": "Listener ID to disable"
                        }
                    },
                    "required": ["id"]
                }
            }
        ]
    })
}

/// Dispatch a tool call to the appropriate handler
///
/// The `api_key_cache` parameter is optional and used to invalidate the cache
/// when API keys are created, revoked, or deleted.
///
/// The `server_info` parameter is optional and used to include server runtime
/// information in db_stats output.
pub fn dispatch_tool(
    name: &str,
    params: Value,
    adapter: &Arc<IronBaseAdapter>,
    api_key_cache: Option<&crate::ApiKeyCache>,
    server_info: Option<&crate::ServerInfo>,
) -> Result<Value> {
    match name {
        // Database Management
        "db_open" => {
            let path = get_string(&params, "path")?;
            let create = params
                .get("create")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            let new_path = adapter.switch_database(&path, create)?;

            Ok(json!({
                "success": true,
                "path": new_path,
                "message": if create { "Database created" } else { "Database opened" }
            }))
        }
        "db_stats" => {
            let mut stats = adapter.stats();
            // Add server info if available
            if let Some(info) = server_info {
                if let Some(obj) = stats.as_object_mut() {
                    obj.insert(
                        "server".to_string(),
                        json!({
                            "version": crate::VERSION,
                            "protocol": info.protocol,
                            "host": info.host,
                            "port": info.port,
                            "require_api_key": info.require_api_key,
                        }),
                    );
                }
            } else {
                // Stdio mode - no server info
                if let Some(obj) = stats.as_object_mut() {
                    obj.insert(
                        "server".to_string(),
                        json!({
                            "version": crate::VERSION,
                            "mode": "stdio",
                        }),
                    );
                }
            }
            Ok(stats)
        }
        "db_compact" => adapter.compact(),
        "db_checkpoint" => {
            adapter.checkpoint()?;
            Ok(json!({"success": true, "message": "Checkpoint completed"}))
        }

        // Collection Management
        "collection_list" => {
            let collections = adapter.list_collections();
            Ok(json!({"collections": collections}))
        }
        "collection_create" => {
            let name = get_string(&params, "name")?;
            adapter.create_collection(&name)?;
            Ok(json!({"success": true, "collection": name}))
        }
        "collection_drop" => {
            use crate::acl::SYSTEM_ACL_COLLECTION;

            let name = get_string(&params, "name")?;
            adapter.drop_collection(&name)?;

            // Also delete ACL for this collection
            let acl_deleted = adapter
                .delete_one(SYSTEM_ACL_COLLECTION, json!({"collection": name}))
                .unwrap_or(0)
                > 0;

            Ok(json!({
                "success": true,
                "dropped": name,
                "acl_deleted": acl_deleted
            }))
        }

        // Document CRUD
        "insert_one" => {
            let collection = get_string(&params, "collection")?;
            let document = get_object(&params, "document")?;
            let id = adapter.insert_one(&collection, document)?;
            Ok(json!({"inserted_id": id}))
        }
        "insert_many" => {
            let collection = get_string(&params, "collection")?;
            let documents = get_array(&params, "documents")?;
            let ids = adapter.insert_many(&collection, documents)?;
            Ok(json!({"inserted_ids": ids, "inserted_count": ids.len()}))
        }
        "find" => {
            let collection = get_string(&params, "collection")?;
            validate_collection_name(&collection)?;
            let query = params.get("query").cloned().unwrap_or(json!({}));
            let include_total = params
                .get("include_total")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let options = FindOptions {
                projection: params.get("projection").cloned(),
                sort: parse_sort(&params),
                limit: parse_limit(&params),
                skip: parse_skip(&params),
                include_total,
            };
            let result = adapter.find(&collection, query, options)?;
            let mut response = json!({
                "documents": result.documents,
                "count": result.documents.len()
            });
            if let Some(total) = result.total {
                response["total"] = json!(total);
            }
            Ok(response)
        }
        "find_one" => {
            let collection = get_string(&params, "collection")?;
            let query = params.get("query").cloned().unwrap_or(json!({}));
            let projection = parse_projection(&params)?;
            let document = adapter.find_one(&collection, query)?;

            // Apply projection if specified
            let result = match (document, projection) {
                (Some(doc), Some(proj)) => {
                    Some(apply_projection(&doc, &proj).map_err(|e| McpError::InvalidParams(e.to_string()))?)
                }
                (doc, _) => doc,
            };
            Ok(json!({"document": result}))
        }
        "fuzzy_search" => {
            let collection = get_string(&params, "collection")?;
            validate_collection_name(&collection)?;
            let field = get_string(&params, "field")?;
            let query = get_string(&params, "query")?;
            let threshold = parse_threshold(&params)?;
            let algorithm = params.get("algorithm").and_then(|v| v.as_str());
            let limit = parse_limit(&params);
            let projection = parse_projection(&params)?;

            // Use the real fuzzy search with index
            let mut results =
                adapter.fuzzy_search(&collection, &field, &query, threshold, algorithm)?;

            // Apply limit if specified (capped at MAX_QUERY_LIMIT)
            if let Some(lim) = limit {
                results.truncate(lim);
            }

            // Format results with scores, applying projection if specified
            let documents: Vec<Value> = results
                .into_iter()
                .map(|(doc, score)| {
                    let projected_doc = if let Some(ref proj) = projection {
                        apply_projection(&doc, proj).unwrap_or(doc)
                    } else {
                        doc
                    };
                    json!({
                        "document": projected_doc,
                        "score": score
                    })
                })
                .collect();

            Ok(json!({"results": documents, "count": documents.len()}))
        }
        "update_one" => {
            let collection = get_string(&params, "collection")?;
            let filter = get_object(&params, "filter")?;
            let update = get_object(&params, "update")?;
            let result = adapter.update_one(&collection, filter, update)?;
            Ok(json!({
                "matched_count": result.matched_count,
                "modified_count": result.modified_count
            }))
        }
        "update_many" => {
            let collection = get_string(&params, "collection")?;
            let filter = get_object(&params, "filter")?;
            let update = get_object(&params, "update")?;
            let result = adapter.update_many(&collection, filter, update)?;
            Ok(json!({
                "matched_count": result.matched_count,
                "modified_count": result.modified_count
            }))
        }
        "delete_one" => {
            let collection = get_string(&params, "collection")?;
            let filter = get_object(&params, "filter")?;
            let count = adapter.delete_one(&collection, filter)?;
            Ok(json!({"deleted_count": count}))
        }
        "delete_many" => {
            let collection = get_string(&params, "collection")?;
            let filter = get_object(&params, "filter")?;
            let count = adapter.delete_many(&collection, filter)?;
            Ok(json!({"deleted_count": count}))
        }

        // Query Features
        "count_documents" => {
            let collection = get_string(&params, "collection")?;
            let query = params.get("query").cloned().unwrap_or(json!({}));
            let count = adapter.count_documents(&collection, query)?;
            Ok(json!({"count": count}))
        }
        "distinct" => {
            let collection = get_string(&params, "collection")?;
            let field = get_string(&params, "field")?;
            let query = params.get("query").cloned().unwrap_or(json!({}));
            let values = adapter.distinct(&collection, &field, query)?;
            Ok(json!({"values": values, "count": values.len()}))
        }
        "aggregate" => {
            let collection = get_string(&params, "collection")?;
            let pipeline = get_array(&params, "pipeline")?;
            let results = adapter.aggregate(&collection, pipeline)?;
            Ok(json!({"results": results, "count": results.len()}))
        }

        // Index Management
        "index_create" => {
            let collection = get_string(&params, "collection")?;
            let unique = params
                .get("unique")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            // Check for compound index
            if let Some(fields) = params.get("fields").and_then(|v| v.as_array()) {
                let field_names: Vec<String> = fields
                    .iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect();
                if field_names.is_empty() {
                    return Err(McpError::InvalidParams("fields array is empty".to_string()));
                }
                let name = adapter.create_compound_index(&collection, &field_names, unique)?;
                Ok(json!({"index_name": name, "fields": field_names, "unique": unique}))
            } else {
                // Single field index
                let field = get_string(&params, "field")?;
                let name = adapter.create_index(&collection, &field, unique)?;
                Ok(json!({"index_name": name, "field": field, "unique": unique}))
            }
        }
        "index_list" => {
            let collection = get_string(&params, "collection")?;
            let indexes = adapter.list_indexes(&collection)?;
            Ok(json!({"indexes": indexes}))
        }
        "index_create_fuzzy" => {
            let collection = get_string(&params, "collection")?;
            let field = get_string(&params, "field")?;
            let algorithm = params
                .get("algorithm")
                .and_then(|v| v.as_str())
                .unwrap_or("jaro_winkler");
            let threshold = params
                .get("threshold")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.8);
            let name = adapter.create_fuzzy_index(&collection, &field, algorithm, threshold)?;
            Ok(json!({
                "index_name": name,
                "field": field,
                "algorithm": algorithm,
                "threshold": threshold
            }))
        }
        "index_create_fulltext" => {
            let collection = get_string(&params, "collection")?;
            let field = get_string(&params, "field")?;
            let language = params
                .get("language")
                .and_then(|v| v.as_str())
                .unwrap_or("none");
            let min_word_length = params
                .get("min_word_length")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);
            let accent_folding = params.get("accent_folding").and_then(|v| v.as_bool());
            let name = adapter.create_fulltext_index(
                &collection,
                &field,
                language,
                min_word_length,
                accent_folding,
            )?;
            Ok(json!({
                "index_name": name,
                "field": field,
                "language": language,
                "min_word_length": min_word_length.unwrap_or(2),
                "accent_folding": accent_folding.unwrap_or(true)
            }))
        }
        "fulltext_search" => {
            let collection = get_string(&params, "collection")?;
            validate_collection_name(&collection)?;
            let field = get_string(&params, "field")?;
            let query = get_string(&params, "query")?;
            let limit = parse_limit(&params);
            let skip = parse_skip(&params);
            let min_score = params.get("min_score").and_then(|v| v.as_f64());

            // Parse projection: {"field": 1} or {"field": 0}
            // BUG #10 fix: Validate BEFORE casting to prevent silent truncation
            let projection: Option<std::collections::HashMap<String, i32>> =
                if let Some(proj_value) = params.get("projection") {
                    if proj_value.is_null() {
                        None
                    } else if let Some(obj) = proj_value.as_object() {
                        let mut map = std::collections::HashMap::new();
                        for (k, v) in obj {
                            // BUG #10 fix: Check exact values BEFORE casting
                            let int_val = if let Some(i) = v.as_i64() {
                                // Validate integer is exactly 0 or 1 before casting
                                if i != 0 && i != 1 {
                                    return Err(McpError::InvalidParams(format!(
                                    "Invalid projection value for '{}': expected 0 or 1, got {}",
                                    k, i
                                )));
                                }
                                i as i32
                            } else if let Some(f) = v.as_f64() {
                                // Validate float is exactly 0.0 or 1.0 (no truncation)
                                if f == 0.0 {
                                    0
                                } else if f == 1.0 {
                                    1
                                } else {
                                    return Err(McpError::InvalidParams(format!(
                                    "Invalid projection value for '{}': expected 0 or 1, got {}",
                                    k, f
                                )));
                                }
                            } else {
                                return Err(McpError::InvalidParams(format!(
                                    "Invalid projection value for '{}': expected 0 or 1, got {:?}",
                                    k, v
                                )));
                            };
                            map.insert(k.clone(), int_val);
                        }
                        Some(map)
                    } else {
                        return Err(McpError::InvalidParams(
                            "projection must be an object like {\"field\": 1} or {\"field\": 0}"
                                .into(),
                        ));
                    }
                } else {
                    None
                };

            let options = FulltextSearchOptions {
                limit,
                skip,
                min_score,
                projection,
            };
            let results = adapter.fulltext_search(&collection, &field, &query, options)?;

            // Format results with scores and matched tokens
            let documents: Vec<Value> = results
                .into_iter()
                .map(|(doc, score, matched_tokens)| {
                    json!({
                        "document": doc,
                        "score": score,
                        "matched_tokens": matched_tokens
                    })
                })
                .collect();

            Ok(json!({"results": documents, "count": documents.len()}))
        }
        "index_list_fulltext" => {
            let collection = get_string(&params, "collection")?;
            validate_collection_name(&collection)?;
            let indexes = adapter.list_fulltext_indexes(&collection)?;
            Ok(json!({"indexes": indexes, "count": indexes.len()}))
        }
        "index_drop" => {
            let collection = get_string(&params, "collection")?;
            let index_name = get_string(&params, "index_name")?;
            adapter.drop_index(&collection, &index_name)?;
            Ok(json!({"success": true, "dropped": index_name}))
        }

        // Query Analysis
        "explain" => {
            let collection = get_string(&params, "collection")?;
            let query = params.get("query").cloned().unwrap_or(json!({}));
            let plan = adapter.explain(&collection, query)?;
            Ok(json!({"plan": plan}))
        }
        "find_with_hint" => {
            let collection = get_string(&params, "collection")?;
            let query = params.get("query").cloned().unwrap_or(json!({}));
            let hint = get_string(&params, "hint")?;
            let projection = parse_projection(&params)?;
            let sort = parse_sort(&params);
            let limit = parse_limit(&params);
            let skip = parse_skip(&params);

            let mut documents = adapter.find_with_hint(&collection, query, &hint)?;

            // Apply sort if specified
            if let Some(ref sort_spec) = sort {
                apply_sort(&mut documents, sort_spec)
                    .map_err(|e| McpError::InvalidParams(e.to_string()))?;
            }

            // Apply skip
            if let Some(s) = skip {
                if s < documents.len() {
                    documents = documents.into_iter().skip(s).collect();
                } else {
                    documents = Vec::new();
                }
            }

            // Apply limit
            if let Some(l) = limit {
                documents.truncate(l.min(MAX_QUERY_LIMIT));
            }

            // Apply projection if specified
            let documents: Vec<Value> = if let Some(ref proj) = projection {
                documents
                    .into_iter()
                    .map(|doc| apply_projection(&doc, proj))
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(|e| McpError::InvalidParams(e.to_string()))?
            } else {
                documents
            };

            Ok(json!({"documents": documents, "count": documents.len()}))
        }

        // Schema Management
        "schema_set" => {
            let collection = get_string(&params, "collection")?;
            let schema = params.get("schema").cloned().filter(|v| !v.is_null());
            adapter.set_schema(&collection, schema.clone())?;
            Ok(json!({"success": true, "schema_set": schema.is_some()}))
        }
        "schema_get" => {
            let collection = get_string(&params, "collection")?;
            let schema = adapter.get_schema(&collection)?;
            Ok(json!({"schema": schema}))
        }

        // Script Management
        "script_save" => {
            let name = get_string(&params, "name")?;
            validate_script_name(&name)?; // BUG #12 fix: Validate script name
            let code = get_string(&params, "code")?;
            let description = params.get("description").and_then(|v| v.as_str());
            let tags: Option<Vec<String>> = params.get("tags").and_then(|v| {
                v.as_array().map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
            });
            let dependencies: Option<Vec<String>> = params.get("dependencies").and_then(|v| {
                v.as_array().map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
            });
            let manager = ScriptManager::new(Arc::clone(adapter));
            let version = manager.save(&name, &code, description, tags, dependencies)?;
            Ok(json!({"success": true, "name": name, "version": version}))
        }
        "script_list" => {
            use crate::scripting::ScriptListFilter;
            let manager = ScriptManager::new(Arc::clone(adapter));
            let filter = {
                let tags: Option<Vec<String>> = params.get("tags").and_then(|v| {
                    v.as_array().map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                });
                let match_all = params
                    .get("match_all")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if tags.is_some() {
                    Some(ScriptListFilter {
                        tags,
                        match_all_tags: match_all,
                    })
                } else {
                    None
                }
            };
            let scripts = manager.list(filter)?;
            Ok(json!({"scripts": scripts, "count": scripts.len()}))
        }
        "script_get" => {
            let name = get_string(&params, "name")?;
            let manager = ScriptManager::new(Arc::clone(adapter));
            match manager.get(&name)? {
                Some(script) => Ok(json!({
                    "name": script.name,
                    "code": script.code,
                    "description": script.description,
                    "created_at": script.created_at,
                    "updated_at": script.updated_at,
                    "version": script.version,
                    "tags": script.tags,
                    "dependencies": script.dependencies
                })),
                None => Err(McpError::InvalidParams(format!(
                    "Script '{}' not found",
                    name
                ))),
            }
        }
        "script_delete" => {
            let name = get_string(&params, "name")?;
            let manager = ScriptManager::new(Arc::clone(adapter));
            let deleted = manager.delete(&name)?;
            if deleted {
                Ok(json!({"success": true, "deleted": name}))
            } else {
                Err(McpError::InvalidParams(format!(
                    "Script '{}' not found",
                    name
                )))
            }
        }
        "script_run" => {
            let name = get_string(&params, "name")?;
            let script_params = params.get("params").cloned();
            let options = params
                .get("max_operations")
                .and_then(|v| v.as_u64())
                .map(ScriptOptions::with_max_operations);

            // Run the script with dependencies and stats tracking
            let manager = ScriptManager::new(Arc::clone(adapter));
            let engine = RhaiEngine::new(Arc::clone(adapter));
            let result = manager.run_script_with_options(&name, script_params, &engine, options)?;

            Ok(json!({
                "success": true,
                "result": result.result,
                "logs": result.logs,
                "execution_time_ms": result.execution_time_ms
            }))
        }
        "script_exec" => {
            let code = get_string(&params, "code")?;
            let script_params = params.get("params").cloned();
            let options = params
                .get("max_operations")
                .and_then(|v| v.as_u64())
                .map(ScriptOptions::with_max_operations);

            // Run inline code directly without saving
            let engine = RhaiEngine::new(Arc::clone(adapter));
            let result = match options {
                Some(opts) => engine.run_with_options(&code, script_params, opts)?,
                None => engine.run(&code, script_params)?,
            };

            Ok(json!({
                "success": true,
                "result": result.result,
                "logs": result.logs,
                "execution_time_ms": result.execution_time_ms
            }))
        }
        // Version Management
        "script_history" => {
            let name = get_string(&params, "name")?;
            let limit = params
                .get("limit")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);
            let manager = ScriptManager::new(Arc::clone(adapter));
            let history = manager.get_history(&name, limit)?;
            Ok(json!({"history": history, "count": history.len()}))
        }
        "script_rollback" => {
            let name = get_string(&params, "name")?;
            let version = params
                .get("version")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| McpError::InvalidParams("version is required".to_string()))?
                as u32;
            let manager = ScriptManager::new(Arc::clone(adapter));
            let new_version = manager.rollback(&name, version)?;
            Ok(json!({"success": true, "name": name, "new_version": new_version}))
        }
        "script_version_get" => {
            let name = get_string(&params, "name")?;
            let version = params
                .get("version")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| McpError::InvalidParams("version is required".to_string()))?
                as u32;
            let manager = ScriptManager::new(Arc::clone(adapter));
            match manager.get_version(&name, version)? {
                Some(v) => Ok(json!({
                    "script_name": v.script_name,
                    "version": v.version,
                    "code": v.code,
                    "description": v.description,
                    "tags": v.tags,
                    "dependencies": v.dependencies,
                    "created_at": v.created_at
                })),
                None => Err(McpError::InvalidParams(format!(
                    "Version {} of script '{}' not found",
                    version, name
                ))),
            }
        }
        // Tag Management
        "script_tags_add" => {
            let name = get_string(&params, "name")?;
            let tags: Vec<String> = params
                .get("tags")
                .and_then(|v| {
                    v.as_array().map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                })
                .ok_or_else(|| McpError::InvalidParams("tags array is required".to_string()))?;
            let manager = ScriptManager::new(Arc::clone(adapter));
            manager.add_tags(&name, tags.clone())?;
            Ok(json!({"success": true, "name": name, "added_tags": tags}))
        }
        "script_tags_remove" => {
            let name = get_string(&params, "name")?;
            let tags: Vec<String> = params
                .get("tags")
                .and_then(|v| {
                    v.as_array().map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                })
                .ok_or_else(|| McpError::InvalidParams("tags array is required".to_string()))?;
            let manager = ScriptManager::new(Arc::clone(adapter));
            manager.remove_tags(&name, tags.clone())?;
            Ok(json!({"success": true, "name": name, "removed_tags": tags}))
        }
        // Execution Statistics
        "script_stats" => {
            let name = get_string(&params, "name")?;
            let manager = ScriptManager::new(Arc::clone(adapter));
            match manager.get_stats(&name)? {
                Some(stats) => Ok(json!({
                    "name": stats.name,
                    "execution_count": stats.execution_count,
                    "last_run_at": stats.last_run_at,
                    "last_run_success": stats.last_run_success,
                    "total_execution_time_ms": stats.total_execution_time_ms,
                    "avg_execution_time_ms": stats.avg_execution_time_ms
                })),
                None => Err(McpError::InvalidParams(format!(
                    "Script '{}' not found",
                    name
                ))),
            }
        }

        // Admin Operations (require IRONBASE_ADMIN_KEY)
        "admin_list_all_collections" => {
            verify_admin_key(&params)?;
            let collections = adapter.list_all_collections();
            Ok(json!({"collections": collections, "count": collections.len()}))
        }
        "admin_create_system_collection" => {
            verify_admin_key(&params)?;
            let name = get_string(&params, "name")?;
            adapter.create_system_collection(&name)?;
            Ok(
                json!({"success": true, "collection": name, "flags": {"is_system": true, "protected": true, "hidden": false}}),
            )
        }
        "admin_set_collection_flags" => {
            verify_admin_key(&params)?;
            let collection = get_string(&params, "collection")?;
            let is_system = params.get("is_system").and_then(|v| v.as_bool());
            let protected = params.get("protected").and_then(|v| v.as_bool());
            let hidden = params.get("hidden").and_then(|v| v.as_bool());
            adapter.set_collection_flags(&collection, is_system, protected, hidden)?;
            Ok(
                json!({"success": true, "collection": collection, "flags": {"is_system": is_system, "protected": protected, "hidden": hidden}}),
            )
        }
        "admin_drop_protected" => {
            verify_admin_key(&params)?;
            let name = get_string(&params, "name")?;
            adapter.force_drop_collection(&name)?;
            Ok(json!({"success": true, "dropped": name}))
        }

        // API Key Management (require IRONBASE_ADMIN_KEY)
        "admin_apikey_create" => {
            verify_admin_key(&params)?;
            let name = get_string(&params, "name")?;

            // Use provided cache or create temporary one (for stdio mode)
            let temp_cache;
            let cache = match api_key_cache {
                Some(c) => c,
                None => {
                    temp_cache = crate::api_keys::ApiKeyCache::new(60, false);
                    &temp_cache
                }
            };

            match crate::api_keys::create_api_key(adapter, &name, cache) {
                Ok(api_key) => Ok(json!({
                    "success": true,
                    "id": api_key._id,
                    "key": api_key.key,
                    "name": api_key.name,
                    "created_at": api_key.created_at,
                    "note": "Save this key now - it cannot be retrieved later!"
                })),
                Err(e) => Err(McpError::Internal(e)),
            }
        }
        "admin_apikey_list" => {
            verify_admin_key(&params)?;
            match crate::api_keys::list_api_keys(adapter) {
                Ok(keys) => Ok(json!({
                    "success": true,
                    "keys": keys,
                    "count": keys.len()
                })),
                Err(e) => Err(McpError::Internal(e)),
            }
        }
        "admin_apikey_revoke" => {
            verify_admin_key(&params)?;
            let id = params
                .get("id")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| McpError::InvalidParams("id parameter is required".into()))?;

            // Use provided cache or create temporary one (for stdio mode)
            let temp_cache;
            let cache = match api_key_cache {
                Some(c) => c,
                None => {
                    temp_cache = crate::api_keys::ApiKeyCache::new(60, false);
                    &temp_cache
                }
            };

            match crate::api_keys::revoke_api_key(adapter, id, cache) {
                Ok(true) => Ok(json!({"success": true, "id": id, "status": "revoked"})),
                Ok(false) => Ok(json!({"success": false, "id": id, "error": "API key not found"})),
                Err(e) => Err(McpError::Internal(e)),
            }
        }
        "admin_apikey_delete" => {
            verify_admin_key(&params)?;
            let id = params
                .get("id")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| McpError::InvalidParams("id parameter is required".into()))?;

            // Use provided cache or create temporary one (for stdio mode)
            let temp_cache;
            let cache = match api_key_cache {
                Some(c) => c,
                None => {
                    temp_cache = crate::api_keys::ApiKeyCache::new(60, false);
                    &temp_cache
                }
            };

            match crate::api_keys::delete_api_key(adapter, id, cache) {
                Ok(true) => Ok(json!({"success": true, "id": id, "status": "deleted"})),
                Ok(false) => Ok(json!({"success": false, "id": id, "error": "API key not found"})),
                Err(e) => Err(McpError::Internal(e)),
            }
        }

        // Transaction Management
        "begin_transaction" => {
            let tx_id = adapter.begin_transaction();
            Ok(json!({
                "transaction_id": tx_id.to_string(),
                "message": "Transaction started. Use _tx operations with this ID. Only one write transaction can be active at a time."
            }))
        }
        "commit_transaction" => {
            let tx_id = parse_transaction_id(&params)?;
            adapter.commit_transaction(tx_id)?;
            Ok(json!({
                "success": true,
                "message": "Transaction committed successfully"
            }))
        }
        "rollback_transaction" => {
            let tx_id = parse_transaction_id(&params)?;
            adapter.rollback_transaction(tx_id)?;
            Ok(json!({
                "success": true,
                "message": "Transaction rolled back successfully"
            }))
        }
        "insert_one_tx" => {
            let tx_id = parse_transaction_id(&params)?;
            let collection = get_string(&params, "collection")?;
            validate_collection_name(&collection)?;
            let document = get_object(&params, "document")?;
            let id = adapter.insert_one_tx(&collection, document, tx_id)?;
            Ok(json!({"inserted_id": id}))
        }
        "update_one_tx" => {
            let tx_id = parse_transaction_id(&params)?;
            let collection = get_string(&params, "collection")?;
            validate_collection_name(&collection)?;
            let filter = get_object(&params, "filter")?;
            let update = get_object(&params, "update")?;
            let result = adapter.update_one_tx(&collection, filter, update, tx_id)?;
            Ok(json!({
                "matched_count": result.matched_count,
                "modified_count": result.modified_count
            }))
        }
        "delete_one_tx" => {
            let tx_id = parse_transaction_id(&params)?;
            let collection = get_string(&params, "collection")?;
            validate_collection_name(&collection)?;
            let filter = get_object(&params, "filter")?;
            let count = adapter.delete_one_tx(&collection, filter, tx_id)?;
            Ok(json!({"deleted_count": count}))
        }
        "transaction_status" => {
            let holder = adapter.get_write_lock_holder();
            Ok(json!({
                "has_active_write_transaction": holder.is_some(),
                "write_lock_holder": holder.map(|id| id.to_string())
            }))
        }

        // ACL Management
        "acl_list" => {
            use crate::acl::SYSTEM_ACL_COLLECTION;
            use crate::adapter::FindOptions;

            // List all ACL rules from _system.acl
            match adapter.find(SYSTEM_ACL_COLLECTION, json!({}), FindOptions::default()) {
                Ok(result) => Ok(json!({
                    "rules": result.documents,
                    "count": result.documents.len(),
                    "note": "Built-in rules (_system.* protection) are not shown here"
                })),
                Err(_) => {
                    // Collection doesn't exist yet - no custom rules
                    Ok(json!({
                        "rules": [],
                        "count": 0,
                        "note": "No custom ACL rules defined. Default rules apply."
                    }))
                }
            }
        }

        "acl_get" => {
            use crate::acl::SYSTEM_ACL_COLLECTION;

            let collection = get_string(&params, "collection")?;

            match adapter.find_one(SYSTEM_ACL_COLLECTION, json!({"collection": collection})) {
                Ok(Some(doc)) => Ok(doc),
                Ok(None) => Ok(json!({
                    "collection": collection,
                    "rules": null,
                    "note": "No custom ACL for this collection. Default rules apply."
                })),
                Err(_) => Ok(json!({
                    "collection": collection,
                    "rules": null,
                    "note": "No custom ACL for this collection. Default rules apply."
                })),
            }
        }

        "acl_set" => {
            use crate::acl::{Permissions, Principal, SYSTEM_ACL_COLLECTION};

            let collection = get_string(&params, "collection")?;
            let rules_arr = get_array(&params, "rules")?;

            // Validate that collection exists (except for wildcard "*")
            if collection != "*" {
                let collections = adapter.list_collections();
                if !collections.contains(&collection) {
                    return Err(McpError::InvalidParams(format!(
                        "Collection '{}' does not exist. Create it first before setting ACL.",
                        collection
                    )));
                }
            }

            // Parse rules
            let mut parsed_rules: Vec<Value> = Vec::new();
            for rule_value in rules_arr {
                let principal_str = rule_value
                    .get("principal")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| McpError::InvalidParams("Rule missing 'principal'".into()))?;

                let permissions_str = rule_value
                    .get("permissions")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| McpError::InvalidParams("Rule missing 'permissions'".into()))?;

                // Validate principal format
                let principal = Principal::parse(principal_str)?;
                let permissions = Permissions::parse(permissions_str);

                parsed_rules.push(json!({
                    "principal": principal,
                    "permissions": permissions
                }));
            }

            let acl_doc = json!({
                "collection": collection,
                "rules": parsed_rules
            });

            // Upsert into _system.acl
            let filter = json!({"collection": collection});
            match adapter.find_one(SYSTEM_ACL_COLLECTION, filter.clone()) {
                Ok(Some(_)) => {
                    // Update existing
                    adapter.update_one(SYSTEM_ACL_COLLECTION, filter, json!({"$set": acl_doc}))?;
                }
                _ => {
                    // Insert new
                    adapter.insert_one(SYSTEM_ACL_COLLECTION, acl_doc)?;
                }
            }

            Ok(json!({
                "success": true,
                "collection": collection,
                "rules_count": parsed_rules.len(),
                "note": "ACL updated. Changes take effect on next request."
            }))
        }

        "acl_delete" => {
            use crate::acl::SYSTEM_ACL_COLLECTION;

            let collection = get_string(&params, "collection")?;

            // Prevent deleting built-in rules
            if collection == "_system.*" {
                return Err(McpError::InvalidParams(
                    "Cannot delete built-in _system.* ACL rules".into(),
                ));
            }

            let filter = json!({"collection": collection});
            let deleted = adapter.delete_one(SYSTEM_ACL_COLLECTION, filter)?;

            Ok(json!({
                "success": true,
                "collection": collection,
                "deleted": deleted > 0,
                "note": if deleted > 0 {
                    "ACL deleted. Default rules now apply."
                } else {
                    "No custom ACL found for this collection."
                }
            }))
        }

        "acl_cleanup" => {
            use crate::acl::SYSTEM_ACL_COLLECTION;

            let existing_collections = adapter.list_collections();

            // Get all ACL rules
            let acl_result =
                adapter.find(SYSTEM_ACL_COLLECTION, json!({}), FindOptions::default())?;

            let mut orphans: Vec<String> = Vec::new();
            for doc in &acl_result.documents {
                if let Some(coll) = doc.get("collection").and_then(|v| v.as_str()) {
                    // Skip wildcard rules
                    if coll == "*" {
                        continue;
                    }
                    // Check if collection exists
                    if !existing_collections.contains(&coll.to_string()) {
                        orphans.push(coll.to_string());
                    }
                }
            }

            // Delete orphan ACLs
            let mut deleted_count = 0;
            for orphan in &orphans {
                let filter = json!({"collection": orphan});
                if adapter
                    .delete_one(SYSTEM_ACL_COLLECTION, filter)
                    .unwrap_or(0)
                    > 0
                {
                    deleted_count += 1;
                }
            }

            Ok(json!({
                "success": true,
                "orphans_found": orphans.len(),
                "orphans_deleted": deleted_count,
                "collections": orphans
            }))
        }

        // Listener Management
        "listener_list" => {
            use crate::listener::{ListenerManager, SYSTEM_LISTENERS_COLLECTION};

            let manager = ListenerManager::new(adapter.clone());
            // Handle case where collection doesn't exist yet
            let listeners = manager.list().unwrap_or_default();

            Ok(json!({
                "listeners": listeners,
                "count": listeners.len(),
                "collection": SYSTEM_LISTENERS_COLLECTION,
                "note": "Changes require server restart to take effect"
            }))
        }

        "listener_get" => {
            use crate::listener::ListenerManager;

            let id = get_string(&params, "id")?;
            let manager = ListenerManager::new(adapter.clone());

            // Handle case where collection doesn't exist yet
            match manager.get(&id).unwrap_or(None) {
                Some(listener) => Ok(serde_json::to_value(listener)?),
                None => Err(McpError::InvalidParams(format!(
                    "Listener not found: {}",
                    id
                ))),
            }
        }

        "listener_add" => {
            use crate::listener::{ListenerConfig, ListenerManager};

            let id = get_string(&params, "id")?;
            let bind = get_string(&params, "bind")?;
            let tls = params.get("tls").and_then(|v| v.as_bool()).unwrap_or(false);
            let cert_path = params
                .get("cert_path")
                .and_then(|v| v.as_str())
                .map(String::from);
            let key_path = params
                .get("key_path")
                .and_then(|v| v.as_str())
                .map(String::from);
            let description = params
                .get("description")
                .and_then(|v| v.as_str())
                .map(String::from);

            let config = ListenerConfig {
                id: id.clone(),
                bind: bind.clone(),
                tls,
                cert_path,
                key_path,
                enabled: true,
                description,
            };

            // Validate before saving
            config.validate()?;

            let manager = ListenerManager::new(adapter.clone());
            // Handle case where collection doesn't exist yet (is_update will be false)
            let is_update = manager.get(&id).unwrap_or(None).is_some();
            manager.set(&config)?;

            Ok(json!({
                "success": true,
                "id": id,
                "bind": bind,
                "tls": tls,
                "action": if is_update { "updated" } else { "created" },
                "note": "Restart server for changes to take effect"
            }))
        }

        "listener_delete" => {
            use crate::listener::ListenerManager;

            let id = get_string(&params, "id")?;

            // Prevent deleting the default listener
            if id == "default" {
                return Err(McpError::InvalidParams(
                    "Cannot delete the default listener. Use listener_disable instead.".into(),
                ));
            }

            let manager = ListenerManager::new(adapter.clone());
            // Handle case where collection doesn't exist yet
            let deleted = manager.delete(&id).unwrap_or(false);

            Ok(json!({
                "success": true,
                "id": id,
                "deleted": deleted,
                "note": if deleted {
                    "Listener deleted. Restart server for changes to take effect."
                } else {
                    "Listener not found."
                }
            }))
        }

        "listener_enable" => {
            use crate::listener::ListenerManager;

            let id = get_string(&params, "id")?;
            let manager = ListenerManager::new(adapter.clone());
            // Handle case where collection doesn't exist yet
            let updated = manager.enable(&id).unwrap_or(false);

            Ok(json!({
                "success": true,
                "id": id,
                "enabled": updated,
                "note": if updated {
                    "Listener enabled. Restart server for changes to take effect."
                } else {
                    "Listener not found."
                }
            }))
        }

        "listener_disable" => {
            use crate::listener::ListenerManager;

            let id = get_string(&params, "id")?;
            let manager = ListenerManager::new(adapter.clone());
            // Handle case where collection doesn't exist yet
            let updated = manager.disable(&id).unwrap_or(false);

            Ok(json!({
                "success": true,
                "id": id,
                "disabled": updated,
                "note": if updated {
                    "Listener disabled. Restart server for changes to take effect."
                } else {
                    "Listener not found."
                }
            }))
        }

        _ => Err(McpError::InvalidParams(format!("Unknown tool: {}", name))),
    }
}

// Helper functions to extract typed values from params

fn get_string(params: &Value, key: &str) -> Result<String> {
    params
        .get(key)
        .and_then(|v| v.as_str())
        .map(String::from)
        .ok_or_else(|| McpError::InvalidParams(format!("Missing or invalid '{}' parameter", key)))
}

fn get_object(params: &Value, key: &str) -> Result<Value> {
    params
        .get(key)
        .filter(|v| v.is_object())
        .cloned()
        .ok_or_else(|| {
            McpError::InvalidParams(format!(
                "Missing or invalid '{}' parameter (expected object)",
                key
            ))
        })
}

fn get_array(params: &Value, key: &str) -> Result<Vec<Value>> {
    params
        .get(key)
        .and_then(|v| v.as_array())
        .cloned()
        .ok_or_else(|| {
            McpError::InvalidParams(format!(
                "Missing or invalid '{}' parameter (expected array)",
                key
            ))
        })
}

/// Parse transaction_id from params (can be string or number)
fn parse_transaction_id(params: &Value) -> Result<u64> {
    let tx_param = params
        .get("transaction_id")
        .ok_or_else(|| McpError::InvalidParams("transaction_id parameter is required".into()))?;

    // Accept both string and number formats
    if let Some(s) = tx_param.as_str() {
        s.parse::<u64>()
            .map_err(|_| McpError::InvalidParams(format!("Invalid transaction_id: {}", s)))
    } else if let Some(n) = tx_param.as_u64() {
        Ok(n)
    } else {
        Err(McpError::InvalidParams(
            "transaction_id must be a string or number".into(),
        ))
    }
}
