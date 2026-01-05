//! Error types for IronBase MCP Server
//!
//! RMCP-style structured error handling with JSON-RPC error codes.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;

// ============================================================================
// Error Codes (JSON-RPC standard + MCP-specific)
// ============================================================================

/// JSON-RPC and MCP error codes
///
/// Standard JSON-RPC codes: -32700 to -32600
/// MCP-specific codes: -32000 to -32099
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(i32)]
pub enum ErrorCode {
    // Standard JSON-RPC 2.0 errors
    /// Invalid JSON was received
    ParseError = -32700,
    /// The JSON sent is not a valid Request object
    InvalidRequest = -32600,
    /// The method does not exist / is not available
    MethodNotFound = -32601,
    /// Invalid method parameter(s)
    InvalidParams = -32602,
    /// Internal JSON-RPC error
    InternalError = -32603,

    // MCP-specific errors (-32000 to -32099)
    /// Generic server error
    ServerError = -32000,
    /// Collection not found
    CollectionNotFound = -32001,
    /// Document not found
    DocumentNotFound = -32002,
    /// Index not found or index error
    IndexError = -32003,
    /// Access denied (ACL/authentication)
    AccessDenied = -32004,
    /// Transaction error
    TransactionError = -32005,
    /// Validation error
    ValidationError = -32006,
    /// Resource exhausted (OOM, limit exceeded)
    ResourceExhausted = -32007,
    /// Operation timeout
    Timeout = -32008,
    /// Script execution error
    ScriptError = -32009,
    /// Serialization/deserialization error
    SerializationError = -32010,
    /// Aggregation pipeline error
    AggregationError = -32011,
}

impl ErrorCode {
    /// Get the numeric code value
    pub fn code(&self) -> i32 {
        *self as i32
    }

    /// Get the default message for this error code
    pub fn default_message(&self) -> &'static str {
        match self {
            ErrorCode::ParseError => "Parse error",
            ErrorCode::InvalidRequest => "Invalid request",
            ErrorCode::MethodNotFound => "Method not found",
            ErrorCode::InvalidParams => "Invalid params",
            ErrorCode::InternalError => "Internal error",
            ErrorCode::ServerError => "Server error",
            ErrorCode::CollectionNotFound => "Collection not found",
            ErrorCode::DocumentNotFound => "Document not found",
            ErrorCode::IndexError => "Index error",
            ErrorCode::AccessDenied => "Access denied",
            ErrorCode::TransactionError => "Transaction error",
            ErrorCode::ValidationError => "Validation error",
            ErrorCode::ResourceExhausted => "Resource exhausted",
            ErrorCode::Timeout => "Operation timeout",
            ErrorCode::ScriptError => "Script error",
            ErrorCode::SerializationError => "Serialization error",
            ErrorCode::AggregationError => "Aggregation error",
        }
    }
}

impl From<i32> for ErrorCode {
    fn from(code: i32) -> Self {
        match code {
            -32700 => ErrorCode::ParseError,
            -32600 => ErrorCode::InvalidRequest,
            -32601 => ErrorCode::MethodNotFound,
            -32602 => ErrorCode::InvalidParams,
            -32603 => ErrorCode::InternalError,
            -32000 => ErrorCode::ServerError,
            -32001 => ErrorCode::CollectionNotFound,
            -32002 => ErrorCode::DocumentNotFound,
            -32003 => ErrorCode::IndexError,
            -32004 => ErrorCode::AccessDenied,
            -32005 => ErrorCode::TransactionError,
            -32006 => ErrorCode::ValidationError,
            -32007 => ErrorCode::ResourceExhausted,
            -32008 => ErrorCode::Timeout,
            -32009 => ErrorCode::ScriptError,
            -32010 => ErrorCode::SerializationError,
            -32011 => ErrorCode::AggregationError,
            _ => ErrorCode::InternalError,
        }
    }
}

// ============================================================================
// Operation Context (kept for detailed logging)
// ============================================================================

/// Operation context for detailed error logging
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

// ============================================================================
// McpError - Structured error type
// ============================================================================

/// MCP Server Error with structured code, message, and optional data
#[derive(Debug, Clone)]
pub struct McpError {
    /// Error code (JSON-RPC standard or MCP-specific)
    pub code: ErrorCode,
    /// Human-readable error message
    pub message: String,
    /// Optional additional data (JSON)
    pub data: Option<Value>,
}

