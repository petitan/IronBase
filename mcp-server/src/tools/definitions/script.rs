//! Script management tool definitions
//!
//! Tools: script_save, script_list, script_get, script_delete, script_run,
//!        script_exec, script_history, script_rollback, script_version_get,
//!        script_tags_add, script_tags_remove, script_stats

use super::common::fields;
use serde_json::{json, Value};

pub fn tools() -> Vec<Value> {
    vec![
        json!({
            "name": "script_save",
            "title": "Save Script",
            "description": "Store a reusable Rhai script in the database with version control.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Unique script identifier"
                    },
                    "code": fields::script_code(),
                    "description": {
                        "type": "string",
                        "description": "Human-readable description of what the script does"
                    },
                    "tags": fields::script_tags(),
                    "dependencies": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Names of other scripts this script depends on"
                    }
                },
                "required": ["name", "code"]
            }
        }),
        json!({
            "name": "script_list",
            "title": "List Scripts",
            "description": "List all saved scripts with optional tag filtering.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "tags": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Filter scripts by tags"
                    },
                    "match_all": {
                        "type": "boolean",
                        "description": "true = match ALL tags (AND), false = match ANY tag (OR)",
                        "default": false
                    }
                },
                "required": []
            }
        }),
        json!({
            "name": "script_get",
            "title": "Get Script",
            "description": "Retrieve a saved script's code and metadata.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": fields::script_name()
                },
                "required": ["name"]
            }
        }),
        json!({
            "name": "script_delete",
            "title": "Delete Script",
            "description": "Permanently remove a script from the database.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Script name to delete"
                    }
                },
                "required": ["name"]
            }
        }),
        json!({
            "name": "script_run",
            "title": "Run Saved Script",
            "description": "Execute a previously saved script by name.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Script name to execute"
                    },
                    "params": fields::script_params(),
                    "max_operations": fields::max_operations()
                },
                "required": ["name"]
            }
        }),
        json!({
            "name": "script_exec",
            "title": "Execute Inline Script",
            "description": "Execute Rhai code directly without saving.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "code": {
                        "type": "string",
                        "description": "Rhai code. Database API: db_find(coll, query, opts?), db_find_one(coll, query), db_count(coll, query), db_aggregate(coll, pipeline), db_insert_one(coll, doc), db_update_one(coll, filter, update), db_delete_one(coll, filter), db_fulltext_search(coll, field, query, opts), db_vector_search(coll, field, vector, limit), db_vector_search_filter(coll, field, vector, filter, limit)"
                    },
                    "params": {
                        "type": "object",
                        "description": "Parameters accessible as 'params' variable in script"
                    },
                    "max_operations": fields::max_operations()
                },
                "required": ["code"]
            }
        }),
        json!({
            "name": "script_history",
            "title": "Script Version History",
            "description": "List all versions of a script for auditing or rollback.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": fields::script_name(),
                    "limit": fields::limit(None)
                },
                "required": ["name"]
            }
        }),
        json!({
            "name": "script_rollback",
            "title": "Rollback Script",
            "description": "Restore a script to a previous version.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": fields::script_name(),
                    "version": {
                        "type": "integer",
                        "description": "Version number to restore"
                    }
                },
                "required": ["name", "version"]
            }
        }),
        json!({
            "name": "script_version_get",
            "title": "Get Script Version",
            "description": "Retrieve a specific historical version of a script.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": fields::script_name(),
                    "version": {
                        "type": "integer",
                        "description": "Version number to retrieve"
                    }
                },
                "required": ["name", "version"]
            }
        }),
        json!({
            "name": "script_tags_add",
            "title": "Add Script Tags",
            "description": "Add categorization tags to a script.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": fields::script_name(),
                    "tags": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Tags to add"
                    }
                },
                "required": ["name", "tags"]
            }
        }),
        json!({
            "name": "script_tags_remove",
            "title": "Remove Script Tags",
            "description": "Remove categorization tags from a script.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": fields::script_name(),
                    "tags": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Tags to remove"
                    }
                },
                "required": ["name", "tags"]
            }
        }),
        json!({
            "name": "script_stats",
            "title": "Script Statistics",
            "description": "Get execution statistics including run count and timing.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": fields::script_name()
                },
                "required": ["name"]
            }
        }),
    ]
}
