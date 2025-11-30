//! INTERNAL RAW OPERATIONS - SEALED TRAIT
//!
//! # WARNING: DO NOT MAKE THIS MODULE OR TRAIT PUBLIC
//!
//! These operations bypass WAL durability guarantees.
//! They exist only for internal use by DatabaseCore which handles WAL.
//!
//! The sealed trait pattern prevents:
//! 1. External crates from implementing this trait
//! 2. Accidental exposure of unsafe operations
//! 3. "Simplification" by making methods public
//!
//! If you need write operations, use DatabaseCore::insert_one(), etc.

use std::collections::HashMap;

use serde_json::Value;

use crate::document::{Document, DocumentId};
use crate::error::{MongoLiteError, Result};
use crate::query::Query;
use crate::storage::{RawStorage, Storage};

use super::{CollectionCore, InsertManyResult};

/// Private module that seals the trait
mod sealed {
    use crate::storage::{RawStorage, Storage};

    /// Marker trait that cannot be implemented outside this module
    pub trait Sealed {}

    // Only CollectionCore can implement Sealed
    impl<S: Storage + RawStorage> Sealed for super::super::CollectionCore<S> {}
}

/// Raw CRUD operations that bypass WAL
///
/// # SEALED TRAIT - CANNOT BE IMPLEMENTED EXTERNALLY
///
/// This trait uses the sealed trait pattern to prevent:
/// - External implementation
/// - Accidental public exposure
///
/// Use DatabaseCore methods for safe, durable operations.
pub(crate) trait RawOperations: sealed::Sealed {
    /// Insert one document WITHOUT WAL protection
    ///
    /// # Warning
    /// This bypasses durability. Use `DatabaseCore::insert_one()` instead.
    fn insert_one_raw(&self, fields: HashMap<String, Value>) -> Result<DocumentId>;

    /// Insert many documents WITHOUT WAL protection
    fn insert_many_raw(&self, documents: Vec<HashMap<String, Value>>) -> Result<InsertManyResult>;

    /// Update one document WITHOUT WAL protection
    fn update_one_raw(&self, query: &Value, update: &Value) -> Result<(u64, u64)>;

    /// Update many documents WITHOUT WAL protection
    fn update_many_raw(&self, query: &Value, update: &Value) -> Result<(u64, u64)>;

    /// Delete one document WITHOUT WAL protection
    fn delete_one_raw(&self, query: &Value) -> Result<u64>;

    /// Delete many documents WITHOUT WAL protection
    fn delete_many_raw(&self, query: &Value) -> Result<u64>;
}

// ============================================================================
// TRAIT IMPLEMENTATION
// ============================================================================

impl<S: Storage + RawStorage> RawOperations for CollectionCore<S> {
    /// Insert one document (raw, no WAL) - use DatabaseCore::insert_one for durability
    /// For batch operations, use DurabilityMode::Batch
    fn insert_one_raw(&self, mut fields: HashMap<String, Value>) -> Result<DocumentId> {
        let mut storage = self.storage.write();

        // Get mutable reference to collection metadata
        let meta = storage
            .get_collection_meta_mut(&self.name)
            .ok_or_else(|| MongoLiteError::CollectionNotFound(self.name.clone()))?;

        // Check if _id already exists in fields
        let doc_id = if let Some(existing_id) = fields.get("_id") {
            // Use existing _id from fields
            let parsed_id: DocumentId = serde_json::from_value(existing_id.clone())
                .map_err(|e| MongoLiteError::Serialization(format!("Invalid _id format: {}", e)))?;

            // Ensure last_id tracks the highest numeric _id to avoid auto-ID collisions
            if let DocumentId::Int(num) = parsed_id {
                if num >= 0 {
                    let numeric = num as u64;
                    if numeric > meta.last_id {
                        meta.last_id = numeric;
                    }
                }
            }

            parsed_id
        } else {
            // Auto-generate new _id
            let new_id = DocumentId::new_auto(meta.last_id);
            meta.last_id += 1;

            // Add _id to fields for query matching
            fields.insert("_id".to_string(), serde_json::to_value(&new_id).unwrap());
            new_id
        };

        // Add _collection field for multi-collection isolation
        fields.insert("_collection".to_string(), Value::String(self.name.clone()));

        // Dokumentum létrehozása
        let doc = Document::new(doc_id.clone(), fields);
        self.validate_document(&doc)?;

        // Update indexes BEFORE writing to storage
        self.add_to_indexes(&doc)?;

        // Szerializálás és írás - USE NEW write_document with catalog tracking
        let doc_json = doc.to_json()?;
        storage.write_document_raw(&self.name, &doc_id, doc_json.as_bytes())?;
        storage.adjust_live_count(&self.name, 1);

        // NOTE: We don't flush metadata here for performance!
        // Catalog changes are kept in memory and flushed on:
        // - Database close
        // - Explicit flush
        // - Before compaction
        // This prevents O(n) metadata rewrites on every insert

        // Invalidate query cache (collection has changed)
        self.query_cache.invalidate_collection(&self.name);

        Ok(doc_id)
    }

