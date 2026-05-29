//! Tool schema: `search` — intent-shaped hybrid retrieval (Stage A).

use super::common::fields;
use serde_json::{json, Value};

pub fn tools() -> Vec<Value> {
    vec![json!({
        "name": "search",
        "title": "Search (intent-shaped hybrid retrieval)",
        "description": "Document-anchored hybrid search for RAG. Returns ranked source documents, each with its relevant passages (overlap-merged, no internals), optimized for a small LLM: no embeddings, no engine metadata, token-budgeted. Intent-only surface — retrieval mechanism (weights, fusion, thresholds) is server-owned. Stage A: built over the existing RRF fusion; `verdict` is always \"unknown\" (no absolute-relevance/abstention yet).",
        "inputSchema": {
            "type": "object",
            "properties": {
                "collection": fields::collection(),
                "query": {
                    "type": "string",
                    "description": "Natural-language query or keywords."
                },
                "filter": {
                    "type": "object",
                    "description": "Optional structured metadata filter, e.g. {\"year\": 2026, \"customer\": \"X\"}. Applied before retrieval."
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Maximum number of documents to return.",
                    "default": 5
                },
                "format": {
                    "type": "string",
                    "enum": ["structured", "context_block"],
                    "description": "\"structured\" (default): documents with passages. \"context_block\": a single citation-marked text block ready to paste into a prompt.",
                    "default": "structured"
                },
                "debug": {
                    "type": "boolean",
                    "description": "Include a diagnostics block (default false).",
                    "default": false
                }
            },
            "required": ["collection", "query"]
        }
    })]
}
