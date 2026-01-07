//! Schema management tool definitions
//!
//! Tools: schema_set, schema_get

use super::common::{fields, schemas};
use serde_json::{json, Value};

pub fn tools() -> Vec<Value> {
    vec![
        json!({
            "name": "schema_set",
            "title": "Set Collection Schema",
            "description": "Define a JSON Schema for document validation on insert/update.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "collection": fields::collection(),
                    "schema": {
                        "description": "JSON Schema for validation. Use null to remove schema.",
                        "oneOf": [
                            { "type": "object" },
                            { "type": "null" }
                        ]
                    }
                },
                "required": ["collection"]
            }
        }),
        json!({
            "name": "schema_get",
            "title": "Get Collection Schema",
            "description": "Retrieve the JSON Schema defined for a collection.",
            "inputSchema": schemas::collection_only()
        }),
    ]
}
