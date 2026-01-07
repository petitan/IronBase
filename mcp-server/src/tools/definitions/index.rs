//! Index management tool definitions
//!
//! Tools: index_create, index_list, index_create_fuzzy, index_create_fulltext,
//!        fulltext_search, fuzzy_search, index_list_fulltext, index_drop,
//!        explain, find_with_hint

use super::common::{fields, schemas};
use serde_json::{json, Value};

pub fn tools() -> Vec<Value> {
    vec![
        json!({
            "name": "index_create",
            "title": "Create B+ Tree Index",
            "description": "Create a B+ tree index on one or more fields to accelerate queries.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "collection": fields::collection(),
                    "field": fields::index_field(),
                    "fields": fields::index_fields(),
                    "unique": fields::unique(),
                    "sparse": fields::sparse()
                },
                "required": ["collection"],
                "anyOf": [
                    { "required": ["field"] },
                    { "required": ["fields"] }
                ]
            }
        }),
        json!({
            "name": "index_list",
            "title": "List Indexes",
            "description": "List all indexes defined on a collection.",
            "inputSchema": schemas::collection_only()
        }),
        json!({
            "name": "index_create_fuzzy",
            "title": "Create Fuzzy Search Index",
            "description": "Create an index for approximate string matching using similarity algorithms.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "collection": fields::collection(),
                    "field": {
                        "type": "string",
                        "description": "Text field to index for fuzzy matching"
                    },
                    "algorithm": fields::fuzzy_algorithm(),
                    "threshold": fields::fuzzy_threshold()
                },
                "required": ["collection", "field"]
            }
        }),
        json!({
            "name": "index_create_fulltext",
            "title": "Create Full-Text Search Index",
            "description": "Create a TF-IDF full-text search index with language-aware stemming.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "collection": fields::collection(),
                    "field": {
                        "type": "string",
                        "description": "Text field to index. Supports nested paths: \"body.content\""
                    },
                    "language": fields::fulltext_language(),
                    "min_word_length": fields::min_word_length(),
                    "accent_folding": fields::accent_folding()
                },
                "required": ["collection", "field"]
            }
        }),
        json!({
            "name": "fulltext_search",
            "title": "Full-Text Search",
            "description": "Search documents using TF-IDF relevance scoring. Requires index_create_fulltext first.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "collection": {
                        "type": "string",
                        "description": "Collection with fulltext index"
                    },
                    "field": {
                        "type": "string",
                        "description": "Indexed field to search"
                    },
                    "query": fields::search_query(),
                    "limit": fields::limit_results(10),
                    "skip": fields::skip_results(),
                    "min_score": fields::min_score(),
                    "projection": fields::projection_simple()
                },
                "required": ["collection", "field", "query"]
            }
        }),
        json!({
            "name": "fuzzy_search",
            "title": "Fuzzy String Search",
            "description": "Find documents with approximate string matching. Requires index_create_fuzzy first.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "collection": {
                        "type": "string",
                        "description": "Collection with fuzzy index"
                    },
                    "field": {
                        "type": "string",
                        "description": "Indexed field to search"
                    },
                    "query": fields::fuzzy_query(),
                    "algorithm": {
                        "type": "string",
                        "description": "Override index algorithm for this search",
                        "enum": ["jaro_winkler", "levenshtein", "damerau_levenshtein"]
                    },
                    "threshold": {
                        "type": "number",
                        "description": "Override similarity threshold (0.0-1.0)",
                        "minimum": 0,
                        "maximum": 1
                    },
                    "limit": fields::limit(None),
                    "projection": fields::projection_simple()
                },
                "required": ["collection", "field", "query"]
            }
        }),
        json!({
            "name": "index_list_fulltext",
            "title": "List Full-Text Indexes",
            "description": "List all full-text search indexes on a collection.",
            "inputSchema": schemas::collection_only()
        }),
        json!({
            "name": "index_drop",
            "title": "Drop Index",
            "description": "Remove an index from a collection.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "collection": fields::collection(),
                    "index_name": fields::index_name()
                },
                "required": ["collection", "index_name"]
            }
        }),
        json!({
            "name": "explain",
            "title": "Explain Query Plan",
            "description": "Analyze query execution plan showing which indexes will be used.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "collection": {
                        "type": "string",
                        "description": "Collection to analyze"
                    },
                    "query": {
                        "type": "object",
                        "description": "Query filter to analyze. Shows: scan type, index used, estimated cost."
                    }
                },
                "required": ["collection"]
            }
        }),
        json!({
            "name": "find_with_hint",
            "title": "Query with Index Hint",
            "description": "Execute a query forcing a specific index. Useful for testing index performance.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "collection": fields::collection_query(),
                    "query": {
                        "type": "object",
                        "description": "MongoDB-style filter"
                    },
                    "hint": fields::hint(),
                    "projection": fields::projection_simple(),
                    "sort": fields::sort(),
                    "limit": {
                        "type": "integer",
                        "description": "Maximum documents to return",
                        "default": 10000
                    },
                    "skip": fields::skip()
                },
                "required": ["collection", "hint"]
            }
        }),
    ]
}
