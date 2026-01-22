//! Hybrid search tool definitions (RRF-based fusion)
//!
//! Tools: hybrid_search
//!
//! v2 Features (2026-01):
//! - Query preprocessing via pluggable language preprocessors
//! - Reranking: heading boost, phrase match, keyword density
//! - Deduplication: content prefix and heading based

use super::common::fields;
use serde_json::{json, Value};

pub fn tools() -> Vec<Value> {
    vec![json!({
        "name": "hybrid_search",
        "title": "Hybrid Search (RRF)",
        "description": "Combines vector similarity and fulltext search using Reciprocal Rank Fusion (RRF). Includes query preprocessing, reranking, and deduplication. Returns documents sorted by final score.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "collection": fields::collection(),
                "vector_field": {
                    "type": "string",
                    "description": "Field with vector index (default: 'embedding' - gaploader compatible)",
                    "default": "embedding"
                },
                "text_field": {
                    "type": "string",
                    "description": "Field with fulltext index (default: 'content' - gaploader compatible)",
                    "default": "content"
                },
                "vector": {
                    "type": "array",
                    "items": { "type": "number" },
                    "description": "Query embedding vector. Dimension must match index."
                },
                "query": {
                    "type": "string",
                    "description": "Text query for fulltext search"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum results to return (default: 10)",
                    "default": 10
                },
                "vector_weight": {
                    "type": "number",
                    "description": "Weight for vector search results (default: 0.5)",
                    "default": 0.5,
                    "minimum": 0.0,
                    "maximum": 1.0
                },
                "fulltext_weight": {
                    "type": "number",
                    "description": "Weight for fulltext search results (default: 0.5)",
                    "default": 0.5,
                    "minimum": 0.0,
                    "maximum": 1.0
                },
                "projection": fields::projection_simple(),

                // ========== v2 parameters ==========
                "language": {
                    "type": "string",
                    "description": "Query preprocessing language. Removes stop words, question words, and strips suffixes. Available: 'hungarian'. If not specified, no preprocessing is applied.",
                    "enum": ["hungarian"]
                },
                "rerank": {
                    "type": "boolean",
                    "description": "Enable reranking after RRF fusion (default: true). Applies exact phrase match (1.3x), keyword density (1.0-1.1x), and short content penalty (0.8x). Uses only text_field, no extra fields needed.",
                    "default": true
                },
                "deduplicate": {
                    "type": "boolean",
                    "description": "Enable deduplication (default: true). Removes results with duplicate content prefixes (based on text_field).",
                    "default": true
                },
                "dedup_threshold": {
                    "type": "integer",
                    "description": "Content prefix length for deduplication (default: 100). Results with matching first N characters are considered duplicates.",
                    "default": 100,
                    "minimum": 10,
                    "maximum": 1000
                }
            },
            "required": ["collection", "vector", "query"]
        }
    })]
}
