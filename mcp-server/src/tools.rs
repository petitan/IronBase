//! MCP Tool definitions and handlers for IronBase

use crate::adapter::{FindOptions, IronBaseAdapter};
use crate::error::{McpError, Result};
use crate::scripting::{RhaiEngine, ScriptManager};
use serde_json::{json, Value};
use std::sync::Arc;

/// Maximum limit for queries (DoS protection)
const MAX_QUERY_LIMIT: usize = 10_000;

/// Maximum collection name length
const MAX_COLLECTION_NAME_LEN: usize = 128;

/// Constant-time string comparison to prevent timing attacks
fn constant_time_compare(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    // XOR all bytes - result is 0 only if all match
    let mut result = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        result |= x ^ y;
    }
    result == 0
}

/// Verify admin key from params against IRONBASE_ADMIN_KEY env var
fn verify_admin_key(params: &Value) -> Result<()> {
    let expected = std::env::var("IRONBASE_ADMIN_KEY").map_err(|_| {
        McpError::InvalidParams(
            "Admin operations require IRONBASE_ADMIN_KEY environment variable to be set".into(),
        )
    })?;

    let provided = params
        .get("admin_key")
        .and_then(|v| v.as_str())
        .ok_or_else(|| McpError::InvalidParams("admin_key parameter is required".into()))?;

    // Use constant-time comparison to prevent timing attacks
    if !constant_time_compare(provided.as_bytes(), expected.as_bytes()) {
        return Err(McpError::InvalidParams("Invalid admin_key".into()));
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
    params.get("skip").and_then(|v| v.as_u64()).map(|v| v as usize)
}

/// Validate threshold is in range [0.0, 1.0]
fn parse_threshold(params: &Value) -> Result<Option<f64>> {
    match params.get("threshold").and_then(|v| v.as_f64()) {
        Some(t) if !(0.0..=1.0).contains(&t) => {
            Err(McpError::InvalidParams(format!(
                "threshold must be between 0.0 and 1.0, got: {}",
                t
            )))
        }
        t => Ok(t),
    }
}

/// Validate collection name
fn validate_collection_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(McpError::InvalidParams("Collection name cannot be empty".into()));
    }
    if name.len() > MAX_COLLECTION_NAME_LEN {
        return Err(McpError::InvalidParams(format!(
            "Collection name too long (max {} chars)",
            MAX_COLLECTION_NAME_LEN
        )));
    }
    // Check for invalid characters (allow alphanumeric, underscore, dot, hyphen)
    if !name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '.' || c == '-') {
        return Err(McpError::InvalidParams(
            "Collection name can only contain alphanumeric characters, underscores, dots, and hyphens".into()
        ));
    }
    Ok(())
}

/// Get the list of all available tools for MCP tools/list
pub fn get_tools_list() -> Value {
    json!({
        "tools": [
            // Database Management
            {
                "name": "db_stats",
                "description": "Get database statistics including collection count and names",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            },
            {
                "name": "db_compact",
                "description": "Compact the database file, removing deleted documents and freeing space",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            },
            {
                "name": "db_checkpoint",
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
                "description": "List all collections in the database",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            },
            {
                "name": "collection_create",
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
                "description": "Find documents matching a query with optional projection, sort, limit, skip, and total count",
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
                "description": "Find a single document matching the query",
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
                        }
                    },
                    "required": ["collection", "query"]
                }
            },
            {
                "name": "fuzzy_search",
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
                        }
                    },
                    "required": ["collection", "field", "query"]
                }
            },
            {
                "name": "update_one",
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
                "description": "Execute an aggregation pipeline",
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
                "name": "index_drop",
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
                "description": "Find documents using a specific index (forces index usage)",
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
                        }
                    },
                    "required": ["collection", "query", "hint"]
                }
            },
            // Schema Management
            {
                "name": "schema_set",
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
                "description": "Run a saved script by name with optional parameters. Scripts have access to database functions: db_find, db_find_one, db_insert_one, db_insert_many, db_update_one, db_update_many, db_delete_one, db_delete_many, db_count, db_aggregate. Returns the script result and captured logs.",
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
                        }
                    },
                    "required": ["name"]
                }
            },
            {
                "name": "script_exec",
                "description": "Execute inline Rhai code without saving. Useful for one-off operations. Scripts have access to: db_find, db_find_one, db_insert_one, db_insert_many, db_update_one, db_update_many, db_delete_one, db_delete_many, db_count, db_aggregate.",
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
                        }
                    },
                    "required": ["code"]
                }
            },
            {
                "name": "script_history",
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
            // Admin Operations (require IRONBASE_ADMIN_KEY)
            {
                "name": "admin_list_all_collections",
                "description": "List ALL collections including hidden/system collections. Requires IRONBASE_ADMIN_KEY env var.",
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
                "description": "Create a system collection with protected/hidden flags. Requires IRONBASE_ADMIN_KEY env var.",
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
                "description": "Set collection flags (is_system, protected, hidden). Requires IRONBASE_ADMIN_KEY env var.",
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
                "description": "Force drop a protected collection. Requires IRONBASE_ADMIN_KEY env var. USE WITH CAUTION!",
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
            }
        ]
    })
}

