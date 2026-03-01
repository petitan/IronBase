//! Reusable field definitions for MCP tool schemas
//!
//! This module provides common JSON Schema field definitions to reduce
//! duplication across tool definitions. Each field is a function that
//! returns a `serde_json::Value` representing the field schema.

use serde_json::{json, Value};

/// Common field definitions used across multiple tools
#[allow(dead_code)]
pub mod fields {
    use super::*;

    /// Collection name field
    pub fn collection() -> Value {
        json!({ "type": "string", "description": "Collection name" })
    }

    /// Collection name for target operations
    pub fn collection_target() -> Value {
        json!({ "type": "string", "description": "Target collection name" })
    }

    /// Collection name for query operations
    pub fn collection_query() -> Value {
        json!({ "type": "string", "description": "Collection to query" })
    }

    /// MongoDB-style query filter
    pub fn query() -> Value {
        json!({
            "type": "object",
            "description": "MongoDB-style filter. Operators: $eq, $ne, $gt, $gte, $lt, $lte, $in, $nin, $and, $or, $not, $regex, $exists. Example: {\"age\": {\"$gte\": 18}}"
        })
    }

    /// Simple query filter for find_one
    pub fn query_simple() -> Value {
        json!({
            "type": "object",
            "description": "MongoDB-style filter. Example: {\"_id\": 123} or {\"email\": \"user@example.com\"}"
        })
    }

    /// Query filter for update/delete operations
    pub fn filter() -> Value {
        json!({
            "type": "object",
            "description": "Query filter to match the document. Example: {\"_id\": 123}"
        })
    }

    /// Query filter for count operations
    pub fn query_count() -> Value {
        json!({
            "type": "object",
            "description": "MongoDB-style filter. Use $gte/$lt for indexed range queries. Example: {\"date\": {\"$gte\": \"2024-01-01\", \"$lt\": \"2025-01-01\"}}"
        })
    }

    /// Projection field (include/exclude)
    pub fn projection() -> Value {
        json!({
            "type": "object",
            "description": "Fields to include (1) or exclude (0). Example: {\"name\": 1, \"email\": 1} or {\"password\": 0}"
        })
    }

    /// Simple projection
    pub fn projection_simple() -> Value {
        json!({
            "type": "object",
            "description": "Fields to include (1) or exclude (0)"
        })
    }

    /// Sort specification (array or object)
    pub fn sort() -> Value {
        json!({
            "description": "Sort specification. Array format: [[\"field\", 1]] for ascending, [[\"field\", -1]] for descending. Object format: {\"field\": 1}",
            "oneOf": [
                { "type": "array" },
                { "type": "object" }
            ]
        })
    }

    /// Limit field with optional default
    pub fn limit(default: Option<i32>) -> Value {
        let mut field = json!({
            "type": "integer",
            "description": "Maximum documents to return"
        });
        if let Some(d) = default {
            field["default"] = json!(d);
        }
        field
    }

    /// Limit field for search results
    pub fn limit_results(default: i32) -> Value {
        json!({
            "type": "integer",
            "description": "Maximum results to return",
            "default": default
        })
    }

    /// Skip field for pagination
    pub fn skip() -> Value {
        json!({
            "type": "integer",
            "description": "Number of documents to skip for pagination"
        })
    }

    /// Skip field for search results
    pub fn skip_results() -> Value {
        json!({
            "type": "integer",
            "description": "Results to skip for pagination",
            "default": 0
        })
    }

    /// Admin authentication key
    pub fn admin_key() -> Value {
        json!({
            "type": "string",
            "description": "Admin authentication key (IRONBASE_ADMIN_KEY)"
        })
    }

    /// Document to insert
    pub fn document() -> Value {
        json!({
            "type": "object",
            "description": "JSON document to insert. Auto-generates _id if not provided."
        })
    }

    /// Documents array for bulk insert
    pub fn documents() -> Value {
        json!({
            "type": "array",
            "items": { "type": "object" },
            "description": "Array of JSON documents to insert"
        })
    }

    /// Update operations
    pub fn update() -> Value {
        json!({
            "type": "object",
            "description": "Update operations. Operators: $set, $inc, $unset, $push, $pull, $addToSet, $pop. Example: {\"$set\": {\"status\": \"active\"}, \"$inc\": {\"views\": 1}}"
        })
    }

    /// Transaction ID field
    pub fn transaction_id() -> Value {
        json!({
            "type": "string",
            "description": "Active transaction ID"
        })
    }

