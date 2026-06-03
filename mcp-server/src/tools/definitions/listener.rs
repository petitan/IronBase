//! HTTP/HTTPS listener management tool definitions
//!
//! Tools: listener_list, listener_get, listener_add, listener_delete,
//!        listener_enable, listener_disable

use super::common::{fields, schemas};
use serde_json::{json, Value};

pub fn tools() -> Vec<Value> {
    vec![
        json!({
            "name": "listener_list",
            "title": "List Listeners",
            "description": "List all configured HTTP/HTTPS listeners.",
            "inputSchema": schemas::empty()
        }),
        json!({
            "name": "listener_get",
            "title": "Get Listener Config",
            "description": "Get configuration details for a specific listener.",
            "inputSchema": schemas::listener_id_only()
        }),
        json!({
            "name": "listener_create",
            "title": "Add Listener",
            "description": "Configure a new HTTP or HTTPS listener endpoint.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "Unique listener identifier"
                    },
                    "bind": fields::bind_address(),
                    "tls": fields::tls_enabled(),
                    "cert_path": {
                        "type": "string",
                        "description": "Path to TLS certificate file (PEM format)"
                    },
                    "key_path": {
                        "type": "string",
                        "description": "Path to TLS private key file (PEM format)"
                    },
                    "description": fields::description()
                },
                "required": ["id", "bind"]
            }
        }),
        json!({
            "name": "listener_delete",
            "title": "Delete Listener",
            "description": "Remove a listener configuration.",
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
        }),
        json!({
            "name": "listener_enable",
            "title": "Enable Listener",
            "description": "Activate a disabled listener.",
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
        }),
        json!({
            "name": "listener_disable",
            "title": "Disable Listener",
            "description": "Deactivate a listener without removing its configuration.",
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
        }),
    ]
}
