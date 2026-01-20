//! Embedding generation tool definitions
//!
//! Tools: embed_text, embed_batch, embed_list_models

use serde_json::{json, Value};

pub fn tools() -> Vec<Value> {
    vec![
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
    ]
}
