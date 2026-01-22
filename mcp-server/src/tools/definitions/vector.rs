//! Vector index and similarity search tool definitions
//!
//! Tools: index_create_vector, index_list_vector, index_drop_vector,
//!        vector_search, vector_search_filter

use super::common::fields;
use serde_json::{json, Value};

pub fn tools() -> Vec<Value> {
    vec![
        json!({
            "name": "index_create_vector",
            "title": "Create Vector Index",
            "description": "Create an HNSW vector index for similarity search. Supports Cosine, Euclidean, and DotProduct distance metrics.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "collection": fields::collection(),
                    "field": {
                        "type": "string",
                        "description": "Field containing embedding vectors (array of numbers)"
                    },
                    "dim": {
                        "type": "integer",
                        "description": "Vector dimension (must match your embeddings)",
                        "minimum": 1,
                        "maximum": 4096
                    },
                    "metric": {
                        "type": "string",
                        "description": "Distance metric for similarity calculation",
                        "enum": ["cosine", "euclidean", "dot_product"],
                        "default": "cosine"
                    },
                    "max_vectors": {
                        "type": "integer",
                        "description": "Maximum number of vectors (default: 100,000)",
                        "default": 100000
                    },
                    "m": {
                        "type": "integer",
                        "description": "HNSW M parameter (connections per node, default: 16)",
                        "default": 16,
                        "minimum": 4,
                        "maximum": 64
                    },
                    "ef_construction": {
                        "type": "integer",
                        "description": "HNSW ef_construction (build-time quality, default: 200)",
                        "default": 200,
                        "minimum": 50,
                        "maximum": 1000
                    },
                    "ef_search": {
                        "type": "integer",
                        "description": "HNSW ef_search (search-time quality, default: 50)",
                        "default": 50,
                        "minimum": 10,
                        "maximum": 500
                    }
                },
                "required": ["collection", "field", "dim"]
            }
        }),
        json!({
            "name": "index_list_vector",
            "title": "List Vector Indexes",
            "description": "List all vector indexes on a collection.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "collection": fields::collection()
                },
                "required": ["collection"]
            }
        }),
        json!({
            "name": "index_drop_vector",
            "title": "Drop Vector Index",
            "description": "Remove a vector index from a collection.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "collection": fields::collection(),
                    "index_name": {
                        "type": "string",
                        "description": "Vector index name to drop. Use index_list_vector to see available indexes."
                    }
                },
                "required": ["collection", "index_name"]
            }
        }),
        json!({
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
                        "description": "Field with vector index (default: 'embedding' - gaploader compatible)",
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
        }),
        json!({
            "name": "vector_search_filter",
            "title": "Vector Search with Filter",
            "description": "Hybrid search: combine vector similarity with attribute filtering. Filter is applied first, then vector search on matching documents.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "collection": {
                        "type": "string",
                        "description": "Collection with vector index"
                    },
                    "field": {
                        "type": "string",
                        "description": "Field with vector index (default: 'embedding' - gaploader compatible)",
                        "default": "embedding"
                    },
                    "vector": {
                        "type": "array",
                        "items": { "type": "number" },
                        "description": "Query embedding vector"
                    },
                    "filter": {
                        "type": "object",
                        "description": "MongoDB-style filter applied BEFORE vector search. Example: {\"category\": \"science\"}"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum results to return",
                        "default": 10
                    },
                    "projection": fields::projection_simple()
                },
                "required": ["collection", "vector", "filter"]
            }
        }),
    ]
}
