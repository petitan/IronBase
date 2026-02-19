//! Hybrid search tool definitions (RRF-based fusion)
//!
//! Tools: hybrid_search
//!
//! Unified tool for both explicit vector and auto-embed modes.
//! If 'vector' is omitted, the query text is automatically embedded
//! using the collection's RAG config or the specified 'provider'.
//!
//! Features:
//! - Reranking: phrase match, keyword density, title boost
//! - MMR diversity reranking: embedding cosine similarity based

use super::common::fields;
use serde_json::{json, Value};

pub fn tools() -> Vec<Value> {
    vec![json!({
        "name": "hybrid_search",
        "title": "Hybrid Search (RRF)",
        "description": "Combines vector similarity and fulltext search using Reciprocal Rank Fusion (RRF). If 'vector' is omitted, the query text is automatically embedded using the collection's configured provider (auto-embed mode). Includes reranking and MMR diversity reranking. Use mmr_lambda to tune relevance vs diversity.",
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
                    "description": "Query embedding vector. If omitted, the query text is auto-embedded using the collection's configured provider or the 'provider' parameter."
                },
                "query": {
                    "type": "string",
                    "description": "Text query for fulltext search (and auto-embedding if vector is omitted)"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum results to return (default: 10)",
                    "default": 10
                },
                "search_mode": {
                    "type": "string",
                    "description": "Search mode preset: 'balanced' (default, equal weights), 'semantic' (favor vector similarity for conceptual queries), 'keyword' (favor fulltext for specific term queries). Overridden by explicit vector_weight/fulltext_weight.",
                    "enum": ["balanced", "semantic", "keyword"],
                    "default": "balanced"
                },
                "vector_weight": {
                    "type": "number",
                    "description": "Explicit vector weight override (overrides search_mode). Default: 0.5",
                    "minimum": 0.0,
                    "maximum": 1.0
                },
                "fulltext_weight": {
                    "type": "number",
                    "description": "Explicit fulltext weight override (overrides search_mode). Default: 0.5",
                    "minimum": 0.0,
                    "maximum": 1.0
                },
                "projection": fields::projection_simple(),
                "provider": {
                    "type": "string",
                    "description": "Embedding provider for auto-embed mode (uses collection RAG config if not specified). Example: 'fasttext'"
                },
                "rrf_k": {
                    "type": "number",
                    "description": "RRF K constant. Lower = wider score spread, more reranking impact. Default: 20. Use 60 for classic RRF behavior.",
                    "default": 20,
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
                },
                "mode": {
                    "type": "string",
                    "description": "Fulltext search mode: 'or' (default) = any word matches, 'and' = ALL words must match in document",
                    "enum": ["or", "and"],
                    "default": "or"
                },
                "text_fields": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Multiple fulltext fields to search across (overrides 'text_field'). Scores merged per document using best-field strategy (max score). Each field must have a fulltext index."
                }
            },
            "required": ["collection", "query"]
        }
    })]
}
