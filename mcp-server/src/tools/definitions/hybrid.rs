//! Hybrid search tool definitions (RRF-based fusion)
//!
//! Tools: hybrid_search
//!
//! v2 Features (2026-01):
//! - Query preprocessing via pluggable language preprocessors
//! - Reranking: heading boost, phrase match, keyword density
//! - MMR diversity reranking: embedding cosine similarity based

use super::common::fields;
use serde_json::{json, Value};

pub fn tools() -> Vec<Value> {
    vec![json!({
        "name": "hybrid_search",
        "title": "Hybrid Search (RRF)",
        "description": "Combines vector similarity and fulltext search using Reciprocal Rank Fusion (RRF). Includes reranking and MMR diversity reranking (cosine similarity based deduplication). Use mmr_lambda to tune relevance vs diversity. Returns documents sorted by final score.",
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
                "rrf_k": {
                    "type": "number",
                    "description": "RRF K constant. Lower = wider score spread, more reranking impact. Default: 60. Try 20 for better differentiation.",
                    "default": 60,
                    "minimum": 1
                },
                "title_field": {
                    "type": "string",
                    "description": "Optional field containing document title. If set, title match gives up to 1.5x reranking boost."
                },
                "rerank": {
                    "type": "boolean",
                    "description": "Enable reranking after RRF fusion (default: true). Applies exact phrase match (1.5x), keyword density (1.0-1.3x), short content penalty (0.8x), and title match boost (up to 1.5x if title_field set).",
                    "default": true
                },
                "deduplicate": {
                    "type": "boolean",
                    "description": "Enable MMR (Maximal Marginal Relevance) diversity reranking (default: true). Uses embedding cosine similarity to remove near-duplicate results while preserving diversity.",
                    "default": true
                },
                "mmr_lambda": {
                    "type": "number",
                    "description": "MMR lambda: balance between relevance (1.0) and diversity (0.0). Default: 0.5 (balanced).",
                    "default": 0.5,
                    "minimum": 0.0,
                    "maximum": 1.0
                },
                "filter": {
                    "type": "object",
                    "description": "MongoDB-style filter applied BEFORE both vector and fulltext search. Example: {\"doc_type\": \"ajanlat\", \"status\": \"active\"}"
                }
            },
            "required": ["collection", "vector", "query"]
        }
    })]
}
