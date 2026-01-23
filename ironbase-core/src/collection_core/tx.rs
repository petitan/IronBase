//! Transaction operations for CollectionCore
//!
//! # Known Limitations
//!
//! ## Index Tracking Not Atomic
//!
//! The current implementation tracks index changes separately from document operations.
//! This means that in case of a crash during commit:
//!
//! 1. Document write may succeed while index update fails
//! 2. This can lead to index-document inconsistency
//! 3. After restart, `rebuild_indexes` may be needed to restore consistency
//!
//! **Future work:** Two-phase commit for atomic index updates (see INDEX_CONSISTENCY.md)
//!
//! ## Optimistic Concurrency
//!
//! Update and delete operations use optimistic concurrency:
//! - `find_one()` locates the document (snapshot taken)
//! - Changes are prepared based on that snapshot
//! - Conflict detection happens at commit time
//!
//! If another transaction modifies the same document between find and commit,
//! the behavior depends on the transaction manager's conflict resolution.

use serde_json::Value;
use std::collections::HashMap;

use crate::document::{Document, DocumentId};
use crate::error::{IronBaseError, Result};
use crate::storage::{RawStorage, Storage};

use super::CollectionCore;

impl<S: Storage + RawStorage> CollectionCore<S> {
    // ========== TRANSACTION OPERATIONS ==========

    /// Insert one document within a transaction
    ///
    /// Note: Index changes are tracked but not yet applied atomically.
    /// See INDEX_CONSISTENCY.md for future two-phase commit implementation.
    pub fn insert_one_tx(
        &self,
        doc: HashMap<String, Value>,
        tx: &mut crate::transaction::Transaction,
    ) -> Result<DocumentId> {
        use crate::transaction::Operation;

        // Generate document ID
        let mut storage = self.storage.write();
        let meta = storage
            .get_collection_meta_mut(&self.name)
            .ok_or_else(|| IronBaseError::CollectionNotFound(self.name.clone()))?;

        let doc_id = DocumentId::new_auto(meta.last_id);
        meta.last_id += 1;
        drop(storage); // Release lock early

        // Create document with _id and _collection
        let mut doc_with_id = doc.clone();
        doc_with_id.insert("_id".to_string(), serde_json::json!(doc_id.clone()));
        doc_with_id.insert("_collection".to_string(), Value::String(self.name.clone()));

        let doc_for_validation = Document::new(doc_id.clone(), doc_with_id.clone());
        self.validate_document(&doc_for_validation)?;

        // Add operation to transaction
        // Convert to Value for nested field access in index tracking
        let doc_value = serde_json::json!(doc_with_id);

        tx.add_operation(Operation::Insert {
            collection: self.name.clone(),
            doc_id: doc_id.clone(),
            doc: doc_value.clone(),
        })?;

        // Track index changes for two-phase commit
        let indexes = self.indexes.read();
        for index_name in indexes.list_indexes() {
            // Get the index to extract field name
            if let Some(btree_index) = indexes.get_btree_index(&index_name) {
                let field_name = &btree_index.metadata.field;

                // FIX #19: Use get_nested_value to support dot notation (e.g., "profile.code")
                if let Some(key_value) =
                    crate::value_utils::get_nested_value(&doc_value, field_name)
                {
                    let key = crate::transaction::IndexKey::from(key_value);
                    tx.add_index_change(
                        index_name.clone(),
                        crate::transaction::IndexChange {
                            operation: crate::transaction::IndexOperation::Insert,
                            key,
                            doc_id: doc_id.clone(),
                        },
                    )?;
                }
            }
        }

        Ok(doc_id)
    }

