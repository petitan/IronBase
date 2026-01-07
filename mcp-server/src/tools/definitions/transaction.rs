//! Transaction management tool definitions
//!
//! Tools: begin_transaction, commit_transaction, rollback_transaction,
//!        insert_one_tx, update_one_tx, delete_one_tx, transaction_status

use super::common::{fields, schemas};
use serde_json::{json, Value};

pub fn tools() -> Vec<Value> {
    vec![
        json!({
            "name": "begin_transaction",
            "title": "Begin Transaction",
            "description": "Start an ACID transaction for atomic multi-operation writes.",
            "inputSchema": schemas::empty()
        }),
        json!({
            "name": "commit_transaction",
            "title": "Commit Transaction",
            "description": "Commit all changes made within a transaction atomically.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "transaction_id": fields::transaction_id_from_begin()
                },
                "required": ["transaction_id"]
            }
        }),
        json!({
            "name": "rollback_transaction",
            "title": "Rollback Transaction",
            "description": "Discard all changes made within a transaction.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "transaction_id": fields::transaction_id_from_begin()
                },
                "required": ["transaction_id"]
            }
        }),
        json!({
            "name": "insert_one_tx",
            "title": "Transactional Insert",
            "description": "Insert a document within an active transaction.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "transaction_id": fields::transaction_id(),
                    "collection": {
                        "type": "string",
                        "description": "Target collection"
                    },
                    "document": {
                        "type": "object",
                        "description": "Document to insert"
                    }
                },
                "required": ["transaction_id", "collection", "document"]
            }
        }),
        json!({
            "name": "update_one_tx",
            "title": "Transactional Update",
            "description": "Update a document within an active transaction.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "transaction_id": fields::transaction_id(),
                    "collection": {
                        "type": "string",
                        "description": "Target collection"
                    },
                    "filter": {
                        "type": "object",
                        "description": "Query to match document"
                    },
                    "update": {
                        "type": "object",
                        "description": "Update operations ($set, $inc, etc.)"
                    }
                },
                "required": ["transaction_id", "collection", "filter", "update"]
            }
        }),
        json!({
            "name": "delete_one_tx",
            "title": "Transactional Delete",
            "description": "Delete a document within an active transaction.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "transaction_id": fields::transaction_id(),
                    "collection": {
                        "type": "string",
                        "description": "Target collection"
                    },
                    "filter": {
                        "type": "object",
                        "description": "Query to match document"
                    }
                },
                "required": ["transaction_id", "collection", "filter"]
            }
        }),
        json!({
            "name": "transaction_status",
            "title": "Transaction Status",
            "description": "Check if there is an active write transaction.",
            "inputSchema": schemas::empty()
        }),
    ]
}
