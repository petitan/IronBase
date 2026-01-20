//! RAG type definitions

use serde::{Deserialize, Serialize};

/// RAG memory limits with dynamic RAM-based scaling
///
/// Similar to `AggregationLimits`, these limits protect against memory exhaustion
/// when processing large documents or collections.
///
/// # Default limits (conservative)
/// - `max_document_bytes`: 10 MB - Max document size
/// - `max_chunks_per_document`: 10,000 - Max chunks from single document
/// - `max_total_chunks`: 100,000 - Max chunks in a collection
/// - `max_hnsw_vectors`: 100,000 - Max vectors in HNSW index
///
/// # Dynamic scaling
/// Use `RagLimits::from_system_memory()` for automatic RAM-based scaling:
/// - Uses max 25% of available RAM for RAG operations
/// - Scales limits proportionally to available memory
///
/// # Example
/// ```rust,ignore
/// use ironbase_core::rag::{RagLimits, RagManager};
///
/// // Automatic RAM-based limits
/// let limits = RagLimits::from_system_memory();
///
/// // Or with explicit memory budget
/// let limits = RagLimits::with_memory_budget(512); // 512 MB
/// ```
#[derive(Debug, Clone, Copy)]
pub struct RagLimits {
    /// Maximum document size in bytes
    /// Default: 10 MB
    pub max_document_bytes: usize,

    /// Maximum chunks per document
    /// Default: 10,000
    pub max_chunks_per_document: usize,

    /// Maximum total chunks in a collection
    /// Default: 100,000
    pub max_total_chunks: usize,

    /// Maximum vectors in HNSW index
    /// Default: 100,000
    pub max_hnsw_vectors: usize,

    /// Maximum memory budget in MB (for try_reserve checks)
    /// Default: 512 MB
    pub max_memory_mb: usize,
}

impl Default for RagLimits {
    fn default() -> Self {
        Self {
            max_document_bytes: 10 * 1024 * 1024, // 10 MB
            max_chunks_per_document: 10_000,
            max_total_chunks: 100_000,
            max_hnsw_vectors: 100_000,
            max_memory_mb: 512,
        }
    }
}

impl RagLimits {
    /// Create limits dynamically based on system RAM
    ///
    /// Automatically scales limits to match available system memory:
    /// - Uses max 25% of available RAM for RAG operations
    /// - Scales limits proportionally
    /// - Falls back to `low_memory()` if detection fails
    ///
    /// # Scaling table
    /// | Available RAM | max_doc_bytes | max_chunks_per_doc | max_total | max_hnsw |
    /// |---------------|---------------|--------------------|-----------| ---------|
    /// | < 512 MB      | 5 MB          | 2,500              | 25K       | 25K      |
    /// | 512MB - 2GB   | 10 MB         | 5,000              | 50K       | 50K      |
    /// | 2GB - 8GB     | 25 MB         | 12,500             | 125K      | 125K     |
    /// | 8GB - 32GB    | 50 MB         | 25,000             | 250K      | 250K     |
    /// | > 32GB        | 100 MB        | 50,000             | 500K      | 500K     |
    pub fn from_system_memory() -> Self {
        use crate::aggregation::memory_info::get_available_memory_bytes;

        match get_available_memory_bytes() {
            Some(bytes) => Self::scale_to_memory(bytes),
            None => Self::low_memory(),
        }
    }

    /// Scale limits based on available memory bytes
    fn scale_to_memory(available_bytes: u64) -> Self {
        let available_mb = available_bytes / (1024 * 1024);

        // Use max 25% of available RAM, bounded between 64MB and 4GB
        let max_memory_mb = (available_mb as usize / 4).clamp(64, 4096);

        // Scale factor: 1.0 at 4GB available, proportionally less below
        let scale_factor = (available_mb as f64 / 4096.0).clamp(0.1, 2.5);

        Self {
            // Document size: 10MB base, scales to 100MB max
            max_document_bytes: ((10 * 1024 * 1024) as f64 * scale_factor.min(10.0)) as usize,

            // Chunks per document: 10K base, scales with memory
            max_chunks_per_document: ((10_000.0 * scale_factor) as usize).clamp(2_500, 50_000),

            // Total chunks: 100K base, scales to 500K max
            max_total_chunks: ((100_000.0 * scale_factor) as usize).clamp(25_000, 500_000),

            // HNSW vectors: same as total chunks
            max_hnsw_vectors: ((100_000.0 * scale_factor) as usize).clamp(25_000, 500_000),

            max_memory_mb,
        }
    }

