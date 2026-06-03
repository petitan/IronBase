//! Collection management tool definitions
//!
//! Tools: collection_list, collection_create, collection_drop, collection_rename

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
        json!({
            "name": "collection_rename",
            "title": "Rename Collection",
            "description": "Rename a collection. Moves its documents, indexes, schema, ACL entries, and RAG config to the new name (no document copy). Fails if the target name already exists or the source is protected.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "old_collection": {
                        "type": "string",
                        "description": "Current collection name"
                    },
                    "new_collection": {
                        "type": "string",
                        "description": "New collection name. Alphanumeric with underscores allowed; must not already exist."
                    }
                },
                "required": ["old_collection", "new_collection"]
            }
        }),
    ]
}
