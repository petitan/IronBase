//! MCP client error types

use thiserror::Error;

/// MCP Client Error
#[derive(Error, Debug)]
pub enum McpError {
    /// HTTP transport error
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    /// JSON serialization/deserialization error
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// IO error (for stdio transport)
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON-RPC error from server
    #[error("RPC error {code}: {message}")]
    Rpc { code: i64, message: String },

    /// MCP tool returned an error
    #[error("Tool error: {0}")]
    Tool(String),

    /// Connection error
    #[error("Connection error: {0}")]
    Connection(String),

    /// Invalid response format
    #[error("Invalid response: {0}")]
    InvalidResponse(String),

    /// Server not initialized
    #[error("Server not initialized")]
    NotInitialized,

    /// Timeout waiting for response
    #[error("Request timeout")]
    Timeout,
}

impl McpError {
    /// Create an RPC error from code and message
    pub fn rpc(code: i64, message: impl Into<String>) -> Self {
        Self::Rpc {
            code,
            message: message.into(),
        }
    }

    /// Create a tool error
    pub fn tool(message: impl Into<String>) -> Self {
        Self::Tool(message.into())
    }

    /// Create a connection error
    pub fn connection(message: impl Into<String>) -> Self {
        Self::Connection(message.into())
    }

    /// Create an invalid response error
    pub fn invalid_response(message: impl Into<String>) -> Self {
        Self::InvalidResponse(message.into())
    }
}

/// Result type for MCP operations
pub type McpResult<T> = Result<T, McpError>;