impl McpError {
    /// Create a new error with code and message
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    /// Create error with additional data
    pub fn with_data(code: ErrorCode, message: impl Into<String>, data: Value) -> Self {
        Self {
            code,
            message: message.into(),
            data: Some(data),
        }
    }

    // ========================================================================
    // Convenience constructors
    // ========================================================================

    /// Invalid parameters error
    pub fn invalid_params(msg: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidParams, msg)
    }

    /// Collection not found error
    pub fn collection_not_found(name: &str) -> Self {
        Self::with_data(
            ErrorCode::CollectionNotFound,
            format!("Collection '{}' not found", name),
            serde_json::json!({"collection": name}),
        )
    }

    /// Document not found error
    pub fn document_not_found(id: impl fmt::Display) -> Self {
        Self::with_data(
            ErrorCode::DocumentNotFound,
            format!("Document '{}' not found", id),
            serde_json::json!({"id": id.to_string()}),
        )
    }

    /// Index error
    pub fn index_error(msg: impl Into<String>) -> Self {
        Self::new(ErrorCode::IndexError, msg)
    }

    /// Access denied error
    pub fn access_denied(msg: impl Into<String>) -> Self {
        Self::new(ErrorCode::AccessDenied, msg)
    }

    /// Forbidden (alias for access_denied, backward compat)
    pub fn forbidden(msg: impl Into<String>) -> Self {
        Self::access_denied(msg)
    }

    /// Resource exhausted (OOM, limits)
    pub fn resource_exhausted(reason: impl Into<String>, limit: Option<usize>) -> Self {
        let msg = reason.into();
        let data = if let Some(lim) = limit {
            Some(serde_json::json!({"limit": lim, "reason": &msg}))
        } else {
            Some(serde_json::json!({"reason": &msg}))
        };
        Self {
            code: ErrorCode::ResourceExhausted,
            message: msg,
            data,
        }
    }

    /// Operation too large (backward compat alias)
    pub fn operation_too_large(msg: impl Into<String>) -> Self {
        Self::resource_exhausted(msg, None)
    }

    /// Script execution error
    pub fn script_error(msg: impl Into<String>) -> Self {
        let sanitized = sanitize_script_error(&msg.into());
        Self::new(ErrorCode::ScriptError, sanitized)
    }

    /// Aggregation error
    pub fn aggregation(msg: impl Into<String>) -> Self {
        Self::new(ErrorCode::AggregationError, msg)
    }

    /// Serialization error
    pub fn serialization(msg: impl Into<String>) -> Self {
        Self::new(ErrorCode::SerializationError, msg)
    }

    /// Internal/storage error (sanitized)
    pub fn storage(msg: impl Into<String>) -> Self {
        let full_msg = msg.into();
        tracing::debug!("Storage error details: {}", full_msg);
        Self::new(ErrorCode::ServerError, "Database operation failed")
    }

    /// Internal error
    pub fn internal(msg: impl Into<String>) -> Self {
        Self::new(ErrorCode::InternalError, msg)
    }

    /// Method not found
    pub fn method_not_found(method: &str) -> Self {
        Self::with_data(
            ErrorCode::MethodNotFound,
            format!("Method '{}' not found", method),
            serde_json::json!({"method": method}),
        )
    }

    /// Timeout error
    pub fn timeout(msg: impl Into<String>) -> Self {
        Self::new(ErrorCode::Timeout, msg)
    }

    /// Transaction error
    pub fn transaction(msg: impl Into<String>) -> Self {
        Self::new(ErrorCode::TransactionError, msg)
    }

    /// Validation error
    pub fn validation(msg: impl Into<String>) -> Self {
        Self::new(ErrorCode::ValidationError, msg)
    }

    /// Panic caught (internal error)
    pub fn panic(msg: impl Into<String>) -> Self {
        let full_msg = msg.into();
        tracing::error!("Panic caught in tool handler: {}", full_msg);
        Self::new(ErrorCode::InternalError, "Internal error: operation failed unexpectedly")
    }

    // ========================================================================
    // Context-aware constructors (for detailed logging)
    // ========================================================================

    /// Create a storage error with operation context
    pub fn storage_with_context(error: impl fmt::Display, ctx: &OperationContext) -> Self {
        tracing::debug!(context = %ctx, "Storage error: {}", error);
        Self::storage(format!("{} ({})", error, ctx))
    }

    /// Create an invalid params error with context
    pub fn invalid_params_with_context(msg: impl Into<String>, ctx: &OperationContext) -> Self {
        let msg = msg.into();
        tracing::debug!(context = %ctx, "Invalid params: {}", msg);
        Self::invalid_params(msg)
    }

    /// Create an index error with context
    pub fn index_error_with_context(msg: impl Into<String>, ctx: &OperationContext) -> Self {
        let msg = msg.into();
        tracing::debug!(context = %ctx, "Index error: {}", msg);
        Self::index_error(msg)
    }

    // ========================================================================
    // JSON-RPC error conversion
    // ========================================================================

    /// Convert to JSON-RPC error object
    pub fn to_json_rpc_error(&self) -> Value {
        let mut error = serde_json::json!({
            "code": self.code.code(),
            "message": self.message
        });
        if let Some(data) = &self.data {
            error["data"] = data.clone();
        }
        error
    }
}

