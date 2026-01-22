//! Error types for gaploader

use std::path::PathBuf;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum GaploaderError {
    #[error("File not found: {0}")]
    FileNotFound(PathBuf),

    #[error("File too large: {path} is {size_mb:.1} MB (max: {max_mb} MB)")]
    FileTooLarge {
        path: PathBuf,
        size_mb: f64,
        max_mb: u64,
    },

    #[error("Unsupported file type: {0}")]
    UnsupportedFileType(String),

    #[error("Bridge not found: {0}")]
    BridgeNotFound(String),

    #[error("Bridge spawn failed: {0}")]
    BridgeSpawnFailed(String),

    #[error("Bridge communication error: {0}")]
    BridgeError(String),

    #[error("JSON-RPC error {code}: {message}")]
    JsonRpc { code: i32, message: String },

    #[error("MCP tool error: {0}")]
    McpToolError(String),

    #[error("Chunking error: {0}")]
    ChunkingError(String),

    #[error("Out of memory: failed to allocate {requested} elements for {context}")]
    OutOfMemory { requested: usize, context: String },

    #[error("Embedding error: {0}")]
    EmbeddingError(String),

    #[error("Config error: {0}")]
    ConfigError(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("TOML parse error: {0}")]
    TomlParse(#[from] toml::de::Error),
}

pub type Result<T> = std::result::Result<T, GaploaderError>;
