//! MCP Prompt definitions for IronBase
//!
//! Prompts are organized into thematic modules:
//! - `guides`: General guides (best-practices, query-operators, aggregation, transactions)
//! - `queries`: Query-related prompts (examples, date-query, wildcard, discover-schema)
//! - `search`: Search prompts (fuzzy, fulltext, schema-validation, index-optimization)
//! - `admin`: Admin prompts (database-admin, security, acl, listener-config, rhai)

mod admin;
mod guides;
mod queries;
mod search;

use serde_json::{json, Value};

/// Get the list of all available prompts for MCP prompts/list
pub fn get_prompts_list() -> Value {
    // TEMPORARILY DISABLED - all prompts
    json!({
        "prompts": []
    })
}

#[allow(dead_code)]
fn get_prompts_list_full() -> Value {
    json!({
        "prompts": [
            {
                "name": "best-practices",
                "description": "Guidelines for safe, efficient database operations to avoid context overflow.",
                "arguments": []
            },
            {
                "name": "discover-schema",
                "description": "Analyze a collection's structure by examining sample documents",
                "arguments": [
                    {"name": "collection", "description": "The collection name to analyze", "required": true},
                    {"name": "sample_size", "description": "Number of documents to sample (default: 10)", "required": false}
                ]
            },
            {
                "name": "query-operators",
                "description": "Reference guide for all IronBase query operators with examples",
                "arguments": []
            },
            {
                "name": "aggregation-guide",
                "description": "Guide for building aggregation pipelines with $match, $group, $project, $sort",
                "arguments": []
            },
            {
                "name": "aggregation-limits",
                "description": "Guide for aggregation memory limits and OOM prevention",
                "arguments": []
            },
            {
                "name": "query-examples",
                "description": "Common query patterns and examples",
                "arguments": [
                    {"name": "category", "description": "Query category: 'crud', 'aggregation', 'indexes', or 'all'", "required": false}
                ]
            },
            {
                "name": "date-query",
                "description": "Help building date range queries from natural language",
                "arguments": [
                    {"name": "date_expression", "description": "Natural language date expression", "required": true},
                    {"name": "date_field", "description": "Name of the date field in documents", "required": true}
                ]
            },
            {
                "name": "wildcard-operator",
                "description": "Guide for using $** recursive descent wildcard operator",
                "arguments": []
            },
            {
                "name": "schema-validation",
                "description": "Guide for JSON schema validation on collections",
                "arguments": [
                    {"name": "collection", "description": "The collection name", "required": true}
                ]
            },
            {
                "name": "index-optimization",
                "description": "Guide for optimizing queries with indexes",
                "arguments": [
                    {"name": "collection", "description": "The collection to analyze", "required": true}
                ]
            },
            {
                "name": "transaction-guide",
                "description": "Guide for ACID transactions with begin, commit, rollback",
                "arguments": []
            },
            {
                "name": "rhai-scripting",
                "description": "Guide for writing Rhai scripts",
                "arguments": []
            },
            {
                "name": "fuzzy-search",
                "description": "Guide for fuzzy text search algorithms",
                "arguments": [
                    {"name": "field", "description": "The field name to search", "required": false}
                ]
            },
            {
                "name": "fulltext-search",
                "description": "Guide for full-text search with TF-IDF",
                "arguments": [
                    {"name": "language", "description": "Language for stemming", "required": false}
                ]
            },
            {
                "name": "acl-guide",
                "description": "Guide for Access Control Lists",
                "arguments": []
            },
            {
                "name": "database-admin",
                "description": "Guide for database administration",
                "arguments": []
            },
            {
                "name": "security-guide",
                "description": "Comprehensive security guide",
                "arguments": []
            },
            {
                "name": "listener-config",
                "description": "Guide for HTTP/HTTPS listener configuration",
                "arguments": []
            }
        ]
    })
}

/// Get prompt content by name
pub fn get_prompt_content(name: &str, arguments: &Value) -> Option<Value> {
    match name {
        // Guides
        "best-practices" => Some(guides::best_practices()),
        "query-operators" => Some(guides::query_operators()),
        "aggregation-guide" => Some(guides::aggregation_guide()),
        "aggregation-limits" => Some(guides::aggregation_limits()),
        "transaction-guide" => Some(guides::transaction_guide()),

        // Queries
        "discover-schema" => Some(queries::discover_schema(arguments)),
        "query-examples" => Some(queries::query_examples(arguments)),
        "date-query" => Some(queries::date_query(arguments)),
        "wildcard-operator" => Some(queries::wildcard_operator()),

        // Search
        "fuzzy-search" => Some(search::fuzzy_search(arguments)),
        "fulltext-search" => Some(search::fulltext_search(arguments)),
        "schema-validation" => Some(search::schema_validation(arguments)),
        "index-optimization" => Some(search::index_optimization(arguments)),

        // Admin
        "database-admin" => Some(admin::database_admin()),
        "security-guide" => Some(admin::security_guide()),
        "acl-guide" => Some(admin::acl_guide()),
        "listener-config" => Some(admin::listener_config()),
        "rhai-scripting" => Some(admin::rhai_scripting()),

        _ => None,
    }
}
