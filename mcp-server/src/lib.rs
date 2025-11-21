// MCP DOCJL Server - AI-assisted document editing

pub mod adapters;
pub mod domain;
pub mod host;

// Re-export main types for convenience
pub use adapters::IronBaseAdapter;

pub use domain::{
    Block, BlockType, Document, DocumentOperations, DomainError, DomainResult,
    InsertOptions, InsertPosition, Label, LabelGenerator, MoveOptions, DeleteOptions,
    OperationResult, CrossReference, ReferenceValidator, SchemaValidator, ValidationResult,
    SearchQuery, OutlineItem, SearchResult, LabelChange, ChangeReason,
};

pub use host::{
    ApiKey, AuthError, AuthManager, AuditEntry, AuditLogger, AuditQuery, CommandResult,
    read_audit_log,
};

/// Library version
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Library name
pub const NAME: &str = env!("CARGO_PKG_NAME");
