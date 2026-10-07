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
//! ## Index Maintenance at Commit
//!
//! The in-memory indexes (all types) are updated at commit from the committed
//! operations by `apply_committed_ops_to_indexes`, the same add/remove path as
//! auto-commit writes (audit 2026-10-06 #11). `add_index_change` still records
//! B+ tree changes for the WAL; it is not what keeps the live indexes current.
//!
//! ## Isolation
//!
//! - A transaction holds the database write lock exclusively from its first
//!   write until commit/rollback, and auto-commit writes hold it shared, so the
//!   committed state a transaction reads cannot change under it (audit #14).
//! - Update and delete locate their target with `find_one_in_tx`, which sees
//!   the transaction's own buffered writes (audit #12).

use serde_json::Value;
use std::collections::HashMap;

use crate::document::{Document, DocumentId};
use crate::error::{IronBaseError, Result};
use crate::storage::{RawStorage, Storage};

use super::CollectionCore;

impl<S: Storage + RawStorage> CollectionCore<S> {
    // ========== TRANSACTION OPERATIONS ==========

    /// Extract DocumentId from a Value (typically from _id field)
    ///
    /// Handles Int, u64 (with overflow check), and String types.
    fn extract_doc_id_from_value(id_value: &Value) -> Result<DocumentId> {
        match id_value {
            Value::Number(n) if n.is_i64() => Ok(DocumentId::Int(n.as_i64().unwrap())),
            Value::Number(n) if n.is_u64() => {
                let u = n.as_u64().unwrap();
                if u > i64::MAX as u64 {
                    return Err(IronBaseError::Serialization(
                        "_id value too large for i64".to_string(),
                    ));
                }
                Ok(DocumentId::Int(u as i64))
            }
            Value::String(s) => Ok(DocumentId::String(s.clone())),
            _ => Err(IronBaseError::Serialization("Invalid _id type".to_string())),
        }
    }

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
        // Same unique-index check as the auto-commit insert (audit 2026-10-06 #13)
        self.check_index_constraints(&doc_for_validation, None)?;
        self.validate_document(&doc_for_validation)?;

        // Add operation to transaction
        // Convert to Value for nested field access in index tracking
        let doc_value = serde_json::json!(doc_with_id);

        tx.add_operation(Operation::Insert {
            collection: self.name.clone(),
            doc_id: doc_id.clone(),
            doc: std::sync::Arc::new(doc_value.clone()),
        })?;