    /// Insert many documents (raw, no WAL) - use DatabaseCore::insert_many for durability
    /// For batch operations, use DurabilityMode::Batch
    fn insert_many_raw(&self, documents: Vec<HashMap<String, Value>>) -> Result<InsertManyResult> {
        if documents.is_empty() {
            return Ok(InsertManyResult {
                inserted_ids: Vec::new(),
                inserted_count: 0,
            });
        }

        let mut storage = self.storage.write();
        let mut inserted_ids = Vec::with_capacity(documents.len());
        let mut live_delta = 0i64;

        // Get mutable reference to collection metadata ONCE
        let meta = storage
            .get_collection_meta_mut(&self.name)
            .ok_or_else(|| MongoLiteError::CollectionNotFound(self.name.clone()))?;

        // Get starting ID for auto-generation (don't pre-reserve)
        let start_id = meta.last_id;

        // Prepare all documents with IDs
        let mut prepared_docs = Vec::with_capacity(documents.len());
        let mut auto_id_count = 0u64;
        for mut fields in documents.into_iter() {
            // Check if _id already exists in fields (same logic as insert_one)
            let doc_id = if let Some(existing_id) = fields.get("_id") {
                // Use existing _id from fields - MongoDB compatible behavior
                let parsed_id: DocumentId =
                    serde_json::from_value(existing_id.clone()).map_err(|e| {
                        MongoLiteError::Serialization(format!("Invalid _id format: {}", e))
                    })?;

                // Ensure last_id tracks highest numeric _id from manual inserts
                if let DocumentId::Int(num) = parsed_id {
                    if num >= 0 {
                        let numeric = num as u64;
                        if numeric > meta.last_id {
                            meta.last_id = numeric;
                        }
                    }
                }

                parsed_id
            } else {
                // Auto-generate new _id only if not provided
                let new_id = DocumentId::new_auto(start_id + auto_id_count);
                auto_id_count += 1;
                fields.insert("_id".to_string(), serde_json::to_value(&new_id).unwrap());
                new_id
            };

            // Add _collection field
            fields.insert("_collection".to_string(), Value::String(self.name.clone()));

            // Create document
            let doc = Document::new(doc_id.clone(), fields);
            self.validate_document(&doc)?;
            prepared_docs.push((doc_id.clone(), doc));
            inserted_ids.push(doc_id);
        }

        // Update last_id with max of manual + auto-generated IDs
        meta.last_id = meta.last_id.max(start_id + auto_id_count);

        // Update indexes in batch BEFORE writing to storage
        let docs_for_index: Vec<Document> =
            prepared_docs.iter().map(|(_, doc)| doc.clone()).collect();
        self.batch_add_to_indexes(&docs_for_index)?;

        // Write all documents to storage
        for (doc_id, doc) in prepared_docs {
            let doc_json = doc.to_json()?;
            storage.write_document_raw(&self.name, &doc_id, doc_json.as_bytes())?;
            live_delta += 1;
        }

        // NOTE: We don't flush metadata here for performance!
        // Catalog changes are kept in memory and flushed on database close

        // Invalidate query cache (collection has changed)
        self.query_cache.invalidate_collection(&self.name);
        if live_delta != 0 {
            storage.adjust_live_count(&self.name, live_delta);
        }

        Ok(InsertManyResult {
            inserted_count: inserted_ids.len(),
            inserted_ids,
        })
    }

