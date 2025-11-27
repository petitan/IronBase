//! Error types for IronBase MCP Server

use std::fmt;

/// MCP Server Error
#[derive(Debug)]
pub enum McpError {
    /// IronBase storage error
    Storage(String),
    /// Invalid parameters
    InvalidParams(String),
    /// Collection not found
    CollectionNotFound(String),
    /// Document not found
    DocumentNotFound(String),
    /// Index error
    IndexError(String),
    /// Serialization error
    Serialization(String),
    /// Internal error
    Internal(String),
}

impl fmt::Display for McpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            McpError::Storage(msg) => write!(f, "Storage error: {}", msg),
            McpError::InvalidParams(msg) => write!(f, "Invalid parameters: {}", msg),
            McpError::CollectionNotFound(name) => write!(f, "Collection not found: {}", name),
            McpError::DocumentNotFound(id) => write!(f, "Document not found: {}", id),
            McpError::IndexError(msg) => write!(f, "Index error: {}", msg),
            McpError::Serialization(msg) => write!(f, "Serialization error: {}", msg),
            McpError::Internal(msg) => write!(f, "Internal error: {}", msg),
        }
    }
}

impl std::error::Error for McpError {}

impl From<ironbase_core::MongoLiteError> for McpError {
    fn from(err: ironbase_core::MongoLiteError) -> Self {
        McpError::Storage(err.to_string())
    }
}

impl From<serde_json::Error> for McpError {
    fn from(err: serde_json::Error) -> Self {
        McpError::Serialization(err.to_string())
    }
}

/// Result type alias
pub type Result<T> = std::result::Result<T, McpError>;
