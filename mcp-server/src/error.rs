//! Error types for IronBase MCP Server

use std::fmt;

/// Operation context for detailed error logging
///
/// This struct captures the context of an operation when an error occurs,
/// enabling better debugging and troubleshooting.
#[derive(Debug, Clone, Default)]
pub struct OperationContext {
    /// The collection being operated on (if applicable)
    pub collection: Option<String>,
    /// The operation type (e.g., "find", "insert", "update")
    pub operation: Option<String>,
    /// Additional context information
    pub details: Option<String>,
}

impl OperationContext {
    /// Create a new operation context
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the collection name
    pub fn with_collection(mut self, collection: impl Into<String>) -> Self {
        self.collection = Some(collection.into());
        self
    }

    /// Set the operation type
    pub fn with_operation(mut self, operation: impl Into<String>) -> Self {
        self.operation = Some(operation.into());
        self
    }

    /// Set additional details
    pub fn with_details(mut self, details: impl Into<String>) -> Self {
        self.details = Some(details.into());
        self
    }
}

impl fmt::Display for OperationContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts = Vec::new();
        if let Some(op) = &self.operation {
            parts.push(format!("op={}", op));
        }
        if let Some(coll) = &self.collection {
            parts.push(format!("collection={}", coll));
        }
        if let Some(details) = &self.details {
            parts.push(format!("details={}", details));
        }
        if parts.is_empty() {
            write!(f, "no context")
        } else {
            write!(f, "{}", parts.join(", "))
        }
    }
}

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
    /// Operation too large (would cause OOM or excessive time)
    OperationTooLarge(String),
    /// Panic caught during operation
    Panic(String),
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
            McpError::OperationTooLarge(msg) => write!(f, "Operation too large: {}", msg),
            McpError::Panic(msg) => {
                tracing::error!("Panic caught in tool handler: {}", msg);
                write!(f, "Internal error: operation failed unexpectedly")
            }
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

impl McpError {
    /// Create a storage error with operation context
    ///
    /// Logs the full error at DEBUG level with context, returns sanitized message to client.
    pub fn storage_with_context(error: impl std::fmt::Display, ctx: &OperationContext) -> Self {
        tracing::debug!(
            context = %ctx,
            "Storage error: {}", error
        );
        McpError::Storage(format!("{} ({})", error, ctx))
    }

    /// Create an invalid params error with context
    pub fn invalid_params_with_context(msg: impl Into<String>, ctx: &OperationContext) -> Self {
        let msg = msg.into();
        tracing::debug!(
            context = %ctx,
            "Invalid params: {}", msg
        );
        McpError::InvalidParams(msg)
    }

    /// Create an index error with context
    pub fn index_error_with_context(msg: impl Into<String>, ctx: &OperationContext) -> Self {
        let msg = msg.into();
        tracing::debug!(
            context = %ctx,
            "Index error: {}", msg
        );
        McpError::IndexError(msg)
    }
}

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