    /// Update one document (raw, no WAL) - use DatabaseCore::update_one for durability
    /// Returns (matched_count, modified_count)
    fn update_one_raw(&self, query_json: &Value, update_json: &Value) -> Result<(u64, u64)> {
        let parsed_query = Query::from_json(query_json)?;

        // OPTIMIZATION: Check if this is an _id equality query (O(1) lookup)
        let docs_by_id = if let Some(query_obj) = query_json.as_object() {
            if query_obj.len() == 1 && query_obj.contains_key("_id") {
                if let Some(id_val) = query_obj.get("_id") {
                    // Direct O(1) lookup using document_catalog (direct DocumentId conversion!)
                    if let Ok(doc_id) = serde_json::from_value::<DocumentId>(id_val.clone()) {
                        if let Some(doc) = self.read_document_by_id(&doc_id)? {
                            let mut single_doc_map = HashMap::new();
                            single_doc_map.insert(doc_id, doc);
                            single_doc_map
                        } else {
                            HashMap::new()
                        }
                    } else {
                        HashMap::new()
                    }
                } else {
                    self.scan_documents_via_catalog()?
                }
            } else {
                // Fallback: Full scan using catalog iteration
                self.scan_documents_via_catalog()?
            }
        } else {
            self.scan_documents_via_catalog()?
        };

        // Find first matching and update (skip tombstones already filtered by catalog scan)
        let mut matched = 0u64;
        let mut modified = 0u64;
        let mut storage = self.storage.write();

        for (_, doc) in docs_by_id {
            if matched > 0 {
                break; // Only update first match
            }

            let doc_json_str = serde_json::to_string(&doc)?;
            let mut document = Document::from_json(&doc_json_str)?;

            // Check if matches query
            if parsed_query.matches(&document) {
                matched = 1;

                // Save original document for index removal
                let original_document = document.clone();

                // Apply update operators
                let was_modified = self.apply_update_operators(&mut document, update_json)?;

                if was_modified {
                    // ✅ Ensure updated document has _collection before constraint check
                    document.set("_collection".to_string(), Value::String(self.name.clone()));

                    // 🔒 CHECK UNIQUE CONSTRAINTS BEFORE ANY CHANGES
                    // exclude_id = Some to allow updating same document's non-key fields
                    self.check_index_constraints(&document, Some(&document.id))?;

                    // Release storage lock for index operations
                    drop(storage);

                    // 📤 REMOVE OLD DOCUMENT FROM INDEXES
                    self.remove_from_indexes(&original_document)?;

                    // 📥 ADD UPDATED DOCUMENT TO INDEXES
                    self.add_to_indexes(&document)?;

                    // Re-acquire storage lock
                    storage = self.storage.write();

                    // Mark old document as tombstone
                    let mut tombstone = doc.clone();
                    if let Value::Object(ref mut map) = tombstone {
                        map.insert("_tombstone".to_string(), Value::Bool(true));
                        map.insert("_collection".to_string(), Value::String(self.name.clone()));
                    }
                    let tombstone_json = serde_json::to_string(&tombstone)?;

                    // Write tombstone (no catalog tracking for tombstones)
                    storage.write_data(tombstone_json.as_bytes())?;

                    self.validate_document(&document)?;

                    // Write updated document WITH catalog tracking
                    let updated_json = document.to_json()?;
                    storage.write_document_raw(
                        &self.name,
                        &document.id,
                        updated_json.as_bytes(),
                    )?;
                    storage.adjust_live_count(&self.name, -1);
                    storage.adjust_live_count(&self.name, 1);

                    modified = 1;
                }
            }
        }

        // Invalidate query cache if any document was modified
        if modified > 0 {
            self.query_cache.invalidate_collection(&self.name);
        }

        Ok((matched, modified))
    }

    /// Update many documents (raw, no WAL) - use DatabaseCore::update_many for durability
    /// Returns (matched_count, modified_count)
    fn update_many_raw(&self, query_json: &Value, update_json: &Value) -> Result<(u64, u64)> {
        // 🚀 MAJOR OPTIMIZATION: Use index-based query to get matching doc IDs
        // This uses indexes when available (34ms vs 1.8s for 10K matching docs!)
        let doc_ids = self.collect_doc_ids(query_json)?;

        let mut matched = 0u64;
        let mut modified = 0u64;

        // 🚀 OPTIMIZATION: Collect all updates for batch index processing
        let mut index_updates: Vec<(Document, Document)> = Vec::new(); // (original, updated)
        let mut storage_writes: Vec<(DocumentId, Value, String)> = Vec::new(); // (id, tombstone, updated_json)

        // 🚀 BATCH OPTIMIZATION: Read all documents in a single lock acquisition
        // Instead of N lock acquisitions for N documents, we only acquire 1 lock!
        let docs_by_id = self.batch_read_documents_by_ids(&doc_ids)?;

        // Only iterate through matching documents (not all 100K!)
        for doc_id in doc_ids {
            // Read document from batch (already loaded!)
            let doc = match docs_by_id.get(&doc_id) {
                Some(d) => d.clone(),
                None => continue, // Document was deleted or not found
            };

            // Skip tombstones (deleted documents)
            if doc
                .get("_tombstone")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                continue;
            }

            matched += 1;

            // Deserialize with proper _id handling
            let doc_json_str = serde_json::to_string(&doc)?;
            let mut document = Document::from_json(&doc_json_str)?;

            // Save original document for index removal
            let original_document = document.clone();

            // Apply update operators
            let was_modified = self.apply_update_operators(&mut document, update_json)?;

            if was_modified {
                // ✅ Ensure updated document has _collection before constraint check
                document.set("_collection".to_string(), Value::String(self.name.clone()));

                // 🔒 CHECK UNIQUE CONSTRAINTS BEFORE ANY CHANGES
                self.check_index_constraints(&document, Some(&document.id))?;

                self.validate_document(&document)?;

                // Mark old document as tombstone
                let mut tombstone = doc.clone();
                if let Value::Object(ref mut map) = tombstone {
                    map.insert("_tombstone".to_string(), Value::Bool(true));
                    map.insert("_collection".to_string(), Value::String(self.name.clone()));
                }

                let updated_json = document.to_json()?;

                // 🚀 Collect for batch processing
                index_updates.push((original_document, document));
                storage_writes.push((doc_id, tombstone, updated_json));

                modified += 1;
            }
        }