    /// Create limits for a specific memory budget
    ///
    /// # Arguments
    /// * `memory_mb` - Maximum memory to use in megabytes (min: 64)
    ///
    /// # Scaling table
    /// | Budget   | max_doc_bytes | max_chunks_per_doc | max_total | max_hnsw |
    /// |----------|---------------|--------------------|-----------| ---------|
    /// | 64 MB    | 5 MB          | 2,500              | 25K       | 25K      |
    /// | 256 MB   | 10 MB         | 5,000              | 50K       | 50K      |
    /// | 512 MB   | 20 MB         | 10,000             | 100K      | 100K     |
    /// | 1 GB     | 40 MB         | 20,000             | 200K      | 200K     |
    /// | 2 GB     | 80 MB         | 40,000             | 400K      | 400K     |
    pub fn with_memory_budget(memory_mb: usize) -> Self {
        let effective_budget = memory_mb.max(64);

        // Scale factor: 1.0 at 512MB budget
        let scale_factor = (effective_budget as f64 / 512.0).clamp(0.1, 4.0);

        Self {
            max_document_bytes: ((10 * 1024 * 1024) as f64 * scale_factor.min(10.0)) as usize,
            max_chunks_per_document: ((10_000.0 * scale_factor) as usize).clamp(2_500, 50_000),
            max_total_chunks: ((100_000.0 * scale_factor) as usize).clamp(25_000, 500_000),
            max_hnsw_vectors: ((100_000.0 * scale_factor) as usize).clamp(25_000, 500_000),
            max_memory_mb: effective_budget,
        }
    }

    /// Create limits suitable for low-memory environments
    ///
    /// Very conservative limits for memory-constrained systems.
    pub fn low_memory() -> Self {
        Self {
            max_document_bytes: 5 * 1024 * 1024, // 5 MB
            max_chunks_per_document: 2_500,
            max_total_chunks: 25_000,
            max_hnsw_vectors: 25_000,
            max_memory_mb: 128,
        }
    }

    /// Create limits for large documents (enterprise use case)
    ///
    /// Use for ISO 17025, large manuals, etc. when RAM is available.
    /// Note: Still checks system RAM to prevent OOM.
    pub fn enterprise() -> Self {
        use crate::aggregation::memory_info::get_available_memory_bytes;

        // Require at least 8GB for enterprise limits
        let available_mb = get_available_memory_bytes()
            .map(|b| b / (1024 * 1024))
            .unwrap_or(0);

        if available_mb < 8192 {
            // Fall back to system memory limits if < 8GB
            return Self::from_system_memory();
        }

        Self {
            max_document_bytes: 50 * 1024 * 1024, // 50 MB
            max_chunks_per_document: 50_000,
            max_total_chunks: 500_000,
            max_hnsw_vectors: 500_000,
            max_memory_mb: 2048,
        }
    }

    /// Create limits with no restrictions (use with caution!)
    /// WARNING: This can cause OOM on large documents!
    pub fn unlimited() -> Self {
        Self {
            max_document_bytes: usize::MAX,
            max_chunks_per_document: usize::MAX,
            max_total_chunks: usize::MAX,
            max_hnsw_vectors: usize::MAX,
            max_memory_mb: usize::MAX,
        }
    }
}

// Backward compatibility constants (deprecated - use RagLimits instead)
/// Default maximum document size: 10 MB (deprecated - use RagLimits)
pub const DEFAULT_MAX_DOCUMENT_BYTES: usize = 10 * 1024 * 1024;

/// Maximum chunks per document (deprecated - use RagLimits)
pub const MAX_CHUNKS_PER_DOCUMENT: usize = 10_000;

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
    /// Maximum document size in bytes (default: 10 MB)
    /// Documents larger than this will be rejected.
    #[serde(default = "default_max_document_bytes")]
    pub max_document_bytes: usize,
    /// Maximum chunks per document (default: 10,000)
    /// Documents producing more chunks will be rejected.
    #[serde(default = "default_max_chunks_per_document")]
    pub max_chunks_per_document: usize,
}

fn default_max_document_bytes() -> usize {
    DEFAULT_MAX_DOCUMENT_BYTES
}

fn default_max_chunks_per_document() -> usize {
    MAX_CHUNKS_PER_DOCUMENT
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
            max_document_bytes: DEFAULT_MAX_DOCUMENT_BYTES,
            max_chunks_per_document: MAX_CHUNKS_PER_DOCUMENT,
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

/// Result of chunk upsert operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpsertResult {
    /// Chunk ID that was upserted
    pub chunk_id: String,
    /// True if this was an update, false if insert
    pub is_update: bool,
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
