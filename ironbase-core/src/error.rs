// ironbase-core/src/error.rs
// Centralized error handling for the entire IronBase project

use thiserror::Error;

// ============================================================================
// Main Error Enum - Central error type for all IronBase operations
// ============================================================================

#[derive(Error, Debug)]
pub enum IronBaseError {
    // ========================================================================
    // I/O and System Errors
    // ========================================================================
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Database '{0}' is locked by another process")]
    DatabaseLocked(String),

    #[error("Database is closed")]
    DatabaseClosed,

    #[error("Out of memory: {0}")]
    OutOfMemory(String),

    #[error("Operation timed out: {0}")]
    Timeout(String),

    #[error("Operation cancelled: {0}")]
    Cancelled(String),

    // ========================================================================
    // Serialization Errors
    // ========================================================================
    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Deserialization error: {0}")]
    Deserialization(#[from] serde_json::Error),

    #[error("Bincode error: {0}")]
    Bincode(#[from] bincode::Error),

    // ========================================================================
    // Collection Errors
    // ========================================================================
    #[error("Collection '{0}' not found")]
    CollectionNotFound(String),

    #[error("Collection '{0}' already exists")]
    CollectionExists(String),

    #[error("Invalid collection name: {0}")]
    InvalidCollectionName(String),

    #[error("System collection operation not allowed: {0}")]
    SystemCollectionError(String),

    // ========================================================================
    // Document Errors
    // ========================================================================
    #[error("Document not found")]
    DocumentNotFound,

    #[error("Invalid document ID: {0}")]
    InvalidDocumentId(String),

    #[error("Document validation failed: {0}")]
    DocumentValidationFailed(String),

    #[error("Duplicate key error on field '{0}': value '{1}' already exists")]
    DuplicateKey(String, String),

    // ========================================================================
    // Query Errors
    // ========================================================================
    #[error("Invalid query: {0}")]
    InvalidQuery(String),

    #[error("Query operator '{0}' is not supported")]
    UnsupportedOperator(String),

    #[error("Invalid query syntax: {0}")]
    QuerySyntaxError(String),

    #[error("Query execution error: {0}")]
    QueryExecutionError(String),

    #[error("Invalid projection: {0}")]
    InvalidProjection(String),

    #[error("Invalid sort specification: {0}")]
    InvalidSort(String),

    // ========================================================================
    // Index Errors
    // ========================================================================
    #[error("Index error: {0}")]
    IndexError(String),

    #[error("Index '{0}' not found")]
    IndexNotFound(String),

    #[error("Index '{0}' already exists")]
    IndexExists(String),

    #[error("Cannot create index on protected field: {0}")]
    ProtectedFieldIndex(String),

    #[error("Compound index prefix mismatch: expected {expected} fields, got {actual}")]
    CompoundIndexPrefixMismatch { expected: usize, actual: usize },

    #[error("Fuzzy index error: {0}")]
    FuzzyIndexError(String),

    #[error("Fulltext index error: {0}")]
    FulltextIndexError(String),

    #[error("Vector index error: {0}")]
    VectorIndexError(String),

    #[error("Vector dimension mismatch: expected {expected}, got {actual}")]
    VectorDimensionMismatch { expected: usize, actual: usize },

    // ========================================================================
    // Aggregation Errors
    // ========================================================================
    #[error("Aggregation error: {0}")]
    AggregationError(String),

    #[error("Invalid pipeline stage: {0}")]
    InvalidPipelineStage(String),

    #[error("Invalid accumulator: {0}")]
    InvalidAccumulator(String),

    #[error("Aggregation memory limit exceeded: {0}")]
    AggregationMemoryLimit(String),

    #[error("Aggregation timeout")]
    AggregationTimeout,

    // ========================================================================
    // Transaction Errors
    // ========================================================================
    #[error("Transaction already committed or aborted")]
    TransactionCommitted,

    #[error("Transaction aborted: {0}")]
    TransactionAborted(String),

    #[error("Transaction not active")]
    TransactionNotActive,

    #[error("Transaction conflict: {0}")]
    TransactionConflict(String),

    #[error("Transaction deadlock detected")]
    TransactionDeadlock,

    #[error("Cannot nest transactions")]
    NestedTransactionNotAllowed,

    // ========================================================================
    // WAL (Write-Ahead Log) Errors
    // ========================================================================
    #[error("WAL corruption detected")]
    WALCorruption,

    #[error("WAL write error: {0}")]
    WALWriteError(String),

    #[error("WAL read error: {0}")]
    WALReadError(String),

    #[error("WAL recovery failed: {0}")]
    WALRecoveryFailed(String),

    #[error("Checkpoint failed: {0}")]
    CheckpointFailed(String),

    // ========================================================================
    // Storage Errors
    // ========================================================================
    #[error("Database corruption: {0}")]
    Corruption(String),

    #[error("Storage error: {0}")]
    StorageError(String),

    #[error("File system error: {0}")]
    FileSystemError(String),

    #[error("Compaction failed: {0}")]
    CompactionFailed(String),

    // ========================================================================
    // Schema and Validation Errors
    // ========================================================================
    #[error("Schema validation error: {0}")]
    SchemaError(String),

    #[error("Schema not found for collection: {0}")]
    SchemaNotFound(String),

    #[error("Invalid schema: {0}")]
    InvalidSchema(String),

    // ========================================================================
    // Operation Errors
    // ========================================================================
    #[error("Operation not allowed: {0}")]
    OperationNotAllowed(String),

    #[error("Read-only operation attempted on write-protected resource: {0}")]
    ReadOnlyViolation(String),

    #[error("Resource exhausted: {0}")]
    ResourceExhausted(String),

    // ========================================================================
    // Configuration Errors
    // ========================================================================
    #[error("Invalid configuration: {0}")]
    InvalidConfiguration(String),

    #[error("Configuration file error: {0}")]
    ConfigFileError(String),

    #[error("Invalid durability mode: {0}")]
    InvalidDurabilityMode(String),

    // ========================================================================
    // Unknown/Catch-all Errors
    // ========================================================================
    #[error("Unknown error: {0}")]
    Unknown(String),

    #[error("Internal error: {0}")]
    InternalError(String),
}

// ============================================================================
// Result Type Alias
// ============================================================================

pub type Result<T> = std::result::Result<T, IronBaseError>;

// ============================================================================
// Error Conversion Implementations
// ============================================================================

// Conversion from &str for easy error creation
impl From<&str> for IronBaseError {
    fn from(s: &str) -> Self {
        IronBaseError::Unknown(s.to_string())
    }
}

// Conversion from String for easy error creation
impl From<String> for IronBaseError {
    fn from(s: String) -> Self {
        IronBaseError::Unknown(s)
    }
}

// Conversion from Box<dyn std::error::Error> for external errors
impl From<Box<dyn std::error::Error + Send + Sync>> for IronBaseError {
    fn from(err: Box<dyn std::error::Error + Send + Sync>) -> Self {
        IronBaseError::Unknown(err.to_string())
    }
}

// ============================================================================
// Error Category Helpers
// ============================================================================

impl IronBaseError {
    /// Returns true if this error is retryable
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            IronBaseError::DatabaseLocked(_)
                | IronBaseError::TransactionConflict(_)
                | IronBaseError::TransactionDeadlock
                | IronBaseError::Timeout(_)
                | IronBaseError::ResourceExhausted(_)
        )
    }

