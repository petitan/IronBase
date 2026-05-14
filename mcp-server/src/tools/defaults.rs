//! Common default values for MCP tools
//!
//! This module centralizes default constants and functions to avoid duplication
//! across tool modules (auto_embed, embedding, chunking, etc.)

// ============================================================================
// Chunking Defaults
// ============================================================================

/// Default chunk size in characters/tokens
pub const DEFAULT_CHUNK_SIZE: usize = 1000;

/// Default overlap between chunks
pub const DEFAULT_CHUNK_OVERLAP: usize = 100;

/// Default chunking mode (auto-detect markdown vs plain text)
pub const DEFAULT_CHUNK_MODE: &str = "auto";

/// Default function for serde
pub fn default_chunk_size() -> usize {
    DEFAULT_CHUNK_SIZE
}

/// Default function for serde
pub fn default_chunk_overlap() -> usize {
    DEFAULT_CHUNK_OVERLAP
}

/// Default function for serde
pub fn default_chunk_mode() -> String {
    DEFAULT_CHUNK_MODE.to_string()
}

// ============================================================================
// Batch Processing Defaults
// ============================================================================

/// Default batch size for backfill operations
pub const DEFAULT_BATCH_SIZE: usize = 100;

/// Maximum batch size for embedding operations
pub const MAX_EMBEDDING_BATCH_SIZE: usize = 100;

/// Configurable batch size override (set from [embedding].batch_size config)
static CONFIGURED_BATCH_SIZE: std::sync::OnceLock<usize> = std::sync::OnceLock::new();

/// Set the batch size from config (call once at startup)
pub fn set_configured_batch_size(size: usize) {
    let _ = CONFIGURED_BATCH_SIZE.set(size);
}

/// Get the effective batch size (config override > default)
pub fn effective_batch_size() -> usize {
    CONFIGURED_BATCH_SIZE
        .get()
        .copied()
        .unwrap_or(DEFAULT_BATCH_SIZE)
}

/// Default function for serde
pub fn default_batch_size() -> usize {
    DEFAULT_BATCH_SIZE
}

// ============================================================================
// Vector Index Defaults
// ============================================================================

/// Default embedding field name (RAG schema default)
pub const DEFAULT_EMBEDDING_FIELD: &str = "embedding";

/// Default text/content field name (RAG schema default)
pub const DEFAULT_TEXT_FIELD: &str = "content";

/// Default vector search limit
pub const DEFAULT_VECTOR_LIMIT: usize = 10;

/// Default vector index max vectors
pub const DEFAULT_MAX_VECTORS: usize = 100_000;

/// Default function for serde
pub fn default_embedding_field() -> String {
    DEFAULT_EMBEDDING_FIELD.to_string()
}

/// Default function for serde
pub fn default_text_field() -> String {
    DEFAULT_TEXT_FIELD.to_string()
}

/// Default function for serde
pub fn default_vector_limit() -> usize {
    DEFAULT_VECTOR_LIMIT
}

// ============================================================================
// System Collections
// ============================================================================

/// List of allowed system collection names (starting with '_')
pub const ALLOWED_SYSTEM_COLLECTIONS: &[&str] = &[
    "_system",
    "_system.rag",
    "_chunks",
    "_cache",
    "_embeddings",
    "_jobs",
    "_scripts",
];

/// Check if a collection name is an allowed system collection
pub fn is_allowed_system_collection(name: &str) -> bool {
    ALLOWED_SYSTEM_COLLECTIONS.contains(&name)
}

// ============================================================================
// Async Job Defaults
// ============================================================================

/// Default for async operations
pub const DEFAULT_ASYNC: bool = true;

/// Default function for serde
pub fn default_async() -> bool {
    DEFAULT_ASYNC
}

/// Maximum consecutive empty batches before stopping backfill
pub const MAX_EMPTY_BATCHES: usize = 3;

// ============================================================================
// Search Fusion Defaults
// ============================================================================

/// Default RRF K constant (lower = wider score spread, more reranking impact)
pub const DEFAULT_RRF_K: f64 = 20.0;

/// Maximum internal limit for search fusion (caps per-source fetch before RRF)
pub const MAX_INTERNAL_LIMIT: usize = 10_000;