        // Track index changes for two-phase commit
        //
        // TODO(N4): Only B+ tree indexes are tracked. Fulltext, fuzzy, and HNSW index
        // changes are NOT recorded in the transaction and will be lost on commit.
        // Fixing this requires extending IndexChange/IndexOperation to support:
        //   - Fulltext: tokenized text (not a single IndexKey)
        //   - Fuzzy: string value for similarity index
        //   - HNSW: vector embedding (f32 array)
        // Until then, non-btree indexes may become inconsistent after transaction
        // commit and require rebuild_indexes to restore consistency.
        let indexes = self.indexes.read();
        for index_name in indexes.list_indexes() {
            // Only B+ tree indexes support transactional tracking (IndexKey-based)
            if let Some(btree_index) = indexes.get_btree_index(&index_name) {
                let field_name = &btree_index.metadata.field;

                // FIX #19: Use get_nested_value to support dot notation (e.g., "profile.code")
                if let Some(key_value) =
                    crate::value_utils::get_nested_value(&doc_value, field_name)
                {
                    let key = crate::index::IndexKey::from(key_value);
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
    /// Applies update operators ($set, $inc, etc.) to the matched document,
    /// consistent with non-transactional `update_one`.
    /// Index changes are tracked but not yet applied atomically.
    /// See INDEX_CONSISTENCY.md for future two-phase commit implementation.
    pub fn update_one_tx(
        &self,
        query: &Value,
        update: Value,
        tx: &mut crate::transaction::Transaction,
    ) -> Result<(u64, u64)> {
        use crate::transaction::Operation;

        // Find the document as this transaction sees it
        let doc = self.find_one_in_tx(query, tx)?;

        if let Some(old_doc) = doc {
            // Extract document ID from _id field
            let id_value = old_doc.get("_id").ok_or(IronBaseError::DocumentNotFound)?;
            let doc_id = Self::extract_doc_id_from_value(id_value)?;

            // Build Document from old_doc Value, apply update operators
            // (mirrors raw_operations.rs:update_one_prepare logic)
            let mut document = Document::from_value(&old_doc)?;
            let was_modified =
                super::update_operators::apply_update_operators(&mut document, &update)?;

            if !was_modified {
                // Matched but nothing changed — return (1, 0)
                return Ok((1, 0));
            }

            // Validate BEFORE recording in the transaction
            self.check_index_constraints(&document, Some(&document.id))?;
            self.validate_document(&document)?;

            // Convert back to Value for the WAL
            let new_doc_value = serde_json::to_value(&document)
                .map_err(|e| IronBaseError::Serialization(e.to_string()))?;

            // Add operation to transaction
            tx.add_operation(Operation::Update {
                collection: self.name.clone(),
                doc_id: doc_id.clone(),
                old_doc: std::sync::Arc::new(old_doc.clone()),
                new_doc: std::sync::Arc::new(new_doc_value.clone()),
            })?;

            // Track index changes for two-phase commit
            // TODO(N4): Only B+ tree indexes are tracked — see insert_one_tx for details.
            let indexes = self.indexes.read();
            for index_name in indexes.list_indexes() {
                // Only B+ tree indexes support transactional tracking (IndexKey-based)
                if let Some(btree_index) = indexes.get_btree_index(&index_name) {
                    let field_name = &btree_index.metadata.field;

                    // Get old and new values
                    // FIX #19: Use get_nested_value to support dot notation (e.g., "profile.code")
                    let old_value = crate::value_utils::get_nested_value(&old_doc, field_name);
                    let new_value =
                        crate::value_utils::get_nested_value(&new_doc_value, field_name);

                    // Delete old key if exists
                    if let Some(old_val) = old_value {
                        let old_key = crate::index::IndexKey::from(old_val);
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
                        let new_key = crate::index::IndexKey::from(new_val);
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

        // Find the document as this transaction sees it
        let doc = self.find_one_in_tx(query, tx)?;

        if let Some(old_doc) = doc {
            // Extract document ID from _id field
            let id_value = old_doc.get("_id").ok_or(IronBaseError::DocumentNotFound)?;
            let doc_id = Self::extract_doc_id_from_value(id_value)?;

            // Add operation to transaction
            tx.add_operation(Operation::Delete {
                collection: self.name.clone(),
                doc_id: doc_id.clone(),
                old_doc: std::sync::Arc::new(old_doc.clone()),
            })?;

            // Track index changes for two-phase commit
            // TODO(N4): Only B+ tree indexes are tracked — see insert_one_tx for details.
            let indexes = self.indexes.read();
            for index_name in indexes.list_indexes() {
                // Only B+ tree indexes support transactional tracking (IndexKey-based)
                if let Some(btree_index) = indexes.get_btree_index(&index_name) {
                    let field_name = &btree_index.metadata.field;

                    // Delete key from index if exists
                    // FIX #19: Use get_nested_value to support dot notation (e.g., "profile.code")
                    if let Some(old_val) =
                        crate::value_utils::get_nested_value(&old_doc, field_name)
                    {
                        let old_key = crate::index::IndexKey::from(old_val);
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

    /// Find the first document matching `query` as `tx` sees it: committed
    /// storage overlaid with the transaction's own buffered inserts, updates
    /// and deletes. Without the overlay a transaction deleted the same document
    /// twice, resurrected a document it had deleted and lost repeated updates
    /// (audit 2026-10-06 #12).
    fn find_one_in_tx(
        &self,
        query: &Value,
        tx: &crate::transaction::Transaction,
    ) -> Result<Option<Value>> {
        use crate::transaction::Operation;

        // Latest buffered version of every document this transaction touched
        // in this collection (None = deleted), in first-touch order.
        let mut overlay: HashMap<DocumentId, Option<&Value>> = HashMap::new();
        let mut order: Vec<&DocumentId> = Vec::new();
        for op in tx.operations() {
            let (collection, doc_id, doc) = match op {
                Operation::Insert {
                    collection,
                    doc_id,
                    doc,
                } => (collection, doc_id, Some(doc.as_ref())),
                Operation::Update {
                    collection,
                    doc_id,
                    new_doc,
                    ..
                } => (collection, doc_id, Some(new_doc.as_ref())),
                Operation::Delete {
                    collection, doc_id, ..
                } => (collection, doc_id, None),
            };
            if collection != &self.name {
                continue;
            }
            if overlay.insert(doc_id.clone(), doc).is_none() {
                order.push(doc_id);
            }
        }
        if overlay.is_empty() {
            return self.find_one(query);
        }

        // A committed match the transaction has not touched. At most
        // overlay.len() of the first overlay.len() + 1 matches are touched.
        let options = crate::find_options::FindOptions::new().with_limit(overlay.len() + 1);
        for doc in self.find_with_options(query, options)? {
            let touched = match doc.get("_id") {
                Some(id) => {
                    overlay.contains_key(&serde_json::from_value::<DocumentId>(id.clone())?)
                }
                None => false,
            };
            if !touched {
                return Ok(Some(doc));
            }
        }

        // Otherwise the transaction's own version of a document, if it matches
        let parsed_query = crate::query::Query::from_json(query)?;
        for doc_id in order {
            if let Some(Some(doc)) = overlay.get(doc_id) {
                if parsed_query.matches(&Document::from_value(doc)?)? {
                    return Ok(Some((*doc).clone()));
                }
            }
        }
        Ok(None)
    }

    /// Apply the operations of a committed transaction to this collection's
    /// in-memory indexes (every index type), in commit order. Operations on
    /// other collections are skipped. Called by `DatabaseCore::commit_transaction`
    /// after the storage commit (audit 2026-10-06 #11: committed transactions
    /// never updated the indexes, so indexed queries missed committed data).
    pub(crate) fn apply_committed_ops_to_indexes(
        &self,
        operations: &[crate::transaction::Operation],
    ) -> Result<()> {
        use crate::transaction::Operation;

        // Build the document from the operation's doc_id: the stored JSON is
        // not required to carry `_id`. An old image that is not an object
        // (unknown) has nothing to remove.
        let as_document = |doc_id: &DocumentId, value: &Value| -> Option<Document> {
            value.as_object().map(|fields| {
                Document::new(
                    doc_id.clone(),
                    fields.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
                )
            })
        };

        // The storage commit is already durable: an index error must not turn
        // it into a reported failure, so it is logged and the rest applied.
        let report = |doc_id: &DocumentId, result: Result<()>| {
            if let Err(e) = result {
                crate::log_warn!(
                    "[WARN] Index update after commit failed for {:?} in '{}': {}",
                    doc_id,
                    self.name,
                    e
                );
            }
        };

        for op in operations {
            match op {
                Operation::Insert {
                    collection,
                    doc_id,
                    doc,
                } if collection == &self.name => {
                    if let Some(document) = as_document(doc_id, doc) {
                        report(doc_id, self.add_to_indexes(&document));
                    }
                }
                Operation::Update {
                    collection,
                    doc_id,
                    old_doc,
                    new_doc,
                } if collection == &self.name => {
                    if let Some(document) = as_document(doc_id, old_doc) {
                        report(doc_id, self.remove_from_indexes(&document));
                    }
                    if let Some(document) = as_document(doc_id, new_doc) {
                        report(doc_id, self.add_to_indexes(&document));
                    }
                }
                Operation::Delete {
                    collection,
                    doc_id,
                    old_doc,
                } if collection == &self.name => {
                    if let Some(document) = as_document(doc_id, old_doc) {
                        report(doc_id, self.remove_from_indexes(&document));
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
}
