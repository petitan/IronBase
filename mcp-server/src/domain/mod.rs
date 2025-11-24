// Domain API for DOCJL document operations
// This module provides high-level operations for manipulating DOCJL documents
// while maintaining structural integrity, label consistency, and cross-references.

pub mod block;
pub mod block_new_types;
pub mod document;
pub mod label;
pub mod reference;
pub mod validation;

pub use block::{Block, BlockType, InlineContent};
pub use document::{Document, DocumentMetadata};
pub use label::{Label, LabelGenerator, LabelRenumberer};
pub use reference::{CrossReference, ReferenceValidator};
pub use validation::{SchemaValidator, ValidationResult};

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Result type for domain operations
pub type DomainResult<T> = Result<T, DomainError>;

/// Errors that can occur during domain operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DomainError {
    /// Block not found with given label
    BlockNotFound { label: String },

    /// Parent block not found
    ParentNotFound { label: String },

    /// Duplicate label detected
    DuplicateLabel { label: String },

    /// Invalid label format
    InvalidLabel { label: String, reason: String },

    /// Cross-reference target does not exist
    BrokenReference { source: String, target: String },

    /// Schema validation failed
    ValidationFailed { errors: Vec<String> },

    /// Operation would create invalid structure
    InvalidOperation { reason: String },

    /// Circular reference detected
    CircularReference { path: Vec<String> },

    /// Storage layer error
    StorageError { message: String },
}

impl std::fmt::Display for DomainError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DomainError::BlockNotFound { label } => {
                write!(f, "Block not found: {}", label)
            }
            DomainError::ParentNotFound { label } => {
                write!(f, "Parent block not found: {}", label)
            }
            DomainError::DuplicateLabel { label } => {
                write!(f, "Duplicate label: {}", label)
            }
            DomainError::InvalidLabel { label, reason } => {
                write!(f, "Invalid label '{}': {}", label, reason)
            }
            DomainError::BrokenReference { source, target } => {
                write!(f, "Broken reference from {} to {}", source, target)
            }
            DomainError::ValidationFailed { errors } => {
                write!(f, "Validation failed: {}", errors.join(", "))
            }
            DomainError::InvalidOperation { reason } => {
                write!(f, "Invalid operation: {}", reason)
            }
            DomainError::CircularReference { path } => {
                write!(f, "Circular reference: {}", path.join(" -> "))
            }
            DomainError::StorageError { message } => {
                write!(f, "Storage error: {}", message)
            }
        }
    }
}

impl std::error::Error for DomainError {}

/// Position for inserting blocks
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InsertPosition {
    /// Before the anchor block
    Before,
    /// After the anchor block
    After,
    /// As first child inside the anchor block
    Inside,
    /// As last child inside the anchor block
    End,
}

/// Options for block insertion
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InsertOptions {
    /// Parent block label (where to insert)
    pub parent_label: Option<String>,

    /// Position relative to anchor
    pub position: InsertPosition,

    /// Anchor block label (for Before/After positioning)
    pub anchor_label: Option<String>,

    /// Auto-generate label if not provided
    pub auto_label: bool,

    /// Validate schema before insertion
    pub validate: bool,
}

impl Default for InsertOptions {
    fn default() -> Self {
        Self {
            parent_label: None,
            position: InsertPosition::End,
            anchor_label: None,
            auto_label: true,
            validate: true,
        }
    }
}

/// Options for block movement
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoveOptions {
    /// Target parent label
    pub target_parent: Option<String>,

    /// Position in new parent
    pub position: InsertPosition,

    /// Update all cross-references
    pub update_references: bool,

    /// Renumber labels if needed
    pub renumber_labels: bool,
}

impl Default for MoveOptions {
    fn default() -> Self {
        Self {
            target_parent: None,
            position: InsertPosition::End,
            update_references: true,
            renumber_labels: true,
        }
    }
}

/// Options for block deletion
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteOptions {
    /// Delete all children recursively
    pub cascade: bool,

    /// Check for cross-references before deletion
    pub check_references: bool,

    /// Force deletion even if references exist
    pub force: bool,
}

impl Default for DeleteOptions {
    fn default() -> Self {
        Self {
            cascade: false,
            check_references: true,
            force: false,
        }
    }
}

/// Result of a block operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationResult {
    /// Operation succeeded
    pub success: bool,

    /// Generated audit ID
    pub audit_id: String,

    /// Affected block labels (for undo/redo)
    pub affected_labels: Vec<LabelChange>,

    /// Warnings (non-fatal issues)
    pub warnings: Vec<String>,
}

/// Label change during an operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LabelChange {
    pub old_label: String,
    pub new_label: String,
    pub reason: ChangeReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChangeReason {
    Moved,
    Renumbered,
    Generated,
}

/// Document operations interface
pub trait DocumentOperations {
    /// Insert a new block
    fn insert_block(
        &mut self,
        document_id: &str,
        block: Block,
        options: InsertOptions,
    ) -> DomainResult<OperationResult>;

    /// Update an existing block
    fn update_block(
        &mut self,
        document_id: &str,
        block_label: &str,
        updates: HashMap<String, serde_json::Value>,
    ) -> DomainResult<OperationResult>;

    /// Move a block to a new location
    fn move_block(
        &mut self,
        document_id: &str,
        block_label: &str,
        options: MoveOptions,
    ) -> DomainResult<OperationResult>;

    /// Delete a block
    fn delete_block(
        &mut self,
        document_id: &str,
        block_label: &str,
        options: DeleteOptions,
    ) -> DomainResult<OperationResult>;

    /// Get document outline (headings tree)
    fn get_outline(
        &self,
        document_id: &str,
        max_depth: Option<usize>,
    ) -> DomainResult<Vec<OutlineItem>>;

    /// Search for blocks
    fn search_blocks(
        &self,
        document_id: &str,
        query: SearchQuery,
    ) -> DomainResult<Vec<SearchResult>>;

    /// Validate all cross-references
    fn validate_references(
        &self,
        document_id: &str,
    ) -> DomainResult<ValidationResult>;

    /// Validate document schema
    fn validate_schema(
        &self,
        document_id: &str,
    ) -> DomainResult<ValidationResult>;
}

/// Outline item (for table of contents)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutlineItem {
    pub level: u8,
    pub label: String,
    pub title: String,
    pub children: Vec<OutlineItem>,
}

/// Search query for finding blocks
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchQuery {
    pub block_type: Option<BlockType>,
    pub content_contains: Option<String>,
    pub has_label: Option<bool>,
    pub has_compliance_note: Option<bool>,
    pub label: Option<String>,  // Exact label match
    pub label_prefix: Option<String>,
}

/// Search result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub label: String,
    pub block: Block,
    pub path: Vec<String>,  // Full path from root
    pub score: f32,         // Relevance score
}
