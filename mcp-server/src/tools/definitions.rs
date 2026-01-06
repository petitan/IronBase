//! Tool JSON definitions for MCP tools/list
//!
//! Contains the JSON schema definitions for all available tools.
//! This is separated from mod.rs to keep the dispatch logic readable.

use serde_json::{json, Value};

pub fn get_all_tools_json() -> Value {
    json!({
        "tools": [
            // Database Management
            {
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
            },
            {
                "name": "db_stats",
                "title": "Database Statistics",
                "description": "Get database metrics including file size, collection count, and document counts.",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            },
            {
                "name": "db_compact",
                "title": "Compact Database",
                "description": "Reclaim disk space by removing tombstones from deleted documents.",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            },
            {
                "name": "db_checkpoint",
                "title": "Force Checkpoint",
                "description": "Flush all pending writes to disk immediately.",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            },
            // Collection Management
            {
                "name": "collection_list",
                "title": "List Collections",
                "description": "List all user collections in the database.",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            },
            {
                "name": "collection_create",
                "title": "Create Collection",
                "description": "Create an empty collection. Collections are also auto-created on first insert.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "Collection name. Alphanumeric with underscores allowed."
                        }
                    },
                    "required": ["name"]
                }
            },
            {
                "name": "collection_drop",
                "title": "Drop Collection",
                "description": "Delete a collection and all its documents, indexes, and schema.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "Collection name to delete"
                        }
                    },
                    "required": ["name"]
                }
            },
            // Document CRUD
            {
                "name": "insert_one",
                "title": "Insert Document",
                "description": "Insert a single document into a collection. Returns the inserted document ID.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": { "type": "string", "description": "Target collection name" },
                        "document": { "type": "object", "description": "JSON document to insert. Auto-generates _id if not provided." }
                    },
                    "required": ["collection", "document"]
                }
            },
            {
                "name": "insert_many",
                "title": "Bulk Insert Documents",
                "description": "Insert multiple documents into a collection in a single operation.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": { "type": "string", "description": "Target collection name" },
                        "documents": { "type": "array", "items": { "type": "object" }, "description": "Array of JSON documents to insert" }
                    },
                    "required": ["collection", "documents"]
                }
            },
            {
                "name": "find",
                "title": "Query Documents",
                "description": "Query documents matching a filter with optional projection, sorting, and pagination.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": { "type": "string", "description": "Collection to query" },
                        "query": { "type": "object", "description": "MongoDB-style filter. Operators: $eq, $ne, $gt, $gte, $lt, $lte, $in, $nin, $and, $or, $not, $regex, $exists. Example: {\"age\": {\"$gte\": 18}}" },
                        "projection": { "type": "object", "description": "Fields to include (1) or exclude (0). Example: {\"name\": 1, \"email\": 1} or {\"password\": 0}" },
                        "sort": { "description": "Sort specification. Array format: [[\"field\", 1]] for ascending, [[\"field\", -1]] for descending. Object format: {\"field\": 1}",
                            "oneOf": [
                                { "type": "array" },
                                { "type": "object" }
                            ]
                        },
                        "limit": { "type": "integer", "description": "Maximum documents to return. Default: 10,000. Use with large collections to prevent timeout." },
                        "skip": { "type": "integer", "description": "Number of documents to skip for pagination" },
                        "include_total": { "type": "boolean", "description": "Include total matching count in response for pagination UI", "default": false }
                    },
                    "required": ["collection"]
                }
            },
            {
                "name": "find_one",
                "title": "Find Single Document",
                "description": "Retrieve the first document matching a query.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": { "type": "string", "description": "Collection to query" },
                        "query": { "type": "object", "description": "MongoDB-style filter. Example: {\"_id\": 123} or {\"email\": \"user@example.com\"}" },
                        "projection": { "type": "object", "description": "Fields to include (1) or exclude (0)" }
                    },
                    "required": ["collection"]
                }
            },
            {
                "name": "update_one",
                "title": "Update Single Document",
                "description": "Update the first document matching a filter.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": { "type": "string", "description": "Collection containing the document" },
                        "filter": { "type": "object", "description": "Query filter to match the document. Example: {\"_id\": 123}" },
                        "update": { "type": "object", "description": "Update operations. Operators: $set, $inc, $unset, $push, $pull, $addToSet, $pop. Example: {\"$set\": {\"status\": \"active\"}, \"$inc\": {\"views\": 1}}" }
                    },
                    "required": ["collection", "filter", "update"]
                }
            },
            {
                "name": "update_many",
                "title": "Bulk Update Documents",
                "description": "Update all documents matching a filter.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": { "type": "string", "description": "Collection containing the documents" },
                        "filter": { "type": "object", "description": "Query filter. Example: {\"status\": \"pending\"}" },
                        "update": { "type": "object", "description": "Update operations to apply to all matches. Example: {\"$set\": {\"status\": \"processed\"}}" }
                    },
                    "required": ["collection", "filter", "update"]
                }
            },
            {
                "name": "delete_one",
                "title": "Delete Single Document",
                "description": "Delete the first document matching a filter.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": { "type": "string", "description": "Collection containing the document" },
                        "filter": { "type": "object", "description": "Query filter. Example: {\"_id\": 123}" }
                    },
                    "required": ["collection", "filter"]
                }
            },
            {
                "name": "delete_many",
                "title": "Bulk Delete Documents",
                "description": "Delete all documents matching a filter.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": { "type": "string", "description": "Collection containing the documents" },
                        "filter": { "type": "object", "description": "Query filter. Use {} to delete all documents (with caution)." }
                    },
                    "required": ["collection", "filter"]
                }
            },
            {
                "name": "count_documents",
                "title": "Count Documents",
                "description": "Count documents matching a query filter.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": { "type": "string", "description": "Collection to count" },
                        "query": { "type": "object", "description": "MongoDB-style filter. Use $gte/$lt for indexed range queries. Example: {\"date\": {\"$gte\": \"2024-01-01\", \"$lt\": \"2025-01-01\"}}" }
                    },
                    "required": ["collection"]
                }
            },
            {
                "name": "distinct",
                "title": "Get Distinct Values",
                "description": "Retrieve unique values for a specified field across documents.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": { "type": "string", "description": "Collection to query" },
                        "field": { "type": "string", "description": "Field path to get distinct values. Supports dot notation: \"address.city\"" },
                        "query": { "type": "object", "description": "Optional filter to limit which documents are considered" }
                    },
                    "required": ["collection", "field"]
                }
            },
            {
                "name": "aggregate",
                "title": "Aggregation Pipeline",
                "description": "Execute a multi-stage data processing pipeline for analytics and transformations.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": { "type": "string", "description": "Source collection" },
                        "pipeline": { "type": "array", "description": "Array of pipeline stages. Stages: $match (filter), $group (aggregate), $project (reshape), $sort, $limit, $skip, $unwind, $count. Example: [{\"$match\": {\"status\": \"active\"}}, {\"$group\": {\"_id\": \"$category\", \"total\": {\"$sum\": 1}}}, {\"$sort\": {\"total\": -1}}, {\"$limit\": 10}]" }
                    },
                    "required": ["collection", "pipeline"]
                }
            },
            // Index Management
            {
                "name": "index_create",
                "title": "Create B+ Tree Index",
                "description": "Create a B+ tree index on one or more fields to accelerate queries.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": { "type": "string", "description": "Collection to index" },
                        "field": { "type": "string", "description": "Single field to index. Use dot notation for nested: \"address.city\"" },
                        "fields": { "type": "array", "items": { "type": "string" }, "minItems": 1, "description": "Multiple fields for compound index. Order matters for query optimization." },
                        "unique": { "type": "boolean", "description": "Enforce unique values. Rejects duplicate inserts.", "default": false },
                        "sparse": { "type": "boolean", "description": "Only index documents where field exists. Optimizes $exists:true queries.", "default": false }
                    },
                    "required": ["collection"],
                    "anyOf": [
                        { "required": ["field"] },
                        { "required": ["fields"] }
                    ]
                }
            },
            {
                "name": "index_list",
                "title": "List Indexes",
                "description": "List all indexes defined on a collection.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": { "type": "string", "description": "Collection name" }
                    },
                    "required": ["collection"]
                }
            },
            {
                "name": "index_create_fuzzy",
                "title": "Create Fuzzy Search Index",
                "description": "Create an index for approximate string matching using similarity algorithms.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": { "type": "string", "description": "Collection to index" },
                        "field": { "type": "string", "description": "Text field to index for fuzzy matching" },
                        "algorithm": { "type": "string", "description": "Similarity algorithm", "enum": ["jaro_winkler", "levenshtein", "damerau_levenshtein"], "default": "jaro_winkler" },
                        "threshold": { "type": "number", "description": "Minimum similarity score (0.0-1.0). Higher = stricter matching.", "default": 0.8, "minimum": 0, "maximum": 1 }
                    },
                    "required": ["collection", "field"]
                }
            },
            {
                "name": "index_create_fulltext",
                "title": "Create Full-Text Search Index",
                "description": "Create a TF-IDF full-text search index with language-aware stemming.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": { "type": "string", "description": "Collection to index" },
                        "field": { "type": "string", "description": "Text field to index. Supports nested paths: \"body.content\"" },
                        "language": { "type": "string", "description": "Language for stemming and stop words", "enum": ["hungarian", "english", "german", "none"], "default": "none" },
                        "min_word_length": { "type": "integer", "description": "Minimum word length to index", "default": 2, "minimum": 1 },
                        "accent_folding": { "type": "boolean", "description": "Normalize accented characters (á→a, ő→o)", "default": true }
                    },
                    "required": ["collection", "field"]
                }
            },
            {
                "name": "fulltext_search",
                "title": "Full-Text Search",
                "description": "Search documents using TF-IDF relevance scoring. Requires index_create_fulltext first.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": { "type": "string", "description": "Collection with fulltext index" },
                        "field": { "type": "string", "description": "Indexed field to search" },
                        "query": { "type": "string", "description": "Search terms. Multiple words are OR-ed. Example: \"database query optimization\"" },
                        "limit": { "type": "integer", "description": "Maximum results to return", "default": 10 },
                        "skip": { "type": "integer", "description": "Results to skip for pagination", "default": 0 },
                        "min_score": { "type": "number", "description": "Minimum TF-IDF relevance score threshold" },
                        "projection": { "type": "object", "description": "Fields to include/exclude in results" }
                    },
                    "required": ["collection", "field", "query"]
                }
            },
            {
                "name": "fuzzy_search",
                "title": "Fuzzy String Search",
                "description": "Find documents with approximate string matching. Requires index_create_fuzzy first.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": { "type": "string", "description": "Collection with fuzzy index" },
                        "field": { "type": "string", "description": "Indexed field to search" },
                        "query": { "type": "string", "description": "Search term. Finds similar strings: \"john\" matches \"Jon\", \"Johan\"" },
                        "algorithm": { "type": "string", "description": "Override index algorithm for this search", "enum": ["jaro_winkler", "levenshtein", "damerau_levenshtein"] },
                        "threshold": { "type": "number", "description": "Override similarity threshold (0.0-1.0)", "minimum": 0, "maximum": 1 },
                        "limit": { "type": "integer", "description": "Maximum results to return" },
                        "projection": { "type": "object", "description": "Fields to include/exclude in results" }
                    },
                    "required": ["collection", "field", "query"]
                }
            },
            {
                "name": "index_list_fulltext",
                "title": "List Full-Text Indexes",
                "description": "List all full-text search indexes on a collection.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": { "type": "string", "description": "Collection name" }
                    },
                    "required": ["collection"]
                }
            },
            {
                "name": "index_drop",
                "title": "Drop Index",
                "description": "Remove an index from a collection.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": { "type": "string", "description": "Collection name" },
                        "index_name": { "type": "string", "description": "Index name to drop. Use index_list to see available indexes." }
                    },
                    "required": ["collection", "index_name"]
                }
            },
            {
                "name": "explain",
                "title": "Explain Query Plan",
                "description": "Analyze query execution plan showing which indexes will be used.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": { "type": "string", "description": "Collection to analyze" },
                        "query": { "type": "object", "description": "Query filter to analyze. Shows: scan type, index used, estimated cost." }
                    },
                    "required": ["collection"]
                }
            },
            {
                "name": "find_with_hint",
                "title": "Query with Index Hint",
                "description": "Execute a query forcing a specific index. Useful for testing index performance.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": { "type": "string", "description": "Collection to query" },
                        "query": { "type": "object", "description": "MongoDB-style filter" },
                        "hint": { "type": "string", "description": "Index name to force. Use index_list to see available indexes." },
                        "projection": { "type": "object", "description": "Fields to include/exclude" },
                        "sort": { "description": "Sort specification",
                            "oneOf": [
                                { "type": "array" },
                                { "type": "object" }
                            ]
                        },
                        "limit": { "type": "integer", "description": "Maximum documents to return", "default": 10000 },
                        "skip": { "type": "integer", "description": "Documents to skip" }
                    },
                    "required": ["collection", "hint"]
                }
            },
            // Schema Management
            {
                "name": "schema_set",
                "title": "Set Collection Schema",
                "description": "Define a JSON Schema for document validation on insert/update.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": { "type": "string", "description": "Collection name" },
                        "schema": { "description": "JSON Schema for validation. Use null to remove schema.",
                            "oneOf": [
                                { "type": "object" },
                                { "type": "null" }
                            ]
                        }
                    },
                    "required": ["collection"]
                }
            },
            {
                "name": "schema_get",
                "title": "Get Collection Schema",
                "description": "Retrieve the JSON Schema defined for a collection.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": { "type": "string", "description": "Collection name" }
                    },
                    "required": ["collection"]
                }
            },
            // Script Management
            {
                "name": "script_save",
                "title": "Save Script",
                "description": "Store a reusable Rhai script in the database with version control.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "Unique script identifier" },
                        "code": { "type": "string", "description": "Rhai script source code" },
                        "description": { "type": "string", "description": "Human-readable description of what the script does" },
                        "tags": { "type": "array", "items": { "type": "string" }, "description": "Tags for categorization and filtering" },
                        "dependencies": { "type": "array", "items": { "type": "string" }, "description": "Names of other scripts this script depends on" }
                    },
                    "required": ["name", "code"]
                }
            },
            {
                "name": "script_list",
                "title": "List Scripts",
                "description": "List all saved scripts with optional tag filtering.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "tags": { "type": "array", "items": { "type": "string" }, "description": "Filter scripts by tags" },
                        "match_all": { "type": "boolean", "description": "true = match ALL tags (AND), false = match ANY tag (OR)", "default": false }
                    },
                    "required": []
                }
            },
            {
                "name": "script_get",
                "title": "Get Script",
                "description": "Retrieve a saved script's code and metadata.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "Script name" }
                    },
                    "required": ["name"]
                }
            },
            {
                "name": "script_delete",
                "title": "Delete Script",
                "description": "Permanently remove a script from the database.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "Script name to delete" }
                    },
                    "required": ["name"]
                }
            },
            {
                "name": "script_run",
                "title": "Run Saved Script",
                "description": "Execute a previously saved script by name.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "Script name to execute" },
                        "params": { "type": "object", "description": "Parameters passed to script as 'params' variable" },
                        "max_operations": { "type": "integer", "description": "Maximum Rhai operations before timeout", "default": 1000000 }
                    },
                    "required": ["name"]
                }
            },
            {
                "name": "script_exec",
                "title": "Execute Inline Script",
                "description": "Execute Rhai code directly without saving.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "code": { "type": "string", "description": "Rhai code. Database API: db_find(coll, query, opts?), db_find_one(coll, query), db_count(coll, query), db_aggregate(coll, pipeline), db_insert_one(coll, doc), db_update_one(coll, filter, update), db_delete_one(coll, filter), db_fulltext_search(coll, field, query, opts)" },
                        "params": { "type": "object", "description": "Parameters accessible as 'params' variable in script" },
                        "max_operations": { "type": "integer", "description": "Maximum operations before timeout", "default": 1000000 }
                    },
                    "required": ["code"]
                }
            },
            {
                "name": "script_history",
                "title": "Script Version History",
                "description": "List all versions of a script for auditing or rollback.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "Script name" },
                        "limit": { "type": "integer", "description": "Maximum versions to return" }
                    },
                    "required": ["name"]
                }
            },
            {
                "name": "script_rollback",
                "title": "Rollback Script",
                "description": "Restore a script to a previous version.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "Script name" },
                        "version": { "type": "integer", "description": "Version number to restore" }
                    },
                    "required": ["name", "version"]
                }
            },
            {
                "name": "script_version_get",
                "title": "Get Script Version",
                "description": "Retrieve a specific historical version of a script.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "Script name" },
                        "version": { "type": "integer", "description": "Version number to retrieve" }
                    },
                    "required": ["name", "version"]
                }
            },
            {
                "name": "script_tags_add",
                "title": "Add Script Tags",
                "description": "Add categorization tags to a script.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "Script name" },
                        "tags": { "type": "array", "items": { "type": "string" }, "description": "Tags to add" }
                    },
                    "required": ["name", "tags"]
                }
            },
            {
                "name": "script_tags_remove",
                "title": "Remove Script Tags",
                "description": "Remove categorization tags from a script.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "Script name" },
                        "tags": { "type": "array", "items": { "type": "string" }, "description": "Tags to remove" }
                    },
                    "required": ["name", "tags"]
                }
            },
            {
                "name": "script_stats",
                "title": "Script Statistics",
                "description": "Get execution statistics including run count and timing.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "Script name" }
                    },
                    "required": ["name"]
                }
            },
            // Transaction Management
            {
                "name": "begin_transaction",
                "title": "Begin Transaction",
                "description": "Start an ACID transaction for atomic multi-operation writes.",
                "inputSchema": { "type": "object", "properties": {}, "required": [] }
            },
            {
                "name": "commit_transaction",
                "title": "Commit Transaction",
                "description": "Commit all changes made within a transaction atomically.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "transaction_id": { "type": "string", "description": "Transaction ID from begin_transaction" }
                    },
                    "required": ["transaction_id"]
                }
            },
            {
                "name": "rollback_transaction",
                "title": "Rollback Transaction",
                "description": "Discard all changes made within a transaction.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "transaction_id": { "type": "string", "description": "Transaction ID from begin_transaction" }
                    },
                    "required": ["transaction_id"]
                }
            },
            {
                "name": "insert_one_tx",
                "title": "Transactional Insert",
                "description": "Insert a document within an active transaction.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "transaction_id": { "type": "string", "description": "Active transaction ID" },
                        "collection": { "type": "string", "description": "Target collection" },
                        "document": { "type": "object", "description": "Document to insert" }
                    },
                    "required": ["transaction_id", "collection", "document"]
                }
            },
            {
                "name": "update_one_tx",
                "title": "Transactional Update",
                "description": "Update a document within an active transaction.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "transaction_id": { "type": "string", "description": "Active transaction ID" },
                        "collection": { "type": "string", "description": "Target collection" },
                        "filter": { "type": "object", "description": "Query to match document" },
                        "update": { "type": "object", "description": "Update operations ($set, $inc, etc.)" }
                    },
                    "required": ["transaction_id", "collection", "filter", "update"]
                }
            },
            {
                "name": "delete_one_tx",
                "title": "Transactional Delete",
                "description": "Delete a document within an active transaction.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "transaction_id": { "type": "string", "description": "Active transaction ID" },
                        "collection": { "type": "string", "description": "Target collection" },
                        "filter": { "type": "object", "description": "Query to match document" }
                    },
                    "required": ["transaction_id", "collection", "filter"]
                }
            },
            {
                "name": "transaction_status",
                "title": "Transaction Status",
                "description": "Check if there is an active write transaction.",
                "inputSchema": { "type": "object", "properties": {}, "required": [] }
            },
            // Admin Operations (require IRONBASE_ADMIN_KEY)
            {
                "name": "admin_list_all_collections",
                "title": "List All Collections (Admin)",
                "description": "List all collections including system and hidden ones.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "admin_key": { "type": "string", "description": "Admin authentication key (IRONBASE_ADMIN_KEY)" }
                    },
                    "required": ["admin_key"]
                }
            },
            {
                "name": "admin_create_system_collection",
                "title": "Create System Collection (Admin)",
                "description": "Create a protected system collection with special flags.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "admin_key": { "type": "string", "description": "Admin authentication key" },
                        "name": { "type": "string", "description": "Collection name (conventionally prefixed with _system.)" }
                    },
                    "required": ["admin_key", "name"]
                }
            },
            {
                "name": "admin_set_collection_flags",
                "title": "Set Collection Flags (Admin)",
                "description": "Modify collection protection and visibility flags.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "admin_key": { "type": "string", "description": "Admin authentication key" },
                        "collection": { "type": "string", "description": "Collection name" },
                        "is_system": { "type": "boolean", "description": "Mark as system collection (restricts modifications)" },
                        "protected": { "type": "boolean", "description": "Prevent accidental deletion" },
                        "hidden": { "type": "boolean", "description": "Hide from collection_list results" }
                    },
                    "required": ["admin_key", "collection"]
                }
            },
            {
                "name": "admin_drop_protected",
                "title": "Drop Protected Collection (Admin)",
                "description": "Force-delete a protected collection.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "admin_key": { "type": "string", "description": "Admin authentication key" },
                        "name": { "type": "string", "description": "Collection name to delete" }
                    },
                    "required": ["admin_key", "name"]
                }
            },
            {
                "name": "admin_apikey_create",
                "title": "Create API Key (Admin)",
                "description": "Generate a new API key for client authentication.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "admin_key": { "type": "string", "description": "Admin authentication key" },
                        "name": { "type": "string", "description": "Descriptive name for the API key" }
                    },
                    "required": ["admin_key", "name"]
                }
            },
            {
                "name": "admin_apikey_list",
                "title": "List API Keys (Admin)",
                "description": "List all API keys with masked values.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "admin_key": { "type": "string", "description": "Admin authentication key" }
                    },
                    "required": ["admin_key"]
                }
            },
            {
                "name": "admin_apikey_revoke",
                "title": "Revoke API Key (Admin)",
                "description": "Disable an API key without deleting it.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "admin_key": { "type": "string", "description": "Admin authentication key" },
                        "id": { "type": "integer", "description": "API key ID to revoke" }
                    },
                    "required": ["admin_key", "id"]
                }
            },
            {
                "name": "admin_apikey_delete",
                "title": "Delete API Key (Admin)",
                "description": "Permanently remove an API key.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "admin_key": { "type": "string", "description": "Admin authentication key" },
                        "id": { "type": "integer", "description": "API key ID to delete" }
                    },
                    "required": ["admin_key", "id"]
                }
            },
            // Access Control List (ACL) Management
            {
                "name": "acl_list",
                "title": "List ACL Rules",
                "description": "List all access control rules across collections.",
                "inputSchema": { "type": "object", "properties": {}, "required": [] }
            },
            {
                "name": "acl_get",
                "title": "Get Collection ACL",
                "description": "Get access control rules for a specific collection.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": { "type": "string", "description": "Collection name" }
                    },
                    "required": ["collection"]
                }
            },
            {
                "name": "acl_set",
                "title": "Set Collection ACL",
                "description": "Define access control rules for a collection.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": { "type": "string", "description": "Collection name" },
                        "rules": { "type": "array", "description": "Array of ACL rule objects defining permissions" }
                    },
                    "required": ["collection", "rules"]
                }
            },
            {
                "name": "acl_delete",
                "title": "Delete Collection ACL",
                "description": "Remove custom ACL rules, reverting to default permissions.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "collection": { "type": "string", "description": "Collection name" }
                    },
                    "required": ["collection"]
                }
            },
            {
                "name": "acl_cleanup",
                "title": "Cleanup Orphan ACLs",
                "description": "Remove ACL rules for collections that no longer exist.",
                "inputSchema": { "type": "object", "properties": {}, "required": [] }
            },
            // HTTP/HTTPS Listener Management
            {
                "name": "listener_list",
                "title": "List Listeners",
                "description": "List all configured HTTP/HTTPS listeners.",
                "inputSchema": { "type": "object", "properties": {}, "required": [] }
            },
            {
                "name": "listener_get",
                "title": "Get Listener Config",
                "description": "Get configuration details for a specific listener.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "description": "Listener identifier" }
                    },
                    "required": ["id"]
                }
            },
            {
                "name": "listener_add",
                "title": "Add Listener",
                "description": "Configure a new HTTP or HTTPS listener endpoint.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "description": "Unique listener identifier" },
                        "bind": { "type": "string", "description": "Network address to bind. Example: \"0.0.0.0:8080\" or \"127.0.0.1:8443\"" },
                        "tls": { "type": "boolean", "description": "Enable TLS/HTTPS", "default": false },
                        "cert_path": { "type": "string", "description": "Path to TLS certificate file (PEM format)" },
                        "key_path": { "type": "string", "description": "Path to TLS private key file (PEM format)" },
                        "description": { "type": "string", "description": "Human-readable description" }
                    },
                    "required": ["id", "bind"]
                }
            },
            {
                "name": "listener_delete",
                "title": "Delete Listener",
                "description": "Remove a listener configuration.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "description": "Listener ID to delete" }
                    },
                    "required": ["id"]
                }
            },
            {
                "name": "listener_enable",
                "title": "Enable Listener",
                "description": "Activate a disabled listener.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "description": "Listener ID to enable" }
                    },
                    "required": ["id"]
                }
            },
            {
                "name": "listener_disable",
                "title": "Disable Listener",
                "description": "Deactivate a listener without removing its configuration.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "description": "Listener ID to disable" }
                    },
                    "required": ["id"]
                }
            }
        ]
    })
}
