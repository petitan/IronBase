//! MCP Tool definitions and handlers for IronBase

use crate::adapter::{FindOptions, IronBaseAdapter};
use crate::error::{McpError, Result};
use serde_json::{json, Value};

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
                "description": "Find documents using fuzzy text matching. Useful for typo-tolerant search, name matching, and approximate string matching.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": {
                            "type": "string",
                            "description": "Collection name"
                        },
                        "field": {
                            "type": "string",
                            "description": "Field name to search in"
                        },
                        "value": {
                            "type": "string",
                            "description": "Search term to match approximately"
                        },
                        "algorithm": {
                            "type": "string",
                            "description": "Fuzzy algorithm: 'jaro_winkler' (default, fast), 'levenshtein' (accurate), 'damerau_levenshtein' (handles transpositions)",
                            "enum": ["jaro_winkler", "levenshtein", "damerau_levenshtein"]
                        },
                        "threshold": {
                            "type": "number",
                            "description": "Similarity threshold 0.0-1.0. Default 0.8 means 80% similarity required."
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Maximum number of documents to return"
                        }
                    },
                    "required": ["collection", "field", "value"]
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
            }
        ]
    })
}

/// Dispatch a tool call to the appropriate handler
pub fn dispatch_tool(name: &str, params: Value, adapter: &IronBaseAdapter) -> Result<Value> {
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
            let query = params.get("query").cloned().unwrap_or(json!({}));
            let include_total = params
                .get("include_total")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let options = FindOptions {
                projection: params.get("projection").cloned(),
                sort: params.get("sort").cloned(),
                limit: params
                    .get("limit")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as usize),
                skip: params
                    .get("skip")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as usize),
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
            let field = get_string(&params, "field")?;
            let value = get_string(&params, "value")?;

            // Build $fuzzy query
            let fuzzy_filter =
                if params.get("algorithm").is_some() || params.get("threshold").is_some() {
                    let mut fuzzy_obj = json!({"value": value});
                    if let Some(algo) = params.get("algorithm").and_then(|v| v.as_str()) {
                        fuzzy_obj["algorithm"] = json!(algo);
                    }
                    if let Some(threshold) = params.get("threshold").and_then(|v| v.as_f64()) {
                        fuzzy_obj["threshold"] = json!(threshold);
                    }
                    fuzzy_obj
                } else {
                    json!(value)
                };

            let query = json!({ field: { "$fuzzy": fuzzy_filter } });

            let options = FindOptions {
                projection: None,
                sort: None,
                limit: params
                    .get("limit")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as usize),
                skip: None,
                include_total: false,
            };

            let result = adapter.find(&collection, query, options)?;
            Ok(json!({"documents": result.documents, "count": result.documents.len()}))
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