    /// Transaction ID from begin_transaction
    pub fn transaction_id_from_begin() -> Value {
        json!({
            "type": "string",
            "description": "Transaction ID from begin_transaction"
        })
    }

    /// Field name (single)
    pub fn field() -> Value {
        json!({
            "type": "string",
            "description": "Field name"
        })
    }

    /// Field path with dot notation support
    pub fn field_path() -> Value {
        json!({
            "type": "string",
            "description": "Field path to get distinct values. Supports dot notation: \"address.city\""
        })
    }

    /// Index field for single-field index
    pub fn index_field() -> Value {
        json!({
            "type": "string",
            "description": "Single field to index. Use dot notation for nested: \"address.city\""
        })
    }

    /// Index fields for compound index
    pub fn index_fields() -> Value {
        json!({
            "type": "array",
            "items": { "type": "string" },
            "minItems": 1,
            "description": "Multiple fields for compound index. Order matters for query optimization."
        })
    }

    /// Index name
    pub fn index_name() -> Value {
        json!({
            "type": "string",
            "description": "Index name to drop. Use index_list to see available indexes."
        })
    }

    /// Unique constraint
    pub fn unique() -> Value {
        json!({
            "type": "boolean",
            "description": "Enforce unique values. Rejects duplicate inserts.",
            "default": false
        })
    }

    /// Sparse index option
    pub fn sparse() -> Value {
        json!({
            "type": "boolean",
            "description": "Only index documents where field exists. Optimizes $exists:true queries.",
            "default": false
        })
    }

    /// Script name
    pub fn script_name() -> Value {
        json!({
            "type": "string",
            "description": "Script name"
        })
    }

    /// Script code
    pub fn script_code() -> Value {
        json!({
            "type": "string",
            "description": "Rhai script source code"
        })
    }

    /// Script parameters
    pub fn script_params() -> Value {
        json!({
            "type": "object",
            "description": "Parameters passed to script as 'params' variable"
        })
    }

    /// Script tags array
    pub fn script_tags() -> Value {
        json!({
            "type": "array",
            "items": { "type": "string" },
            "description": "Tags for categorization and filtering"
        })
    }

    /// Max operations for scripts
    pub fn max_operations() -> Value {
        json!({
            "type": "integer",
            "description": "Maximum Rhai operations before timeout",
            "default": 1000000
        })
    }

    /// Version number
    pub fn version() -> Value {
        json!({
            "type": "integer",
            "description": "Version number"
        })
    }

    /// Listener ID
    pub fn listener_id() -> Value {
        json!({
            "type": "string",
            "description": "Listener identifier"
        })
    }

    /// Network bind address
    pub fn bind_address() -> Value {
        json!({
            "type": "string",
            "description": "Network address to bind. Example: \"0.0.0.0:8080\" or \"127.0.0.1:8443\""
        })
    }

    /// TLS enable flag
    pub fn tls_enabled() -> Value {
        json!({
            "type": "boolean",
            "description": "Enable TLS/HTTPS",
            "default": false
        })
    }

    /// Include total count flag
    pub fn include_total() -> Value {
        json!({
            "type": "boolean",
            "description": "Include total matching count in response for pagination UI",
            "default": false
        })
    }

    /// Fuzzy search algorithm
    pub fn fuzzy_algorithm() -> Value {
        json!({
            "type": "string",
            "description": "Similarity algorithm",
            "enum": ["jaro_winkler", "levenshtein", "damerau_levenshtein"],
            "default": "jaro_winkler"
        })
    }

    /// Fuzzy threshold
    pub fn fuzzy_threshold() -> Value {
        json!({
            "type": "number",
            "description": "Minimum similarity score (0.0-1.0). Higher = stricter matching.",
            "default": 0.8,
            "minimum": 0,
            "maximum": 1
        })
    }

    /// Fulltext language
    pub fn fulltext_language() -> Value {
        json!({
            "type": "string",
            "description": "Language for stemming and stop words",
            "enum": ["hungarian", "english", "german", "none"],
            "default": "none"
        })
    }

    /// Minimum word length
    pub fn min_word_length() -> Value {
        json!({
            "type": "integer",
            "description": "Minimum word length to index",
            "default": 2,
            "minimum": 1
        })
    }

