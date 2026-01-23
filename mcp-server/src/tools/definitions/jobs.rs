//! Job management tool definitions
//!
//! Tools: embed_job_status, embed_job_list, embed_job_cancel

use serde_json::{json, Value};

pub fn tools() -> Vec<Value> {
    vec![
        json!({
            "name": "embed_job_status",
            "title": "Get Job Status",
            "description": "Get the status of a background job by its ID.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "job_id": {
                        "type": "string",
                        "description": "The job ID to check"
                    }
                },
                "required": ["job_id"]
            }
        }),
        json!({
            "name": "embed_job_list",
            "title": "List Jobs",
            "description": "List all background jobs (active and recent completed).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "active_only": {
                        "type": "boolean",
                        "description": "Only show active (non-completed) jobs",
                        "default": false
                    }
                }
            }
        }),
        json!({
            "name": "embed_job_cancel",
            "title": "Cancel Job",
            "description": "Cancel a running background job.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "job_id": {
                        "type": "string",
                        "description": "The job ID to cancel"
                    }
                },
                "required": ["job_id"]
            }
        }),
    ]
}
