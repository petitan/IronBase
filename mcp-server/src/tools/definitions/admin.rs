//! Admin operations tool definitions
//!
//! Tools: admin_list_all_collections, admin_create_system_collection,
//!        admin_set_collection_flags, admin_drop_protected,
//!        admin_apikey_create, admin_apikey_list, admin_apikey_revoke,
//!        admin_apikey_delete

use super::common::{fields, schemas};
use serde_json::{json, Value};

pub fn tools() -> Vec<Value> {
    vec![
        json!({
            "name": "admin_list_all_collections",
            "title": "List All Collections (Admin)",
            "description": "List all collections including system and hidden ones.",
            "inputSchema": schemas::admin_key_only()
        }),
        json!({
            "name": "admin_create_system_collection",
            "title": "Create System Collection (Admin)",
            "description": "Create a protected system collection with special flags.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "admin_key": fields::admin_key(),
                    "collection": {
                        "type": "string",
                        "description": "Collection name (conventionally prefixed with _system.)"
                    }
                },
                "required": ["admin_key", "collection"]
            }
        }),
        json!({
            "name": "admin_set_collection_flags",
            "title": "Set Collection Flags (Admin)",
            "description": "Modify collection protection and visibility flags.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "admin_key": fields::admin_key(),
                    "collection": fields::collection(),
                    "is_system": {
                        "type": "boolean",
                        "description": "Mark as system collection (restricts modifications)"
                    },
                    "protected": {
                        "type": "boolean",
                        "description": "Prevent accidental deletion"
                    },
                    "hidden": {
                        "type": "boolean",
                        "description": "Hide from collection_list results"
                    }
                },
                "required": ["admin_key", "collection"]
            }
        }),
        json!({
            "name": "admin_drop_protected",
            "title": "Drop Protected Collection (Admin)",
            "description": "Force-delete a protected collection.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "admin_key": fields::admin_key(),
                    "collection": {
                        "type": "string",
                        "description": "Collection name to delete"
                    }
                },
                "required": ["admin_key", "collection"]
            }
        }),
        json!({
            "name": "admin_apikey_create",
            "title": "Create API Key (Admin)",
            "description": "Generate a new API key for client authentication.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "admin_key": fields::admin_key(),
                    "name": {
                        "type": "string",
                        "description": "Descriptive name for the API key"
                    }
                },
                "required": ["admin_key", "name"]
            }
        }),
        json!({
            "name": "admin_apikey_list",
            "title": "List API Keys (Admin)",
            "description": "List all API keys with masked values.",
            "inputSchema": schemas::admin_key_only()
        }),
        json!({
            "name": "admin_apikey_revoke",
            "title": "Revoke API Key (Admin)",
            "description": "Disable an API key without deleting it.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "admin_key": fields::admin_key(),
                    "id": {
                        "type": "integer",
                        "description": "API key ID to revoke"
                    }
                },
                "required": ["admin_key", "id"]
            }
        }),
        json!({
            "name": "admin_apikey_delete",
            "title": "Delete API Key (Admin)",
            "description": "Permanently remove an API key.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "admin_key": fields::admin_key(),
                    "id": {
                        "type": "integer",
                        "description": "API key ID to delete"
                    }
                },
                "required": ["admin_key", "id"]
            }
        }),
    ]
}
