//! Collection management tool definitions
//!
//! Tools: collection_list, collection_create, collection_drop

use super::common::schemas;
use serde_json::{json, Value};

pub fn tools() -> Vec<Value> {
    vec![
        json!({
            "name": "collection_list",
            "title": "List Collections",
            "description": "List all user collections in the database.",
            "inputSchema": schemas::empty()
        }),
        json!({
            "name": "collection_create",
            "title": "Create Collection",
            "description": "Create an empty collection. Collections are also auto-created on first insert.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "collection": {
                        "type": "string",
                        "description": "Collection name. Alphanumeric with underscores allowed."
                    }
                },
                "required": ["collection"]
            }
        }),
        json!({
            "name": "collection_drop",
            "title": "Drop Collection",
            "description": "Delete a collection and all its documents, indexes, and schema.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "collection": {
                        "type": "string",
                        "description": "Collection name to delete"
                    }
                },
                "required": ["collection"]
            }
        }),
    ]
}
