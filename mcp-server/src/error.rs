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
    /// Request was cancelled (via notifications/cancelled)
    RequestCancelled = -32012,
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
            ErrorCode::RequestCancelled => "Request cancelled",
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
            -32012 => ErrorCode::RequestCancelled,
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

    /// Script execution error (no masking)
    pub fn script_error(msg: impl Into<String>) -> Self {
        Self::new(ErrorCode::ScriptError, msg)
    }

    /// Aggregation error
    pub fn aggregation(msg: impl Into<String>) -> Self {
        Self::new(ErrorCode::AggregationError, msg)
    }

    /// Serialization error
    pub fn serialization(msg: impl Into<String>) -> Self {
        Self::new(ErrorCode::SerializationError, msg)
    }

    /// Internal/storage error (no masking - full details exposed to client)
    pub fn storage(msg: impl Into<String>) -> Self {
        Self::new(ErrorCode::ServerError, msg)
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

    /// Request cancelled (via MCP notifications/cancelled)
    pub fn cancelled(reason: impl Into<String>) -> Self {
        Self::new(ErrorCode::RequestCancelled, reason)
    }

    /// Transaction error
    pub fn transaction(msg: impl Into<String>) -> Self {
        Self::new(ErrorCode::TransactionError, msg)
    }

    /// Validation error
    pub fn validation(msg: impl Into<String>) -> Self {
        Self::new(ErrorCode::ValidationError, msg)
    }

    /// Panic caught (no masking - full panic details exposed)
    pub fn panic(msg: impl Into<String>) -> Self {
        let full_msg = msg.into();
        tracing::error!("Panic caught in tool handler: {}", full_msg);
        Self::new(
            ErrorCode::InternalError,
            format!("Internal panic: {}", full_msg),
        )
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
            // Collection errors
            IronBaseError::CollectionNotFound(name) => McpError::collection_not_found(&name),
            IronBaseError::CollectionExists(name) => {
                McpError::validation(format!("Collection '{}' already exists", name))
            }
            IronBaseError::InvalidCollectionName(name) => {
                McpError::validation(format!("Invalid collection name: {}", name))
            }
            IronBaseError::SystemCollectionError(msg) => McpError::validation(msg),

            // Document errors
            IronBaseError::DocumentNotFound => McpError::document_not_found("unknown"),
            IronBaseError::InvalidDocumentId(msg) => McpError::validation(msg),
            IronBaseError::DocumentValidationFailed(msg) => McpError::validation(msg),
            IronBaseError::DuplicateKey(field, value) => McpError::validation(format!(
                "Duplicate key on field '{}': value '{}' already exists",
                field, value
            )),

            // Query/validation errors
            IronBaseError::InvalidQuery(msg) => McpError::invalid_params(msg),
            IronBaseError::UnsupportedOperator(op) => {
                McpError::invalid_params(format!("Unsupported operator: {}", op))
            }
            IronBaseError::QuerySyntaxError(msg) => McpError::invalid_params(msg),
            IronBaseError::QueryExecutionError(msg) => McpError::internal(msg),
            IronBaseError::InvalidProjection(msg) => McpError::invalid_params(msg),
            IronBaseError::InvalidSort(msg) => McpError::invalid_params(msg),
            IronBaseError::SchemaError(msg) => McpError::validation(msg),
            IronBaseError::SchemaNotFound(name) => {
                McpError::validation(format!("Schema not found for collection: {}", name))
            }
            IronBaseError::InvalidSchema(msg) => McpError::validation(msg),
            IronBaseError::OperationNotAllowed(msg) => McpError::invalid_params(msg),
            IronBaseError::ReadOnlyViolation(msg) => McpError::invalid_params(msg),

            // Index errors
            IronBaseError::IndexError(msg) => McpError::index_error(msg),
            IronBaseError::IndexNotFound(name) => {
                McpError::index_error(format!("Index '{}' not found", name))
            }
            IronBaseError::IndexExists(name) => {
                McpError::validation(format!("Index '{}' already exists", name))
            }
            IronBaseError::ProtectedFieldIndex(field) => {
                McpError::validation(format!("Cannot create index on protected field: {}", field))
            }
            IronBaseError::CompoundIndexPrefixMismatch { expected, actual } => {
                McpError::validation(format!(
                    "Compound index prefix mismatch: expected {} fields, got {}",
                    expected, actual
                ))
            }
            IronBaseError::FuzzyIndexError(msg) => McpError::index_error(msg),
            IronBaseError::FulltextIndexError(msg) => McpError::index_error(msg),
            IronBaseError::VectorIndexError(msg) => McpError::index_error(msg),
            IronBaseError::VectorDimensionMismatch { expected, actual } => {
                McpError::validation(format!(
                    "Vector dimension mismatch: expected {}, got {}",
                    expected, actual
                ))
            }

            // Aggregation errors
            IronBaseError::AggregationError(msg) => McpError::aggregation(msg),
            IronBaseError::InvalidPipelineStage(stage) => {
                McpError::invalid_params(format!("Invalid pipeline stage: {}", stage))
            }
            IronBaseError::InvalidAccumulator(acc) => {
                McpError::invalid_params(format!("Invalid accumulator: {}", acc))
            }
            IronBaseError::AggregationMemoryLimit(msg) => McpError::resource_exhausted(msg, None),
            IronBaseError::AggregationTimeout => McpError::timeout("Aggregation timed out"),

            // Transaction errors
            IronBaseError::TransactionCommitted => {
                McpError::transaction("Transaction already committed or aborted")
            }
            IronBaseError::TransactionAborted(msg) => McpError::transaction(msg),
            IronBaseError::TransactionNotActive => {
                McpError::transaction("Transaction is not active")
            }
            IronBaseError::TransactionConflict(msg) => McpError::transaction(msg),
            IronBaseError::TransactionDeadlock => McpError::transaction("Deadlock detected"),
            IronBaseError::NestedTransactionNotAllowed => {
                McpError::transaction("Nested transactions are not allowed")
            }

            // WAL errors
            IronBaseError::WALCorruption => McpError::storage("WAL corruption detected"),
            IronBaseError::WALWriteError(msg) => McpError::storage(msg),
            IronBaseError::WALReadError(msg) => McpError::storage(msg),
            IronBaseError::WALRecoveryFailed(msg) => McpError::storage(msg),
            IronBaseError::CheckpointFailed(msg) => McpError::storage(msg),

            // Resource errors
            IronBaseError::OutOfMemory(msg) => McpError::resource_exhausted(msg, None),
            IronBaseError::ResourceExhausted(msg) => McpError::resource_exhausted(msg, None),
            IronBaseError::DatabaseLocked(path) => {
                McpError::resource_exhausted(format!("Database '{}' is locked", path), None)
            }

            // Timeout/cancellation errors
            IronBaseError::Timeout(msg) => McpError::timeout(msg),
            IronBaseError::Cancelled(msg) => McpError::cancelled(msg),

            // Storage/corruption errors
            IronBaseError::Io(e) => McpError::storage(e.to_string()),
            IronBaseError::Serialization(msg) => McpError::serialization(msg),
            IronBaseError::Deserialization(e) => McpError::serialization(e.to_string()),
            IronBaseError::Bincode(e) => McpError::serialization(e.to_string()),
            IronBaseError::Corruption(msg) => McpError::storage(format!("Corruption: {}", msg)),
            IronBaseError::StorageError(msg) => McpError::storage(msg),
            IronBaseError::FileSystemError(msg) => McpError::storage(msg),
            IronBaseError::CompactionFailed(msg) => McpError::storage(msg),
            IronBaseError::DatabaseClosed => McpError::storage("Database is closed"),

            // Configuration errors
            IronBaseError::InvalidConfiguration(msg) => McpError::validation(msg),
            IronBaseError::ConfigFileError(msg) => McpError::validation(msg),
            IronBaseError::InvalidDurabilityMode(msg) => McpError::validation(msg),

            // Unknown/internal errors
            IronBaseError::Unknown(msg) => McpError::storage(msg),
            IronBaseError::InternalError(msg) => McpError::internal(msg),
        }
    }
}

impl From<serde_json::Error> for McpError {
    fn from(err: serde_json::Error) -> Self {
        McpError::serialization(err.to_string())
    }
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
}
