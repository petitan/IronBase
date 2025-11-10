// src/error.rs
use thiserror::Error;

#[derive(Error, Debug)]
pub enum MongoLiteError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    
    #[error("Serialization error: {0}")]
    Serialization(String),
    
    #[error("Deserialization error: {0}")]
    Deserialization(#[from] serde_json::Error),

    #[error("Collection '{0}' not found")]
    CollectionNotFound(String),
    
    #[error("Collection '{0}' already exists")]
    CollectionExists(String),
    
    #[error("Document not found")]
    DocumentNotFound,
    
    #[error("Invalid query: {0}")]
    InvalidQuery(String),
    
    #[error("Database corruption: {0}")]
    Corruption(String),
    
    #[error("Index error: {0}")]
    IndexError(String),

    #[error("Aggregation error: {0}")]
    AggregationError(String),

    #[error("Transaction already committed or aborted")]
    TransactionCommitted,

    #[error("Transaction aborted: {0}")]
    TransactionAborted(String),

    #[error("WAL corruption detected")]
    WALCorruption,

    #[error("Unknown error: {0}")]
    Unknown(String),
}

pub type Result<T> = std::result::Result<T, MongoLiteError>;
