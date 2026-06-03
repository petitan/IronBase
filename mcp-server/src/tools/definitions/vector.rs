//! Vector similarity search tool definition
//!
//! Tool: vector_search. Vector index lifecycle (create/list/drop/stats) is handled
//! by the generic `index_*` tools via `type: "vector"`.

use super::common::fields;
use serde_json::{json, Value};

pub fn tools() -> Vec<Value> {
    vec![json!({
        "name": "vector_search",
        "title": "Vector Similarity Search",
        "description": "Find similar documents using vector embeddings. Returns documents sorted by similarity score.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "collection": {
                    "type": "string",
                    "description": "Collection with vector index"
                },
                "field": {
                    "type": "string",
                    "description": "Field with vector index (default: 'embedding')",
                    "default": "embedding"
                },
                "vector": {
                    "type": "array",
                    "items": { "type": "number" },
                    "description": "Query embedding vector. Dimension must match index."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum results to return",
                    "default": 10
                },
                "projection": fields::projection_simple()
            },
            "required": ["collection", "vector"]
        }
    })]
}