    /// Accent folding option
    pub fn accent_folding() -> Value {
        json!({
            "type": "boolean",
            "description": "Normalize accented characters (a->a, o->o)",
            "default": true
        })
    }

    /// Search query text
    pub fn search_query() -> Value {
        json!({
            "type": "string",
            "description": "Search terms. Multiple words are OR-ed by default. Use quotes for exact phrase match (MongoDB-style): \\\"exact phrase\\\". Examples: 'database optimization' (OR), '\\\"database optimization\\\"' (exact phrase)"
        })
    }

    /// Fuzzy search query text
    pub fn fuzzy_query() -> Value {
        json!({
            "type": "string",
            "description": "Search term. Finds similar strings: \"john\" matches \"Jon\", \"Johan\""
        })
    }

    /// Min score threshold
    pub fn min_score() -> Value {
        json!({
            "type": "number",
            "description": "Minimum BM25 relevance score threshold"
        })
    }

    /// MongoDB-style post-filter for fulltext search results
    pub fn fulltext_filter() -> Value {
        json!({
            "type": "object",
            "description": "MongoDB-style filter applied AFTER BM25 scoring. Use to combine fulltext search with other operators. Example: {\"from.email\": {\"$regex\": \"@company\\\\.com$\"}}"
        })
    }

    /// Highlight enable flag
    pub fn highlight() -> Value {
        json!({
            "type": "boolean",
            "description": "Enable snippet highlighting with <mark> tags around matched terms. Requires the searched field to be included in projection.",
            "default": false
        })
    }

    /// Highlight context characters
    pub fn highlight_context() -> Value {
        json!({
            "type": "integer",
            "description": "Characters to include around each match in snippets",
            "default": 100,
            "minimum": 20,
            "maximum": 500
        })
    }

    /// Highlight max snippets
    pub fn highlight_max_snippets() -> Value {
        json!({
            "type": "integer",
            "description": "Maximum number of snippets to return per field",
            "default": 3,
            "minimum": 1,
            "maximum": 10
        })
    }

    /// Text to analyze
    pub fn text_to_analyze() -> Value {
        json!({
            "type": "string",
            "description": "Text to tokenize and analyze"
        })
    }

    /// Aggregation pipeline
    pub fn pipeline() -> Value {
        json!({
            "type": "array",
            "description": "Array of pipeline stages. Stages: $match (filter), $group (aggregate), $project (reshape), $sort, $limit, $skip, $unwind, $count. Example: [{\"$match\": {\"status\": \"active\"}}, {\"$group\": {\"_id\": \"$category\", \"total\": {\"$sum\": 1}}}, {\"$sort\": {\"total\": -1}}, {\"$limit\": 10}]"
        })
    }

    /// Index hint field
    pub fn hint() -> Value {
        json!({
            "type": "string",
            "description": "Index name to force. Use index_list to see available indexes."
        })
    }

    /// API key ID
    pub fn api_key_id() -> Value {
        json!({
            "type": "integer",
            "description": "API key ID"
        })
    }

    /// Description field
    pub fn description() -> Value {
        json!({
            "type": "string",
            "description": "Human-readable description"
        })
    }
}

/// Common schema patterns for tool inputSchema
#[allow(dead_code)]
pub mod schemas {
    use super::*;

    /// Empty schema (no parameters)
    pub fn empty() -> Value {
        json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    /// Schema with only collection parameter
    pub fn collection_only() -> Value {
        json!({
            "type": "object",
            "properties": {
                "collection": fields::collection()
            },
            "required": ["collection"]
        })
    }

    /// Schema with admin_key only
    pub fn admin_key_only() -> Value {
        json!({
            "type": "object",
            "properties": {
                "admin_key": fields::admin_key()
            },
            "required": ["admin_key"]
        })
    }

    /// Schema with admin_key and name
    pub fn admin_with_name() -> Value {
        json!({
            "type": "object",
            "properties": {
                "admin_key": fields::admin_key(),
                "name": {
                    "type": "string",
                    "description": "Name"
                }
            },
            "required": ["admin_key", "name"]
        })
    }

    /// Schema with admin_key and id
    pub fn admin_with_id() -> Value {
        json!({
            "type": "object",
            "properties": {
                "admin_key": fields::admin_key(),
                "id": fields::api_key_id()
            },
            "required": ["admin_key", "id"]
        })
    }

    /// Schema with listener_id only
    pub fn listener_id_only() -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": fields::listener_id()
            },
            "required": ["id"]
        })
    }
}
