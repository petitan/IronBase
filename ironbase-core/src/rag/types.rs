//! RAG type definitions

use serde::{Deserialize, Serialize};

/// Configuration for chunking documents
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkConfig {
    /// Maximum tokens per chunk (default: 1000)
    pub max_tokens: usize,
    /// Overlap tokens between chunks (default: 100)
    pub overlap_tokens: usize,
    /// Minimum chunk size in tokens (default: 50)
    pub min_chunk_size: usize,
    /// Preserve tables as single chunks (default: true)
    pub preserve_tables: bool,
    /// Preserve code blocks as single chunks (default: true)
    pub preserve_code: bool,
    /// Split on headings (default: true)
    pub split_on_heading: bool,
}

impl Default for ChunkConfig {
    fn default() -> Self {
        Self {
            max_tokens: 1000,
            overlap_tokens: 100,
            min_chunk_size: 50,
            preserve_tables: true,
            preserve_code: true,
            split_on_heading: true,
        }
    }
}

/// Type of markdown block
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BlockType {
    /// Regular paragraph
    Paragraph,
    /// Heading (h1-h6)
    Heading,
    /// Table
    Table,
    /// Code block
    Code,
    /// List (ordered or unordered)
    List,
    /// Block quote
    Quote,
}

/// A chunk of text with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chunk {
    /// The text content
    pub text: String,
    /// Character range in original document (start, end)
    pub char_range: (usize, usize),
    /// Estimated token count
    pub token_count: usize,
    /// Type of block this chunk came from
    pub block_type: BlockType,
    /// Section hierarchy (e.g., ["5. Requirements", "5.3 Management"])
    pub section_path: Vec<String>,
    /// Parent heading text
    pub parent_heading: Option<String>,
}

/// Search result from RAG query
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    /// Document ID
    pub doc_id: String,
    /// Document title
    pub doc_title: String,
    /// Chunk ID
    pub chunk_id: String,
    /// Section path
    pub section: String,
    /// Matching text
    pub text: String,
    /// Similarity score (0.0 - 1.0)
    pub score: f32,
    /// Block type
    pub block_type: BlockType,
}

/// Result of document import
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportResult {
    /// Document ID
    pub doc_id: String,
    /// Number of chunks created
    pub chunks_created: usize,
    /// Number of tables extracted
    pub tables_extracted: usize,
    /// Import time in milliseconds
    pub import_time_ms: u64,
}

/// HNSW index configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HnswConfig {
    /// Maximum number of connections per node (default: 16)
    pub m: usize,
    /// Size of dynamic candidate list during construction (default: 200)
    pub ef_construction: usize,
    /// Size of dynamic candidate list during search (default: 50)
    pub ef_search: usize,
    /// Vector dimension
    pub dim: usize,
}

impl Default for HnswConfig {
    fn default() -> Self {
        Self {
            m: 16,
            ef_construction: 200,
            ef_search: 50,
            dim: 300, // FastText dimension
        }
    }
}

/// RAG-specific errors
#[derive(Debug, thiserror::Error)]
pub enum RagError {
    #[error("FastText model not found: {0}")]
    ModelNotFound(String),

    #[error("Invalid FastText format: {0}")]
    InvalidFormat(String),

    #[error("Word not found in vocabulary: {0}")]
    WordNotFound(String),

    #[error("HNSW index error: {0}")]
    HnswError(String),

    #[error("Chunk error: {0}")]
    ChunkError(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

pub type RagResult<T> = Result<T, RagError>;
