//! RAG (Retrieval-Augmented Generation) tool definitions
//!
//! Tools: rag_collection_create, rag_document_import, rag_collection_stats, rag_load_all_chunks

use super::common::fields;
use serde_json::{json, Value};

pub fn tools() -> Vec<Value> {
    vec![
        // rag_collection_create
        json!({
            "name": "rag_collection_create",
            "title": "Create RAG Collection",
            "description": "Creates a collection optimized for RAG (Retrieval-Augmented Generation). Automatically creates vector index (HNSW) and fulltext index for hybrid search. Stores RAG configuration for use by the `search` tool and rag_document_import.",
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
                    "text_fields": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Extra text fields to fulltext-index besides text_field, e.g. [\"title\", \"customer\"]. Each gets its own fulltext index, and the `search` tool then defaults to searching all of them — no manual index_create_fulltext needed."
                    },
                    "provider": {
                        "type": "string",
                        "description": "Embedding provider (ollama, vllm, openai). Defaults to the provider configured in [embedding] section of config.toml."
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
                    },
                    "if_exists": {
                        "type": "string",
                        "description": "How to handle an existing doc_id. 'replace' (default): delete the old chunks and keep the new set (idempotent retry). 'skip': do nothing if the doc_id already has chunks. 'error': fail if it exists. 'append': insert alongside existing chunks (legacy, may duplicate on retry).",
                        "enum": ["replace", "skip", "error", "append"],
                        "default": "replace"
                    },
                    "language": {
                        "type": "string",
                        "description": "Fulltext language for the auto-created index, enabling stemming (e.g. 'hungarian' collapses fékpadon/fékpadot/fékpad). Only applies when no fulltext index exists yet. Default 'none' (no stemming). For an existing collection, set the language via rag_collection_create.",
                        "enum": ["none", "hungarian", "english", "german"],
                        "default": "none"
                    },
                    "text_fields": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Extra metadata text fields to fulltext-index (besides content), e.g. [\"title\", \"customer\"]. Only applies when auto-creating the collection (no RAG config yet)."
                    }
                },
                "required": ["collection", "content"]
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
        // rag_load_all_chunks
        json!({
            "name": "rag_load_all_chunks",
            "title": "Load All Chunks for Document IDs",
            "description": "Loads every chunk for the given doc_ids from a RAG collection. Two modes: (1) pure load when `query` is omitted — chunks sorted by (doc_id, chunk_index) ASC, no scoring; (2) scored load when `query` is provided — chunks scored via fulltext OR-search, sorted by _score DESC, every hit carries `_score`. In both modes adjacent chunks from the same document are merged by default (same algorithm as the `search` tool: removes overlap-induced duplication, dedups table headers). Use for UI doc-detail-view (pure load) or RAG context expansion after a `search` query (scored load). Empty doc_ids returns an empty result (not an error); missing doc_ids are silently skipped.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "collection": fields::collection(),
                    "doc_ids": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Document IDs to load chunks for. Empty list returns empty results."
                    },
                    "query": {
                        "type": "string",
                        "description": "Optional query text. When provided, chunks are scored via fulltext OR-search and `_score` is added to every hit. When omitted, this is a pure load with no scoring."
                    },
                    "text_field": {
                        "type": "string",
                        "description": "Text field used for fulltext search and merge-adjacent-chunks overlap removal.",
                        "default": "content"
                    },
                    "projection": {
                        "type": "object",
                        "description": "MongoDB-style projection: {field: 1} include, {field: 0} exclude."
                    },
                    "merge_chunks": {
                        "type": "boolean",
                        "description": "Merge adjacent chunks from the same source document (default: true). Eliminates overlap-induced duplicates and dedups repeated table headers.",
                        "default": true
                    },
                    "max_chunks_per_doc": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Cap chunks returned per source document. Applied AFTER merge (a merged run counts as one chunk). Omit for no cap."
                    }
                },
                "required": ["collection", "doc_ids"]
            }
        }),
    ]
}
