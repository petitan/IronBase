//! Batch constraint validation for unique indexes
//!
//! This module provides unified constraint checking for batch operations
//! (insert_many, update_many) that need to detect duplicates within the same batch.

use std::collections::{HashMap, HashSet};

use crate::error::{MongoLiteError, Result};
use crate::index::IndexManager;
use crate::value_utils::get_nested_value;
use serde_json::Value;

/// Validates unique constraints within a batch of documents
///
/// Tracks values that have been seen in the current batch operation
/// to detect duplicates BEFORE they reach the index.
///
/// # Problem Solved
///
/// Without this validator, batch operations could bypass unique index constraints:
/// - `insert_many([{email: "a@b.com"}, {email: "a@b.com"}])` would succeed
/// - `update_many({}, {$set: {email: "same@value.com"}})` would succeed
///
/// The B+ tree index only sees documents one at a time, so it can't detect
/// that two documents in the same batch have the same value.
///
/// # Usage
///
/// ```ignore
/// let mut validator = BatchConstraintValidator::new(&indexes, "collection_name");
/// for doc in documents {
///     validator.check_and_track(&doc_as_json)?;  // Returns Err if duplicate
/// }
/// ```
pub struct BatchConstraintValidator {
    /// Maps index_name -> Set of already-seen values (as JSON strings)
    pending_values: HashMap<String, HashSet<String>>,
    /// List of (field_name, index_name) for unique indexes
    unique_indexes: Vec<(String, String)>,
}

impl BatchConstraintValidator {
    /// Create a new validator by scanning the index manager for unique indexes
    ///
    /// Automatically excludes the `_id` index (which is always unique but
    /// handled separately during document ID generation).
    pub fn new(indexes: &IndexManager, _collection_name: &str) -> Self {
        let unique_indexes: Vec<(String, String)> = indexes
            .list_indexes()
            .into_iter()
            .filter_map(|name| {
                indexes.get_btree_index(&name).and_then(|idx| {
                    // Exclude _id field - handled separately during document ID generation
                    if idx.metadata.unique && idx.metadata.field != "_id" {
                        Some((idx.metadata.field.clone(), name))
                    } else {
                        None
                    }
                })
            })
            .collect();

        BatchConstraintValidator {
            pending_values: HashMap::new(),
            unique_indexes,
        }
    }

    /// Check if a document would create a duplicate within the current batch
    ///
    /// If the value is unique, it's added to the tracking set.
    /// If the value is a duplicate, returns an error immediately.
    ///
    /// # Arguments
    ///
    /// * `doc` - The document as a JSON Value (must be an Object)
    ///
    /// # Returns
    ///
    /// * `Ok(())` if the document is unique within the batch
    /// * `Err(MongoLiteError::IndexError)` if a duplicate is detected
    pub fn check_and_track(&mut self, doc: &Value) -> Result<()> {
        for (field, index_name) in &self.unique_indexes {
            // FIX #17: Use get_nested_value to support dot notation (e.g., "profile.code")
            // Previously used doc.get(field) which only works for top-level fields
            if let Some(field_value) = get_nested_value(doc, field) {
                let value_key = field_value.to_string();
                let seen_set = self.pending_values.entry(index_name.clone()).or_default();

                if !seen_set.insert(value_key.clone()) {
                    return Err(MongoLiteError::IndexError(format!(
                        "Duplicate key in batch: {} in field '{}' (unique index)",
                        value_key, field
                    )));
                }
            }
        }
        Ok(())
    }

