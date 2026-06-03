//! Document CRUD tool definitions
//!
//! Tools: insert_one, insert_many, find, find_one, update_one, update_many,
//!        delete_one, delete_many, count_documents, distinct, aggregate

use super::common::fields;
use serde_json::{json, Value};

pub fn tools() -> Vec<Value> {
    vec![
        json!({
            "name": "insert_one",
            "title": "Insert Document",
            "description": "Insert a single document into a collection. Returns the inserted document ID.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "collection": fields::collection_target(),
                    "document": fields::document()
                },
                "required": ["collection", "document"]
            }
        }),
        json!({
            "name": "insert_many",
            "title": "Bulk Insert Documents",
            "description": "Insert multiple documents into a collection in a single operation.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "collection": fields::collection_target(),
                    "documents": fields::documents()
                },
                "required": ["collection", "documents"]
            }
        }),
        json!({
            "name": "find",
            "title": "Query Documents",
            "description": "Query documents matching a filter with optional projection, sorting, and pagination.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "collection": fields::collection_query(),
                    "query": fields::query(),
                    "projection": fields::projection(),
                    "sort": fields::sort(),
                    "limit": {
                        "type": "integer",
                        "description": "Maximum documents to return. Default: 10,000. Use with large collections to prevent timeout."
                    },
                    "skip": fields::skip(),
                    "include_total": fields::include_total()
                },
                "required": ["collection"]
            }
        }),
        json!({
            "name": "find_one",
            "title": "Find Single Document",
            "description": "Retrieve the first document matching a query.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "collection": fields::collection_query(),
                    "query": fields::query_simple(),
                    "projection": fields::projection_simple()
                },
                "required": ["collection"]
            }
        }),
        json!({
            "name": "update_one",
            "title": "Update Single Document",
            "description": "Update the first document matching a filter. Use upsert=true to insert if no match found.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "collection": {
                        "type": "string",
                        "description": "Collection containing the document"
                    },
                    "filter": fields::filter(),
                    "update": fields::update(),
                    "upsert": {
                        "type": "boolean",
                        "description": "If true, insert a new document when no match is found. The new document is created from filter fields + update operations. Default: false",
                        "default": false
                    }
                },
                "required": ["collection", "filter", "update"]
            }
        }),
        json!({
            "name": "update_many",
            "title": "Bulk Update Documents",
            "description": "Update all documents matching a filter.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "collection": {
                        "type": "string",
                        "description": "Collection containing the documents"
                    },
                    "filter": {
                        "type": "object",
                        "description": "Query filter. Example: {\"status\": \"pending\"}"
                    },
                    "update": {
                        "type": "object",
                        "description": "Update operations to apply to all matches. Example: {\"$set\": {\"status\": \"processed\"}}"
                    }
                },
                "required": ["collection", "filter", "update"]
            }
        }),
        json!({
            "name": "delete_one",
            "title": "Delete Single Document",
            "description": "Delete the first document matching a filter.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "collection": {
                        "type": "string",
                        "description": "Collection containing the document"
                    },
                    "filter": fields::filter()
                },
                "required": ["collection", "filter"]
            }
        }),
        json!({
            "name": "delete_many",
            "title": "Bulk Delete Documents",
            "description": "Delete all documents matching a filter.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "collection": {
                        "type": "string",
                        "description": "Collection containing the documents"
                    },
                    "filter": {
                        "type": "object",
                        "description": "Query filter. Use {} to delete all documents (with caution)."
                    }
                },
                "required": ["collection", "filter"]
            }
        }),
        json!({
            "name": "count",
            "title": "Count Documents",
            "description": "Count documents matching a query filter.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "collection": fields::collection_query(),
                    "query": fields::query_count()
                },
                "required": ["collection"]
            }
        }),
        json!({
            "name": "distinct",
            "title": "Get Distinct Values",
            "description": "Retrieve unique values for a specified field across documents.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "collection": fields::collection_query(),
                    "field": fields::field_path(),
                    "query": {
                        "type": "object",
                        "description": "Optional filter to limit which documents are considered"
                    }
                },
                "required": ["collection", "field"]
            }
        }),
        json!({
            "name": "aggregate",
            "title": "Aggregation Pipeline",
            "description": "Execute a multi-stage data processing pipeline for analytics and transformations.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "collection": {
                        "type": "string",
                        "description": "Source collection"
                    },
                    "pipeline": fields::pipeline()
                },
                "required": ["collection", "pipeline"]
            }
        }),
    ]
}
