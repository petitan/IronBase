//! Database management tool definitions
//!
//! Tools: db_open, db_stats, db_compact, db_checkpoint

use super::common::schemas;
use serde_json::{json, Value};

pub fn tools() -> Vec<Value> {
    vec![
        json!({
            "name": "db_open",
            "title": "Open Database",
            "description": "Open an existing database file or create a new one.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to database file (.mlite extension)"
                    },
                    "create": {
                        "type": "boolean",
                        "description": "true = create new (fails if exists), false = open existing (fails if missing)",
                        "default": false
                    }
                },
                "required": ["path"]
            }
        }),
        json!({
            "name": "db_stats",
            "title": "Database Statistics",
            "description": "Get database metrics including file size, collection count, and document counts.",
            "inputSchema": schemas::empty()
        }),
        json!({
            "name": "db_compact",
            "title": "Compact Database",
            "description": "Reclaim disk space by removing tombstones. Runs in background — returns a job_id. Use embed_job_status to check progress. Non-blocking: reads and writes continue during compaction.",
            "inputSchema": schemas::empty()
        }),
        json!({
            "name": "db_checkpoint",
            "title": "Force Checkpoint",
            "description": "Flush all pending writes to disk immediately.",
            "inputSchema": schemas::empty()
        }),
    ]
}
