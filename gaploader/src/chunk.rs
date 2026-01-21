//! Chunk structure and metadata

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Source file metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceMetadata {
    /// Full file path
    pub file: String,
    /// Filename only
    pub filename: String,
    /// Load timestamp
    pub loaded_at: DateTime<Utc>,
}

/// Chunk position metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkPosition {
    /// Chunk index (0-based)
    pub index: usize,
    /// Total number of chunks
    pub total: usize,
    /// Start character offset in source
    pub start_char: usize,
    /// End character offset in source
    pub end_char: usize,
}

/// A single chunk of text
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chunk {
    /// Unique chunk ID
    pub _id: String,

    /// Source file metadata
    pub source: SourceMetadata,

    /// Position metadata
    pub chunk: ChunkPosition,

    /// Chunk content
    pub content: String,

    /// Content length in characters
    pub content_length: usize,

    /// Parent heading (markdown only)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heading: Option<String>,

    /// Heading level 1-6 (markdown only)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heading_level: Option<u8>,

    /// Section path (markdown only)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section_path: Option<Vec<String>>,

    /// Embedding vector (if --embed used)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding: Option<Vec<f64>>,

    /// Embedding provider (if --embed used)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding_provider: Option<String>,

    /// Embedding dimension (if --embed used)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding_dim: Option<usize>,
}

impl Chunk {
    /// Create a new chunk
    pub fn new(
        source_file: &str,
        filename: &str,
        content: String,
        index: usize,
        total: usize,
        start_char: usize,
        end_char: usize,
    ) -> Self {
        Self {
            _id: Uuid::new_v4().to_string(),
            source: SourceMetadata {
                file: source_file.to_string(),
                filename: filename.to_string(),
                loaded_at: Utc::now(),
            },
            chunk: ChunkPosition {
                index,
                total,
                start_char,
                end_char,
            },
            content_length: content.len(),
            content,
            heading: None,
            heading_level: None,
            section_path: None,
            embedding: None,
            embedding_provider: None,
            embedding_dim: None,
        }
    }

    /// Set heading metadata (for markdown chunks)
    pub fn with_heading(mut self, heading: String, level: u8) -> Self {
        self.heading = Some(heading);
        self.heading_level = Some(level);
        self
    }

    /// Set section path (for markdown chunks)
    pub fn with_section_path(mut self, path: Vec<String>) -> Self {
        self.section_path = Some(path);
        self
    }

    /// Set embedding
    pub fn with_embedding(mut self, embedding: Vec<f64>, provider: &str) -> Self {
        self.embedding_dim = Some(embedding.len());
        self.embedding = Some(embedding);
        self.embedding_provider = Some(provider.to_string());
        self
    }

    /// Convert to JSON Value
    pub fn to_value(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}
