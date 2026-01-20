//! RAG (Retrieval Augmented Generation) tool definitions
//!
//! Provides semantic search capabilities using FastText embeddings and HNSW indexing.

use super::common::fields;
use serde_json::{json, Value};

pub fn tools() -> Vec<Value> {
    vec![
        // =====================================================================
        // RAG Collection Management
        // =====================================================================
        json!({
            "name": "rag_collection_create",
            "title": "Create RAG Collection",
            "description": "Create a new RAG collection for semantic document search. Uses FastText model from config.toml [rag] section by default.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Collection name for the RAG knowledge base"
                    },
                    "model_path": {
                        "type": "string",
                        "description": "Path to FastText .bin model file. Optional if configured in config.toml [rag] fasttext_model"
                    },
                    "chunk_max_tokens": {
                        "type": "integer",
                        "description": "Maximum tokens per chunk (default: 1000)",
                        "default": 1000
                    },
                    "chunk_overlap": {
                        "type": "integer",
                        "description": "Token overlap between chunks (default: 100)",
                        "default": 100
                    },
                    "admin_key": fields::admin_key()
                },
                "required": ["name", "admin_key"]
            }
        }),
        json!({
            "name": "rag_collection_list",
            "title": "List RAG Collections",
            "description": "List all RAG collections with their statistics",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "required": []
            }
        }),
        json!({
            "name": "rag_collection_stats",
            "title": "RAG Collection Statistics",
            "description": "Get detailed statistics for a RAG collection",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "RAG collection name"
                    },
                    "collection": {
                        "type": "string",
                        "description": "RAG collection name (alias for 'name')"
                    }
                },
                "required": []
            }
        }),
        json!({
            "name": "rag_collection_delete",
            "title": "Delete RAG Collection",
            "description": "Delete a RAG collection and all its documents",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "RAG collection name to delete"
                    },
                    "admin_key": fields::admin_key()
                },
                "required": ["name", "admin_key"]
            }
        }),
        // =====================================================================
        // Document Management
        // =====================================================================
        json!({
            "name": "rag_document_import",
            "title": "Import Document to RAG",
            "description": "Import a markdown/text document into a RAG collection. Automatically chunks, embeds, and indexes the content.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "collection": {
                        "type": "string",
                        "description": "RAG collection name"
                    },
                    "doc_id": {
                        "type": "string",
                        "description": "Unique document identifier"
                    },
                    "title": {
                        "type": "string",
                        "description": "Document title for display"
                    },
                    "content": {
                        "type": "string",
                        "description": "Markdown or plain text content"
                    }
                },
                "required": ["collection", "doc_id", "title", "content"]
            }
        }),
        json!({
            "name": "rag_document_list",
            "title": "List RAG Documents",
            "description": "List all documents in a RAG collection",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "collection": {
                        "type": "string",
                        "description": "RAG collection name"
                    },
                    "name": {
                        "type": "string",
                        "description": "RAG collection name (alias for 'collection')"
                    }
                },
                "required": []
            }
        }),
        json!({
            "name": "rag_document_delete",
            "title": "Delete RAG Document",
            "description": "Remove a document and its chunks from a RAG collection",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "collection": {
                        "type": "string",
                        "description": "RAG collection name"
                    },
                    "doc_id": {
                        "type": "string",
                        "description": "Document ID to delete"
                    }
                },
                "required": ["collection", "doc_id"]
            }
        }),
        json!({
            "name": "rag_chunk_upsert",
            "title": "Upsert RAG Chunk",
            "description": "Insert or update a single chunk in a RAG collection. If chunk_id exists, updates the text and re-embeds. If not, creates new chunk.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "collection": {
                        "type": "string",
                        "description": "RAG collection name"
                    },
                    "doc_id": {
                        "type": "string",
                        "description": "Document ID this chunk belongs to (must exist)"
                    },
                    "chunk_id": {
                        "type": "string",
                        "description": "Chunk ID (e.g., '7:13' for doc 7, chunk 13)"
                    },
                    "text": {
                        "type": "string",
                        "description": "New text content for the chunk"
                    },
                    "section": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional section path (e.g., ['Chapter 1', 'Section 1.2'])"
                    }
                },
                "required": ["collection", "doc_id", "chunk_id", "text"]
            }
        }),
        // =====================================================================
        // Search
        // =====================================================================
        json!({
            "name": "rag_search",
            "title": "Semantic Search",
            "description": "Search for relevant document chunks using semantic similarity. Returns chunks ranked by relevance.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "collection": {
                        "type": "string",
                        "description": "RAG collection name"
                    },
                    "query": {
                        "type": "string",
                        "description": "Natural language search query"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum results to return (default: 5)",
                        "default": 5
                    },
                    "min_score": {
                        "type": "number",
                        "description": "Minimum similarity score threshold (0.0-1.0)",
                        "minimum": 0,
                        "maximum": 1
                    }
                },
                "required": ["collection", "query"]
            }
        }),
    ]
}