    /// Returns true if this error indicates data corruption
    pub fn is_corruption(&self) -> bool {
        matches!(
            self,
            IronBaseError::Corruption(_)
                | IronBaseError::WALCorruption
                | IronBaseError::DatabaseClosed
        )
    }

    /// Returns true if this error is related to transactions
    pub fn is_transaction_error(&self) -> bool {
        matches!(
            self,
            IronBaseError::TransactionCommitted
                | IronBaseError::TransactionAborted(_)
                | IronBaseError::TransactionNotActive
                | IronBaseError::TransactionConflict(_)
                | IronBaseError::TransactionDeadlock
                | IronBaseError::NestedTransactionNotAllowed
        )
    }

    /// Returns true if this error indicates a not-found condition
    pub fn is_not_found(&self) -> bool {
        matches!(
            self,
            IronBaseError::CollectionNotFound(_)
                | IronBaseError::DocumentNotFound
                | IronBaseError::IndexNotFound(_)
                | IronBaseError::SchemaNotFound(_)
        )
    }

    /// Returns the error category as a string for logging/monitoring
    pub fn category(&self) -> &'static str {
        match self {
            IronBaseError::Io(_) => "io",
            IronBaseError::DatabaseLocked(_) => "locking",
            IronBaseError::DatabaseClosed => "lifecycle",
            IronBaseError::OutOfMemory(_) => "memory",
            IronBaseError::Timeout(_) => "timeout",
            IronBaseError::Cancelled(_) => "cancelled",
            IronBaseError::Serialization(_)
            | IronBaseError::Deserialization(_)
            | IronBaseError::Bincode(_) => "serialization",
            IronBaseError::CollectionNotFound(_)
            | IronBaseError::CollectionExists(_)
            | IronBaseError::InvalidCollectionName(_)
            | IronBaseError::SystemCollectionError(_) => "collection",
            IronBaseError::DocumentNotFound
            | IronBaseError::InvalidDocumentId(_)
            | IronBaseError::DocumentValidationFailed(_)
            | IronBaseError::DuplicateKey(_, _) => "document",
            IronBaseError::InvalidQuery(_)
            | IronBaseError::UnsupportedOperator(_)
            | IronBaseError::QuerySyntaxError(_)
            | IronBaseError::QueryExecutionError(_)
            | IronBaseError::InvalidProjection(_)
            | IronBaseError::InvalidSort(_) => "query",
            IronBaseError::IndexError(_)
            | IronBaseError::IndexNotFound(_)
            | IronBaseError::IndexExists(_)
            | IronBaseError::ProtectedFieldIndex(_)
            | IronBaseError::CompoundIndexPrefixMismatch { .. }
            | IronBaseError::FuzzyIndexError(_)
            | IronBaseError::FulltextIndexError(_)
            | IronBaseError::VectorIndexError(_)
            | IronBaseError::VectorDimensionMismatch { .. } => "index",
            IronBaseError::AggregationError(_)
            | IronBaseError::InvalidPipelineStage(_)
            | IronBaseError::InvalidAccumulator(_)
            | IronBaseError::AggregationMemoryLimit(_)
            | IronBaseError::AggregationTimeout => "aggregation",
            IronBaseError::TransactionCommitted
            | IronBaseError::TransactionAborted(_)
            | IronBaseError::TransactionNotActive
            | IronBaseError::TransactionConflict(_)
            | IronBaseError::TransactionDeadlock
            | IronBaseError::NestedTransactionNotAllowed => "transaction",
            IronBaseError::WALCorruption
            | IronBaseError::WALWriteError(_)
            | IronBaseError::WALReadError(_)
            | IronBaseError::WALRecoveryFailed(_)
            | IronBaseError::CheckpointFailed(_) => "wal",
            IronBaseError::Corruption(_)
            | IronBaseError::StorageError(_)
            | IronBaseError::FileSystemError(_)
            | IronBaseError::CompactionFailed(_) => "storage",
            IronBaseError::SchemaError(_)
            | IronBaseError::SchemaNotFound(_)
            | IronBaseError::InvalidSchema(_) => "schema",
            IronBaseError::OperationNotAllowed(_)
            | IronBaseError::ReadOnlyViolation(_)
            | IronBaseError::ResourceExhausted(_) => "operation",
            IronBaseError::InvalidConfiguration(_)
            | IronBaseError::ConfigFileError(_)
            | IronBaseError::InvalidDurabilityMode(_) => "configuration",
            IronBaseError::Unknown(_) | IronBaseError::InternalError(_) => "unknown",
        }
    }
}
