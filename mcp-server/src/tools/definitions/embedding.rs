//! Embedding generation tool definitions
//!
//! Tools: embed_text, embed_batch, embed_list_models, embed_document

use serde_json::{json, Value};

pub fn tools() -> Vec<Value> {
    vec![
        json!({
            "name": "embed_document",
            "title": "Embed Document with Chunking",
            "description": "Chunk a long document, generate embeddings for each chunk, and store them in a collection. Supports markdown-aware and plain text chunking. Creates a vector index automatically.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "collection": {
                        "type": "string",
                        "description": "Target collection for storing chunks with embeddings"
                    },
                    "content": {
                        "type": "string",
                        "description": "Document content to chunk and embed"
                    },
                    "doc_id": {
                        "type": "string",
                        "description": "Optional parent document ID for linking chunks"
                    },
                    "title": {
                        "type": "string",
                        "description": "Document title (stored in each chunk)"
                    },
                    "metadata": {
                        "type": "object",
                        "description": "Additional metadata to store with each chunk"
                    },
                    "mode": {
                        "type": "string",
                        "description": "Chunking mode: auto (default), markdown, text",
                        "enum": ["auto", "markdown", "text"],
                        "default": "auto"
                    },
                    "chunk_size": {
                        "type": "integer",
                        "description": "Maximum chunk size in characters",
                        "default": 1000
                    },
                    "overlap": {
                        "type": "integer",
                        "description": "Overlap between chunks in characters (default: 100)",
                        "default": 100
                    },
                    "provider": {
                        "type": "string",
                        "description": "Embedding provider",
                        "default": "fasttext"
                    },
                    "create_vector_index": {
                        "type": "boolean",
                        "description": "Create a vector index on the embedding field (default: true)",
                        "default": true
                    }
                },
                "required": ["collection", "content"]
            }
        }),
        json!({
            "name": "embed_text",
            "title": "Generate Text Embedding",
            "description": "Generate an embedding vector for a single text. Default: FastText (Hungarian, offline). Auto-detects available providers from environment variables.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "text": {
                        "type": "string",
                        "description": "Text to embed"
                    },
                    "provider": {
                        "type": "string",
                        "description": "Embedding provider. Available: fasttext (default, offline), ollama (local), openai, cohere, mistral, azure-openai, voyage. Use embed_list_models to see configured providers.",
                        "default": "fasttext"
                    }
                },
                "required": ["text"]
            }
        }),
        json!({
            "name": "embed_batch",
            "title": "Batch Text Embedding",
            "description": "Generate embedding vectors for multiple texts. More efficient than calling embed_text multiple times. Maximum 100 texts per call.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "texts": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Array of texts to embed",
                        "maxItems": 100
                    },
                    "provider": {
                        "type": "string",
                        "description": "Embedding provider (use embed_list_models to see available)",
                        "default": "fasttext"
                    }
                },
                "required": ["texts"]
            }
        }),
        json!({
            "name": "embed_list_models",
            "title": "List Embedding Models",
            "description": "List all available embedding models and their properties (provider, dimension, availability).",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        }),
        json!({
            "name": "embed_cache_stats",
            "title": "Embedding Cache Statistics",
            "description": "Get statistics about the embedding cache including hit rate, entries count, and memory usage.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        }),
        json!({
            "name": "embed_cache_clear",
            "title": "Clear Embedding Cache",
            "description": "Clear all entries from the embedding cache.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        }),
    ]
}