        // 🚀 BATCH INDEX UPDATE: Single lock acquisition for all index operations
        if !index_updates.is_empty() {
            self.batch_update_indexes(&index_updates)?;
        }

        // 🚀 BATCH STORAGE WRITE: Single lock acquisition for all storage operations
        self.batch_write_updates(storage_writes)?;

        // Invalidate query cache if any document was modified
        if modified > 0 {
            self.query_cache.invalidate_collection(&self.name);
        }

        Ok((matched, modified))
    }

    /// Delete one document (raw, no WAL) - use DatabaseCore::delete_one for durability
    /// Returns deleted_count
    fn delete_one_raw(&self, query_json: &Value) -> Result<u64> {
        let parsed_query = Query::from_json(query_json)?;

        // OPTIMIZATION: Try O(1) _id lookup first, fallback to full scan
        let docs_by_id = match self.try_id_query_optimization(query_json)? {
            Some(docs) => docs,
            None => self.scan_documents_via_catalog()?,
        };

        // Find first matching and delete (skip tombstones already filtered by catalog scan)
        let mut deleted = 0u64;
        let mut storage = self.storage.write();

        for (_, doc) in docs_by_id {
            if deleted > 0 {
                break; // Only delete first match
            }

            let doc_json_str = serde_json::to_string(&doc)?;
            let document = Document::from_json(&doc_json_str)?;

            // Check if matches query
            if parsed_query.matches(&document) {
                // Remove from all indexes BEFORE deleting
                // Drop storage lock temporarily to avoid potential deadlock
                drop(storage);
                self.remove_from_indexes(&document)?;
                storage = self.storage.write();

                // Mark as tombstone (logical delete)
                let mut tombstone = doc.clone();
                if let Value::Object(ref mut map) = tombstone {
                    map.insert("_tombstone".to_string(), Value::Bool(true));
                    map.insert("_collection".to_string(), Value::String(self.name.clone()));
                }
                let tombstone_json = serde_json::to_string(&tombstone)?;

                // Write tombstone WITH catalog tracking (updates catalog entry)
                storage.write_document_raw(&self.name, &document.id, tombstone_json.as_bytes())?;
                storage.adjust_live_count(&self.name, -1);

                deleted = 1;
            }
        }

        // Invalidate query cache if any document was deleted
        if deleted > 0 {
            self.query_cache.invalidate_collection(&self.name);
        }

        Ok(deleted)
    }

    /// Delete many documents (raw, no WAL) - use DatabaseCore::delete_many for durability
    /// Returns deleted_count
    fn delete_many_raw(&self, query_json: &Value) -> Result<u64> {
        let parsed_query = Query::from_json(query_json)?;
        let docs_by_id = self.scan_documents_via_catalog()?;
        let mut storage = self.storage.write();

        let mut deleted = 0u64;

        for (_, doc) in docs_by_id {
            // Skip tombstones (already deleted documents)
            if doc
                .get("_tombstone")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                continue;
            }

            let doc_json_str = serde_json::to_string(&doc)?;
            let document = Document::from_json(&doc_json_str)?;

            // Check if matches query
            if parsed_query.matches(&document) {
                // Remove from all indexes BEFORE deleting
                // Drop storage lock temporarily to avoid potential deadlock
                drop(storage);
                self.remove_from_indexes(&document)?;
                storage = self.storage.write();

                // Mark as tombstone (logical delete)
                let mut tombstone = doc.clone();
                if let Value::Object(ref mut map) = tombstone {
                    map.insert("_tombstone".to_string(), Value::Bool(true));
                    map.insert("_collection".to_string(), Value::String(self.name.clone()));
                }
                let tombstone_json = serde_json::to_string(&tombstone)?;

                storage.write_document_raw(&self.name, &document.id, tombstone_json.as_bytes())?;

                deleted += 1;
            }
        }

        // Invalidate query cache if any document was deleted
        if deleted > 0 {
            self.query_cache.invalidate_collection(&self.name);
            storage.adjust_live_count(&self.name, -(deleted as i64));
        }

        Ok(deleted)
    }
}
