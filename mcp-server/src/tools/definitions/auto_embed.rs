//! Auto-embedding tool definitions
//!
//! Tools: auto_embed_enable, auto_embed_disable, auto_embed_status

use serde_json::{json, Value};

pub fn tools() -> Vec<Value> {
    vec![
        json!({
            "name": "auto_embed_enable",
            "title": "Enable Auto-Embedding",
            "description": "Enable automatic embedding generation for a collection. When enabled, documents inserted or updated will automatically have embeddings generated from the source field.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "collection": {
                        "type": "string",
                        "description": "Collection name"
                    },
                    "source_field": {
                        "type": "string",
                        "description": "Field containing text to embed (e.g., 'content', 'text', 'body')",
                        "default": "content"
                    },
                    "target_field": {
                        "type": "string",
                        "description": "Field where embedding vector will be stored",
                        "default": "embedding"
                    },
                    "provider": {
                        "type": "string",
                        "description": "Embedding provider (fasttext, openai, ollama, etc.)",
                        "default": "fasttext"
                    },
                    "model": {
                        "type": "string",
                        "description": "Optional model override (provider-specific)"
                    },
                    "dimension": {
                        "type": "integer",
                        "description": "Expected embedding dimension (for validation)"
                    },
                    "skip_if_exists": {
                        "type": "boolean",
                        "description": "Skip embedding if target field already exists (default: false)",
                        "default": false
                    },
                    "chunking": {
                        "type": "object",
                        "description": "Optional chunking configuration for long texts",
                        "properties": {
                            "mode": {
                                "type": "string",
                                "description": "Chunking mode: auto, markdown, text",
                                "default": "auto"
                            },
                            "chunk_size": {
                                "type": "integer",
                                "description": "Maximum chunk size in characters",
                                "default": 1000
                            },
                            "overlap": {
                                "type": "integer",
                                "description": "Overlap between chunks in characters",
                                "default": 100
                            }
                        }
                    }
                },
                "required": ["collection", "source_field", "target_field", "provider"]
            }
        }),
        json!({
            "name": "auto_embed_disable",
            "title": "Disable Auto-Embedding",
            "description": "Disable automatic embedding generation for a collection.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "collection": {
                        "type": "string",
                        "description": "Collection name"
                    }
                },
                "required": ["collection"]
            }
        }),
        json!({
            "name": "auto_embed_status",
            "title": "Auto-Embedding Status",
            "description": "Get the auto-embedding configuration for a collection.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "collection": {
                        "type": "string",
                        "description": "Collection name"
                    }
                },
                "required": ["collection"]
            }
        }),
    ]
}