    /// Update one document within a transaction
    ///
    /// Note: Pass the new_doc directly (not update operators).
    /// Index changes are tracked but not yet applied atomically.
    /// See INDEX_CONSISTENCY.md for future two-phase commit implementation.
    pub fn update_one_tx(
        &self,
        query: &Value,
        new_doc: Value,
        tx: &mut crate::transaction::Transaction,
    ) -> Result<(u64, u64)> {
        use crate::transaction::Operation;

        // Find the document first
        let doc = self.find_one(query)?;

        if let Some(old_doc) = doc {
            // Extract document ID from _id field
            let id_value = old_doc.get("_id").ok_or(IronBaseError::DocumentNotFound)?;

            let doc_id = match id_value {
                Value::Number(n) if n.is_i64() => DocumentId::Int(n.as_i64().unwrap()),
                Value::Number(n) if n.is_u64() => {
                    let u = n.as_u64().unwrap();
                    if u > i64::MAX as u64 {
                        return Err(IronBaseError::Serialization(
                            "_id value too large for i64".to_string(),
                        ));
                    }
                    DocumentId::Int(u as i64)
                }
                Value::String(s) => DocumentId::String(s.clone()),
                _ => return Err(IronBaseError::Serialization("Invalid _id type".to_string())),
            };

            // Ensure new_doc has _id and _collection fields
            let new_doc_with_meta = if let Value::Object(mut map) = new_doc {
                map.insert("_id".to_string(), id_value.clone());
                map.insert("_collection".to_string(), Value::String(self.name.clone()));
                Value::Object(map)
            } else {
                return Err(IronBaseError::Serialization(
                    "new_doc must be an object".to_string(),
                ));
            };

            // Prepare new_doc for index tracking
            let new_doc_for_tracking = new_doc_with_meta.clone();
            self.validate_value_against_schema(&new_doc_for_tracking)?;

            // Add operation to transaction
            tx.add_operation(Operation::Update {
                collection: self.name.clone(),
                doc_id: doc_id.clone(),
                old_doc: old_doc.clone(),
                new_doc: new_doc_with_meta,
            })?;

            // Track index changes for two-phase commit
            let indexes = self.indexes.read();
            for index_name in indexes.list_indexes() {
                if let Some(btree_index) = indexes.get_btree_index(&index_name) {
                    let field_name = &btree_index.metadata.field;

                    // Get old and new values
                    // FIX #19: Use get_nested_value to support dot notation (e.g., "profile.code")
                    let old_value = crate::value_utils::get_nested_value(&old_doc, field_name);
                    let new_value =
                        crate::value_utils::get_nested_value(&new_doc_for_tracking, field_name);

                    // Delete old key if exists
                    if let Some(old_val) = old_value {
                        let old_key = crate::transaction::IndexKey::from(old_val);
                        tx.add_index_change(
                            index_name.clone(),
                            crate::transaction::IndexChange {
                                operation: crate::transaction::IndexOperation::Delete,
                                key: old_key,
                                doc_id: doc_id.clone(),
                            },
                        )?;
                    }

                    // Insert new key if exists
                    if let Some(new_val) = new_value {
                        let new_key = crate::transaction::IndexKey::from(new_val);
                        tx.add_index_change(
                            index_name.clone(),
                            crate::transaction::IndexChange {
                                operation: crate::transaction::IndexOperation::Insert,
                                key: new_key,
                                doc_id: doc_id.clone(),
                            },
                        )?;
                    }
                }
            }

            Ok((1, 1)) // matched_count, modified_count
        } else {
            Ok((0, 0))
        }
    }

    /// Delete one document within a transaction
    ///
    /// Note: Index changes are tracked but not yet applied atomically.
    /// See INDEX_CONSISTENCY.md for future two-phase commit implementation.
    pub fn delete_one_tx(
        &self,
        query: &Value,
        tx: &mut crate::transaction::Transaction,
    ) -> Result<u64> {
        use crate::transaction::Operation;

        // Find the document first
        let doc = self.find_one(query)?;

        if let Some(old_doc) = doc {
            // Extract document ID from _id field
            let id_value = old_doc.get("_id").ok_or(IronBaseError::DocumentNotFound)?;

            let doc_id = match id_value {
                Value::Number(n) if n.is_i64() => DocumentId::Int(n.as_i64().unwrap()),
                Value::Number(n) if n.is_u64() => {
                    let u = n.as_u64().unwrap();
                    if u > i64::MAX as u64 {
                        return Err(IronBaseError::Serialization(
                            "_id value too large for i64".to_string(),
                        ));
                    }
                    DocumentId::Int(u as i64)
                }
                Value::String(s) => DocumentId::String(s.clone()),
                _ => return Err(IronBaseError::Serialization("Invalid _id type".to_string())),
            };

            // Add operation to transaction
            tx.add_operation(Operation::Delete {
                collection: self.name.clone(),
                doc_id: doc_id.clone(),
                old_doc: old_doc.clone(),
            })?;

            // Track index changes for two-phase commit
            let indexes = self.indexes.read();
            for index_name in indexes.list_indexes() {
                if let Some(btree_index) = indexes.get_btree_index(&index_name) {
                    let field_name = &btree_index.metadata.field;

                    // Delete key from index if exists
                    // FIX #19: Use get_nested_value to support dot notation (e.g., "profile.code")
                    if let Some(old_val) =
                        crate::value_utils::get_nested_value(&old_doc, field_name)
                    {
                        let old_key = crate::transaction::IndexKey::from(old_val);
                        tx.add_index_change(
                            index_name.clone(),
                            crate::transaction::IndexChange {
                                operation: crate::transaction::IndexOperation::Delete,
                                key: old_key,
                                doc_id: doc_id.clone(),
                            },
                        )?;
                    }
                }
            }

            Ok(1) // deleted_count
        } else {
            Ok(0)
        }
    }
}