/// Dispatch a tool call to the appropriate handler
pub fn dispatch_tool(name: &str, params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    match name {
        // Database Management
        "db_stats" => Ok(adapter.stats()),
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
            // Collections are created implicitly on first insert
            // Just return success
            Ok(json!({"success": true, "collection": name}))
        }
        "collection_drop" => {
            let name = get_string(&params, "name")?;
            adapter.drop_collection(&name)?;
            Ok(json!({"success": true, "dropped": name}))
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
                sort: params.get("sort").cloned(),
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
            let document = adapter.find_one(&collection, query)?;
            Ok(json!({"document": document}))
        }
        "fuzzy_search" => {
            let collection = get_string(&params, "collection")?;
            validate_collection_name(&collection)?;
            let field = get_string(&params, "field")?;
            let query = get_string(&params, "query")?;
            let threshold = parse_threshold(&params)?;
            let algorithm = params.get("algorithm").and_then(|v| v.as_str());
            let limit = parse_limit(&params);

            // Use the real fuzzy search with index
            let mut results = adapter.fuzzy_search(&collection, &field, &query, threshold, algorithm)?;

            // Apply limit if specified (capped at MAX_QUERY_LIMIT)
            if let Some(lim) = limit {
                results.truncate(lim);
            }

            // Format results with scores
            let documents: Vec<Value> = results
                .into_iter()
                .map(|(doc, score)| {
                    json!({
                        "document": doc,
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
            let documents = adapter.find_with_hint(&collection, query, &hint)?;
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
            let code = get_string(&params, "code")?;
            let description = params.get("description").and_then(|v| v.as_str());
            let tags: Option<Vec<String>> = params.get("tags").and_then(|v| {
                v.as_array().map(|arr| {
                    arr.iter().filter_map(|v| v.as_str().map(String::from)).collect()
                })
            });
            let dependencies: Option<Vec<String>> = params.get("dependencies").and_then(|v| {
                v.as_array().map(|arr| {
                    arr.iter().filter_map(|v| v.as_str().map(String::from)).collect()
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
                        arr.iter().filter_map(|v| v.as_str().map(String::from)).collect()
                    })
                });
                let match_all = params.get("match_all").and_then(|v| v.as_bool()).unwrap_or(false);
                if tags.is_some() {
                    Some(ScriptListFilter { tags, match_all_tags: match_all })
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
                None => Err(McpError::InvalidParams(format!("Script '{}' not found", name))),
            }
        }
        "script_delete" => {
            let name = get_string(&params, "name")?;
            let manager = ScriptManager::new(Arc::clone(adapter));
            let deleted = manager.delete(&name)?;
            if deleted {
                Ok(json!({"success": true, "deleted": name}))
            } else {
                Err(McpError::InvalidParams(format!("Script '{}' not found", name)))
            }
        }
        "script_run" => {
            let name = get_string(&params, "name")?;
            let script_params = params.get("params").cloned();

            // Run the script with dependencies and stats tracking
            let manager = ScriptManager::new(Arc::clone(adapter));
            let engine = RhaiEngine::new(Arc::clone(adapter));
            let result = manager.run_script(&name, script_params, &engine)?;

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

            // Run inline code directly without saving
            let engine = RhaiEngine::new(Arc::clone(adapter));
            let result = engine.run(&code, script_params)?;

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
            let limit = params.get("limit").and_then(|v| v.as_u64()).map(|v| v as usize);
            let manager = ScriptManager::new(Arc::clone(adapter));
            let history = manager.get_history(&name, limit)?;
            Ok(json!({"history": history, "count": history.len()}))
        }
        "script_rollback" => {
            let name = get_string(&params, "name")?;
            let version = params.get("version").and_then(|v| v.as_u64()).ok_or_else(|| {
                McpError::InvalidParams("version is required".to_string())
            })? as u32;
            let manager = ScriptManager::new(Arc::clone(adapter));
            let new_version = manager.rollback(&name, version)?;
            Ok(json!({"success": true, "name": name, "new_version": new_version}))
        }
        "script_version_get" => {
            let name = get_string(&params, "name")?;
            let version = params.get("version").and_then(|v| v.as_u64()).ok_or_else(|| {
                McpError::InvalidParams("version is required".to_string())
            })? as u32;
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
                    "Version {} of script '{}' not found", version, name
                ))),
            }
        }
        // Tag Management
        "script_tags_add" => {
            let name = get_string(&params, "name")?;
            let tags: Vec<String> = params.get("tags").and_then(|v| {
                v.as_array().map(|arr| {
                    arr.iter().filter_map(|v| v.as_str().map(String::from)).collect()
                })
            }).ok_or_else(|| McpError::InvalidParams("tags array is required".to_string()))?;
            let manager = ScriptManager::new(Arc::clone(adapter));
            manager.add_tags(&name, tags.clone())?;
            Ok(json!({"success": true, "name": name, "added_tags": tags}))
        }
        "script_tags_remove" => {
            let name = get_string(&params, "name")?;
            let tags: Vec<String> = params.get("tags").and_then(|v| {
                v.as_array().map(|arr| {
                    arr.iter().filter_map(|v| v.as_str().map(String::from)).collect()
                })
            }).ok_or_else(|| McpError::InvalidParams("tags array is required".to_string()))?;
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
                None => Err(McpError::InvalidParams(format!("Script '{}' not found", name))),
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
            Ok(json!({"success": true, "collection": name, "flags": {"is_system": true, "protected": true, "hidden": false}}))
        }
        "admin_set_collection_flags" => {
            verify_admin_key(&params)?;
            let collection = get_string(&params, "collection")?;
            let is_system = params.get("is_system").and_then(|v| v.as_bool());
            let protected = params.get("protected").and_then(|v| v.as_bool());
            let hidden = params.get("hidden").and_then(|v| v.as_bool());
            adapter.set_collection_flags(&collection, is_system, protected, hidden)?;
            Ok(json!({"success": true, "collection": collection, "flags": {"is_system": is_system, "protected": protected, "hidden": hidden}}))
        }
        "admin_drop_protected" => {
            verify_admin_key(&params)?;
            let name = get_string(&params, "name")?;
            adapter.force_drop_collection(&name)?;
            Ok(json!({"success": true, "dropped": name}))
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