    /// Returns true if there are no unique indexes to check
    ///
    /// Useful for early-exit optimization when no validation is needed.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.unique_indexes.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn create_test_index_manager_with_unique(collection_name: &str, field: &str) -> IndexManager {
        let mut manager = IndexManager::new();
        let index_name = format!("{}_{}", collection_name, field);
        manager
            .create_btree_index(index_name, field.to_string(), true)
            .unwrap();
        manager
    }

    #[test]
    fn test_empty_validator_accepts_all() {
        let manager = IndexManager::new();
        let mut validator = BatchConstraintValidator::new(&manager, "test");

        assert!(validator.is_empty());
        assert!(validator
            .check_and_track(&json!({"email": "a@b.com"}))
            .is_ok());
        assert!(validator
            .check_and_track(&json!({"email": "a@b.com"}))
            .is_ok());
    }

    #[test]
    fn test_detects_duplicate_in_batch() {
        let manager = create_test_index_manager_with_unique("test", "email");
        let mut validator = BatchConstraintValidator::new(&manager, "test");

        assert!(!validator.is_empty());
        assert!(validator
            .check_and_track(&json!({"email": "a@b.com"}))
            .is_ok());

        let result = validator.check_and_track(&json!({"email": "a@b.com"}));
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Duplicate key in batch"));
    }

    #[test]
    fn test_allows_different_values() {
        let manager = create_test_index_manager_with_unique("test", "email");
        let mut validator = BatchConstraintValidator::new(&manager, "test");

        assert!(validator
            .check_and_track(&json!({"email": "a@b.com"}))
            .is_ok());
        assert!(validator
            .check_and_track(&json!({"email": "b@c.com"}))
            .is_ok());
        assert!(validator
            .check_and_track(&json!({"email": "c@d.com"}))
            .is_ok());
    }

    #[test]
    fn test_ignores_documents_without_indexed_field() {
        let manager = create_test_index_manager_with_unique("test", "email");
        let mut validator = BatchConstraintValidator::new(&manager, "test");

        assert!(validator.check_and_track(&json!({"name": "Alice"})).is_ok());
        assert!(validator.check_and_track(&json!({"name": "Bob"})).is_ok());
    }

    #[test]
    fn test_multiple_unique_indexes() {
        let mut manager = IndexManager::new();
        manager
            .create_btree_index("test_email".to_string(), "email".to_string(), true)
            .unwrap();
        manager
            .create_btree_index("test_username".to_string(), "username".to_string(), true)
            .unwrap();

        let mut validator = BatchConstraintValidator::new(&manager, "test");

        // First document with both unique fields
        assert!(validator
            .check_and_track(&json!({"email": "a@b.com", "username": "alice"}))
            .is_ok());

        // Same email, different username - should fail
        let result = validator.check_and_track(&json!({"email": "a@b.com", "username": "bob"}));
        assert!(result.is_err());
    }

    #[test]
    fn test_excludes_id_index() {
        let mut manager = IndexManager::new();
        // Simulate the _id index that always exists
        manager
            .create_btree_index("users__id".to_string(), "_id".to_string(), true)
            .unwrap();
        manager
            .create_btree_index("users_email".to_string(), "email".to_string(), true)
            .unwrap();

        let validator = BatchConstraintValidator::new(&manager, "users");

        // Should only track 'email', not '_id'
        assert_eq!(validator.unique_indexes.len(), 1);
        assert_eq!(validator.unique_indexes[0].0, "email");
    }

    /// FIX #17: Test nested field unique index support (dot notation)
    #[test]
    fn test_detects_duplicate_in_nested_field() {
        // Create index on nested field "profile.code"
        let mut manager = IndexManager::new();
        manager
            .create_btree_index(
                "test_profile.code".to_string(),
                "profile.code".to_string(),
                true,
            )
            .unwrap();

        let mut validator = BatchConstraintValidator::new(&manager, "test");

        // First document with nested field
        assert!(validator
            .check_and_track(&json!({"name": "Alice", "profile": {"code": "CODE_A"}}))
            .is_ok());

        // Different nested value - should succeed
        assert!(validator
            .check_and_track(&json!({"name": "Bob", "profile": {"code": "CODE_B"}}))
            .is_ok());

        // Duplicate nested value - should fail
        let result =
            validator.check_and_track(&json!({"name": "Charlie", "profile": {"code": "CODE_A"}}));
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Duplicate key in batch"));
    }

    #[test]
    fn test_deeply_nested_field() {
        // Create index on deeply nested field "user.address.city"
        let mut manager = IndexManager::new();
        manager
            .create_btree_index(
                "test_user.address.city".to_string(),
                "user.address.city".to_string(),
                true,
            )
            .unwrap();

        let mut validator = BatchConstraintValidator::new(&manager, "test");

        assert!(validator
            .check_and_track(&json!({"user": {"address": {"city": "NYC"}}}))
            .is_ok());
        assert!(validator
            .check_and_track(&json!({"user": {"address": {"city": "LA"}}}))
            .is_ok());

        // Duplicate deeply nested value
        let result = validator.check_and_track(&json!({"user": {"address": {"city": "NYC"}}}));
        assert!(result.is_err());
    }
}