impl fmt::Display for McpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for McpError {}

// ============================================================================
// Backward compatibility - From implementations
// ============================================================================

impl From<ironbase_core::IronBaseError> for McpError {
    fn from(err: ironbase_core::IronBaseError) -> Self {
        use ironbase_core::IronBaseError;
        match err {
            IronBaseError::CollectionNotFound(name) => McpError::collection_not_found(&name),
            IronBaseError::AggregationError(msg) => McpError::aggregation(msg),
            other => McpError::storage(other.to_string()),
        }
    }
}

impl From<serde_json::Error> for McpError {
    fn from(err: serde_json::Error) -> Self {
        McpError::serialization(err.to_string())
    }
}

// ============================================================================
// Helper functions
// ============================================================================

/// Sanitize script error messages to prevent internal information disclosure
fn sanitize_script_error(msg: &str) -> String {
    let mut result = msg.to_string();

    // Remove line/position references like "(line 5, position 20)"
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

// ============================================================================
// Result type alias
// ============================================================================

/// Result type alias
pub type Result<T> = std::result::Result<T, McpError>;

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_code_values() {
        assert_eq!(ErrorCode::ParseError.code(), -32700);
        assert_eq!(ErrorCode::InvalidParams.code(), -32602);
        assert_eq!(ErrorCode::CollectionNotFound.code(), -32001);
        assert_eq!(ErrorCode::ResourceExhausted.code(), -32007);
    }

    #[test]
    fn test_error_code_from_i32() {
        assert_eq!(ErrorCode::from(-32700), ErrorCode::ParseError);
        assert_eq!(ErrorCode::from(-32001), ErrorCode::CollectionNotFound);
        assert_eq!(ErrorCode::from(-99999), ErrorCode::InternalError); // Unknown -> Internal
    }

    #[test]
    fn test_mcp_error_constructors() {
        let err = McpError::invalid_params("missing field 'collection'");
        assert_eq!(err.code, ErrorCode::InvalidParams);
        assert!(err.message.contains("missing field"));
        assert!(err.data.is_none());

        let err = McpError::collection_not_found("users");
        assert_eq!(err.code, ErrorCode::CollectionNotFound);
        assert!(err.message.contains("users"));
        assert!(err.data.is_some());
    }

    #[test]
    fn test_mcp_error_to_json_rpc() {
        let err = McpError::collection_not_found("test");
        let json = err.to_json_rpc_error();

        assert_eq!(json["code"], -32001);
        assert!(json["message"].as_str().unwrap().contains("test"));
        assert!(json["data"]["collection"].as_str().unwrap() == "test");
    }

    #[test]
    fn test_resource_exhausted_with_limit() {
        let err = McpError::resource_exhausted("Too many documents", Some(10000));
        assert_eq!(err.code, ErrorCode::ResourceExhausted);
        assert!(err.data.as_ref().unwrap()["limit"] == 10000);
    }

    #[test]
    fn test_sanitize_script_error() {
        let msg = "Error at (line 5, position 20): undefined variable";
        let sanitized = sanitize_script_error(msg);
        assert!(!sanitized.contains("line 5"));
        assert!(sanitized.contains("(...)"));
    }
}
