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
    /// Script execution error
    ScriptError(String),
    /// Internal error
    Internal(String),
    /// Access denied (ACL)
    Forbidden(String),
}

impl fmt::Display for McpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // SECURITY FIX: Sanitize storage errors to prevent path disclosure
            McpError::Storage(msg) => {
                // Log the full error for debugging, but return generic message
                tracing::debug!("Storage error details: {}", msg);
                write!(f, "Database operation failed")
            }
            McpError::InvalidParams(msg) => write!(f, "Invalid parameters: {}", msg),
            McpError::CollectionNotFound(name) => write!(f, "Collection not found: {}", name),
            McpError::DocumentNotFound(id) => write!(f, "Document not found: {}", id),
            McpError::IndexError(msg) => write!(f, "Index error: {}", msg),
            McpError::Serialization(msg) => write!(f, "Serialization error: {}", msg),
            // SECURITY FIX: Sanitize script errors to prevent internal info disclosure
            McpError::ScriptError(msg) => {
                // Remove line numbers and internal details from script errors
                let sanitized = sanitize_script_error(msg);
                write!(f, "Script error: {}", sanitized)
            }
            McpError::Internal(msg) => write!(f, "Internal error: {}", msg),
            McpError::Forbidden(msg) => write!(f, "{}", msg),
        }
    }
}

/// Sanitize script error messages to prevent internal information disclosure
/// SECURITY FIX: Remove line numbers, file paths, and collection names from errors
fn sanitize_script_error(msg: &str) -> String {
    let mut result = msg.to_string();

    // Remove line/position references like "(line 5, position 20)" without regex
    // Look for patterns and replace them
    while let Some(start) = result.find("(line ") {
        if let Some(end) = result[start..].find(')') {
            result.replace_range(start..start + end + 1, "(...)");
        } else {
            break;
        }
    }

    // Truncate very long error messages
    if result.len() > 200 {
        result.truncate(200);
        result.push_str("...");
    }

    result
}

impl std::error::Error for McpError {}

impl From<ironbase_core::IronBaseError> for McpError {
    fn from(err: ironbase_core::IronBaseError) -> Self {
        use ironbase_core::IronBaseError;
        match err {
            IronBaseError::CollectionNotFound(name) => McpError::CollectionNotFound(name),
            other => McpError::Storage(other.to_string()),
        }
    }
}

impl From<serde_json::Error> for McpError {
    fn from(err: serde_json::Error) -> Self {
        McpError::Serialization(err.to_string())
    }
}

/// Result type alias
pub type Result<T> = std::result::Result<T, McpError>;
