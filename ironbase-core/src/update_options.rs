//! Update Options and Result types for upsert support
//!
//! Provides MongoDB-compatible upsert functionality:
//! - UpdateOptions: Configuration for update operations
//! - UpdateResult: Result containing matched/modified counts and upserted ID

use crate::document::DocumentId;

/// Options for update operations (MongoDB-compatible)
///
/// # Example
/// ```rust,ignore
/// use ironbase_core::UpdateOptions;
///
/// let options = UpdateOptions::new().with_upsert(true);
/// let result = db.update_one_with_options("users", &filter, &update, options)?;
/// ```
#[derive(Debug, Clone, Default)]
pub struct UpdateOptions {
    /// If true, insert a new document when no match is found
    pub upsert: bool,
}

impl UpdateOptions {
    /// Create new UpdateOptions with defaults (upsert=false)
    pub fn new() -> Self {
        Self::default()
    }

    /// Enable or disable upsert mode
    ///
    /// When upsert is true and no document matches the filter,
    /// a new document is created from the filter criteria and update.
    pub fn with_upsert(mut self, upsert: bool) -> Self {
        self.upsert = upsert;
        self
    }
}

/// Result of an update operation (MongoDB-compatible)
///
/// # Fields
/// - `matched_count`: Number of documents matching the filter
/// - `modified_count`: Number of documents actually modified
/// - `upserted_id`: ID of inserted document if upsert occurred
///
/// # Example
/// ```rust,ignore
/// let result = db.update_one_with_options("users", &filter, &update, options)?;
/// if let Some(id) = result.upserted_id {
///     println!("New document inserted with id: {:?}", id);
/// } else {
///     println!("Updated {} documents", result.modified_count);
/// }
/// ```
#[derive(Debug, Clone, Default)]
pub struct UpdateResult {
    /// Number of documents matching the filter
    pub matched_count: u64,
    /// Number of documents actually modified
    pub modified_count: u64,
    /// ID of the upserted document (if upsert occurred)
    pub upserted_id: Option<DocumentId>,
}

impl UpdateResult {
    /// Create a new UpdateResult for a standard update (no upsert)
    pub fn from_counts(matched: u64, modified: u64) -> Self {
        Self {
            matched_count: matched,
            modified_count: modified,
            upserted_id: None,
        }
    }

    /// Create a new UpdateResult for an upsert (insert occurred)
    pub fn from_upsert(doc_id: DocumentId) -> Self {
        Self {
            matched_count: 0,
            modified_count: 0,
            upserted_id: Some(doc_id),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_update_options_default() {
        let opts = UpdateOptions::default();
        assert!(!opts.upsert);
    }

    #[test]
    fn test_update_options_with_upsert() {
        let opts = UpdateOptions::new().with_upsert(true);
        assert!(opts.upsert);
    }

    #[test]
    fn test_update_result_from_counts() {
        let result = UpdateResult::from_counts(5, 3);
        assert_eq!(result.matched_count, 5);
        assert_eq!(result.modified_count, 3);
        assert!(result.upserted_id.is_none());
    }

    #[test]
    fn test_update_result_from_upsert() {
        let doc_id = DocumentId::Int(42);
        let result = UpdateResult::from_upsert(doc_id.clone());
        assert_eq!(result.matched_count, 0);
        assert_eq!(result.modified_count, 0);
        assert_eq!(result.upserted_id, Some(doc_id));
    }
}
