//! ACL (Access Control List) management tool definitions
//!
//! Tools: acl_list, acl_get, acl_set, acl_delete, acl_cleanup

use super::common::{fields, schemas};
use serde_json::{json, Value};

pub fn tools() -> Vec<Value> {
    vec![
        json!({
            "name": "acl_list",
            "title": "List ACL Rules",
            "description": "List all access control rules across collections.",
            "inputSchema": schemas::empty()
        }),
        json!({
            "name": "acl_get",
            "title": "Get Collection ACL",
            "description": "Get access control rules for a specific collection.",
            "inputSchema": schemas::collection_only()
        }),
        json!({
            "name": "acl_set",
            "title": "Set Collection ACL",
            "description": "Define access control rules for a collection.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "collection": fields::collection(),
                    "rules": {
                        "type": "array",
                        "description": "Array of ACL rule objects defining permissions"
                    }
                },
                "required": ["collection", "rules"]
            }
        }),
        json!({
            "name": "acl_delete",
            "title": "Delete Collection ACL",
            "description": "Remove custom ACL rules, reverting to default permissions.",
            "inputSchema": schemas::collection_only()
        }),
        json!({
            "name": "acl_cleanup",
            "title": "Cleanup Orphan ACLs",
            "description": "Remove ACL rules for collections that no longer exist.",
            "inputSchema": schemas::empty()
        }),
    ]
}
