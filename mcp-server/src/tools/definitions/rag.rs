//! RAG (Retrieval-Augmented Generation) tool definitions
//!
//! Tools: rag_collection_create, rag_document_import, rag_search, rag_collection_stats
//!
//! Key difference from hybrid_search: rag_search auto-embeds the query!

use super::common::fields;
use serde_json::{json, Value};

pub fn tools() -> Vec<Value> {
    vec![
        // rag_collection_create
        json!({
            "name": "rag_collection_create",
            "title": "Create RAG Collection",
            "description": "Creates a collection optimized for RAG (Retrieval-Augmented Generation). Automatically creates vector index (HNSW) and fulltext index for hybrid search. Stores RAG configuration for use by rag_search and rag_document_import.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "collection": fields::collection(),
                    "embedding_field": {
                        "type": "string",
                        "description": "Field to store embedding vectors (default: 'embedding')",
                        "default": "embedding"
                    },
                    "text_field": {
                        "type": "string",
                        "description": "Field containing text content (default: 'content')",
                        "default": "content"
                    },
                    "provider": {
                        "type": "string",
                        "description": "Embedding provider: fasttext, openai, ollama, etc. (default: 'fasttext')",
                        "default": "fasttext"
                    },
                    "language": {
                        "type": "string",
                        "description": "Language for fulltext stemming/stop words (default: 'none' = no stemming)",
                        "enum": ["none", "hungarian", "english", "german"],
                        "default": "none"
                    }
                },
                "required": ["collection"]
            }
        }),
        // rag_document_import
        json!({
            "name": "rag_document_import",
            "title": "Import Document for RAG",
            "description": "Imports a document with automatic chunking and embedding. Creates chunks with embeddings suitable for semantic search. Works without rag_collection_create (uses defaults). Each chunk includes: doc_id, chunk_index, chunk_total, content, embedding, title, section metadata.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "collection": fields::collection(),
                    "content": {
                        "type": "string",
                        "description": "Document content to import and chunk"
                    },
                    "title": {
                        "type": "string",
                        "description": "Optional document title (stored with each chunk)"
                    },
                    "metadata": {
                        "type": "object",
                        "description": "Optional metadata to attach to each chunk"
                    },
                    "doc_id": {
                        "type": "string",
                        "description": "Optional parent document ID (auto-generated UUID if not provided)"
                    },
                    "chunk_size": {
                        "type": "integer",
                        "description": "Maximum chunk size in characters (default: 1000)",
                        "default": 1000,
                        "minimum": 100,
                        "maximum": 10000
                    },
                    "overlap": {
                        "type": "integer",
                        "description": "Overlap between chunks in characters (default: 100)",
                        "default": 100,
                        "minimum": 0,
                        "maximum": 500
                    },
                    "mode": {
                        "type": "string",
                        "description": "Chunking mode: auto (detect markdown), markdown (preserve headings), text (simple split)",
                        "enum": ["auto", "markdown", "text"],
                        "default": "auto"
                    },
                    "provider": {
                        "type": "string",
                        "description": "Override embedding provider (uses collection config if not specified)"
                    }
                },
                "required": ["collection", "content"]
            }
        }),
        // rag_search - THE KEY TOOL
        json!({
            "name": "rag_search",
            "title": "RAG Semantic Search",
            "description": "Semantic search with AUTOMATIC query embedding. Unlike hybrid_search, you only provide a text query - the embedding is generated internally. Combines vector similarity and fulltext search using RRF fusion with reranking and MMR diversity reranking (cosine similarity based deduplication). Use mmr_lambda to tune relevance vs diversity.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "collection": fields::collection(),
                    "query": {
                        "type": "string",
                        "description": "Natural language search query. Will be automatically embedded using the collection's configured provider."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum results to return (default: 10)",
                        "default": 10,
                        "minimum": 1,
                        "maximum": 100
                    },
                    "vector_weight": {
                        "type": "number",
                        "description": "Weight for vector similarity (default: 0.5)",
                        "default": 0.5,
                        "minimum": 0.0,
                        "maximum": 1.0
                    },
                    "fulltext_weight": {
                        "type": "number",
                        "description": "Weight for fulltext search (default: 0.5)",
                        "default": 0.5,
                        "minimum": 0.0,
                        "maximum": 1.0
                    },
                    "rerank": {
                        "type": "boolean",
                        "description": "Enable result reranking (default: true). Applies exact phrase match (1.3x), keyword density (1.0-1.1x), length penalty (0.8x).",
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
                    "projection": fields::projection_simple(),
                    "provider": {
                        "type": "string",
                        "description": "Override embedding provider for query (uses collection config if not specified)"
                    }
                },
                "required": ["collection", "query"]
            }
        }),
        // rag_collection_stats
        json!({
            "name": "rag_collection_stats",
            "title": "RAG Collection Statistics",
            "description": "Returns RAG-specific statistics for a collection including chunk count, source document count, index info, and embedding configuration.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "collection": fields::collection()
                },
                "required": ["collection"]
            }
        }),
    ]
}
