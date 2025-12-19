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

use super::{BatchConstraintValidator, CollectionCore, InsertManyResult};

// ============================================================================
// PREPARE/PERSIST STRUCTURES (BUG #1 FIX - WAL ORDERING)
// ============================================================================

/// Prepared data for update_many operation.
/// Contains all info needed for WAL and storage persist.
///
/// BUG #1 FIX: This enables write-ahead logging by separating:
/// - PREPARE phase: Compute updates in memory (no storage writes)
/// - PERSIST phase: Write to storage (after WAL commit)
#[derive(Debug)]
pub struct UpdateManyPrepared {
    /// Number of documents matching the query
    pub matched: u64,
    /// Number of documents actually modified
    pub modified: u64,
    /// WAL entries: (doc_id, old_doc, new_doc)
    pub wal_entries: Vec<(DocumentId, Value, Value)>,
    /// Index updates: (original_doc, updated_doc)
    pub(crate) index_updates: Vec<(Document, Document)>,
    /// Storage writes: (doc_id, tombstone_value, updated_json_string)
    pub(crate) storage_writes: Vec<(DocumentId, Value, String)>,
}

/// Prepared data for delete_many operation.
/// Contains all info needed for WAL and storage persist.
///
/// BUG #1 FIX: This enables write-ahead logging by separating:
/// - PREPARE phase: Identify deletions in memory (no storage writes)
/// - PERSIST phase: Write tombstones to storage (after WAL commit)
#[derive(Debug)]
pub struct DeleteManyPrepared {
    /// Number of documents to be deleted
    pub deleted: u64,
    /// WAL entries: (doc_id, old_doc)
    pub wal_entries: Vec<(DocumentId, Value)>,
    /// Documents to remove from indexes
    pub(crate) index_removals: Vec<Document>,
    /// Tombstone writes: (doc_id, tombstone_json_string)
    pub(crate) tombstone_writes: Vec<(DocumentId, String)>,
}

/// Helper: Check if document is a tombstone
#[inline]
fn is_tombstone(doc: &Value) -> bool {
    doc.get("_tombstone")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// Helper: Direct _id lookup optimization (O(1) instead of full scan)
/// Returns Some(map) if query is `{_id: <value>}`, None otherwise (fallback to scan)
fn try_direct_id_lookup<S: Storage + RawStorage>(
    storage: &mut S,
    catalog: &HashMap<DocumentId, u64>,
    query_json: &Value,
) -> Option<HashMap<DocumentId, Value>> {
    let query_obj = query_json.as_object()?;
    if query_obj.len() != 1 || !query_obj.contains_key("_id") {
        return None;
    }
    let id_val = query_obj.get("_id")?;
    let doc_id: DocumentId = serde_json::from_value(id_val.clone()).ok()?;
    let &offset = catalog.get(&doc_id)?;
    let doc_bytes = storage.read_data(offset).ok()?;
    let doc: Value = serde_json::from_slice(&doc_bytes).ok()?;

    if is_tombstone(&doc) {
        return Some(HashMap::new());
    }

    let mut map = HashMap::new();
    map.insert(doc_id, doc);
    Some(map)
}

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

    /// Update many documents WITHOUT WAL protection - returns actual modified documents
    /// Returns (matched_count, modified_count, Vec<(doc_id, old_doc, new_doc)>)
    ///
    /// BUG #2 FIX: This method returns the ACTUAL documents that were modified,
    /// eliminating the race condition where a concurrent insert could be updated
    /// but not logged in the WAL.
    #[allow(clippy::type_complexity)]
    fn update_many_raw_with_docs(
        &self,
        query: &Value,
        update: &Value,
    ) -> Result<(u64, u64, Vec<(DocumentId, Value, Value)>)>;

    /// Delete one document WITHOUT WAL protection
    fn delete_one_raw(&self, query: &Value) -> Result<u64>;

    /// Delete many documents WITHOUT WAL protection
    fn delete_many_raw(&self, query: &Value) -> Result<u64>;

    /// Delete many documents WITHOUT WAL protection - returns actual deleted documents
    /// Returns (deleted_count, Vec<(doc_id, deleted_doc)>)
    ///
    /// BUG #2 FIX: This method returns the ACTUAL documents that were deleted,
    /// eliminating the race condition where a concurrent insert could be deleted
    /// but not logged in the WAL.
    fn delete_many_raw_with_docs(&self, query: &Value) -> Result<(u64, Vec<(DocumentId, Value)>)>;

    // ========================================================================
    // PREPARE/PERSIST METHODS (BUG #1 FIX - WAL ORDERING)
    // ========================================================================

    /// PREPARE phase for update_many: compute updates in memory, NO storage writes.
    ///
    /// BUG #1 FIX: This enables proper write-ahead logging:
    /// 1. Call update_many_prepare() - computes updates, validates constraints
    /// 2. Write to WAL using prepared.wal_entries
    /// 3. Commit WAL (fsync)
    /// 4. Call update_many_persist() - writes to storage (safe now, WAL is committed)
    fn update_many_prepare(&self, query: &Value, update: &Value) -> Result<UpdateManyPrepared>;

    /// PERSIST phase for update_many: write to storage AFTER WAL commit.
    ///
    /// BUG #1 FIX: Only call this after WAL is committed!
    fn update_many_persist(&self, prepared: UpdateManyPrepared) -> Result<(u64, u64)>;

    /// PREPARE phase for delete_many: identify deletions in memory, NO storage writes.
    ///
    /// BUG #1 FIX: This enables proper write-ahead logging:
    /// 1. Call delete_many_prepare() - identifies documents to delete
    /// 2. Write to WAL using prepared.wal_entries
    /// 3. Commit WAL (fsync)
    /// 4. Call delete_many_persist() - writes tombstones (safe now, WAL is committed)
    fn delete_many_prepare(&self, query: &Value) -> Result<DeleteManyPrepared>;

    /// PERSIST phase for delete_many: write tombstones AFTER WAL commit.
    ///
    /// BUG #1 FIX: Only call this after WAL is committed!
    fn delete_many_persist(&self, prepared: DeleteManyPrepared) -> Result<u64>;
}

// ============================================================================
// TRAIT IMPLEMENTATION
// ============================================================================

impl<S: Storage + RawStorage> RawOperations for CollectionCore<S> {
    /// Insert one document (raw, no WAL) - use DatabaseCore::insert_one for durability
    /// For batch operations, use DurabilityMode::Batch
    fn insert_one_raw(&self, mut fields: HashMap<String, Value>) -> Result<DocumentId> {
        self.check_not_closed()?;
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

        // Create document
        let doc = Document::new(doc_id.clone(), fields);
        self.validate_document(&doc)?;

        // Update indexes BEFORE writing to storage
        self.add_to_indexes(&doc)?;

        // Serialize and write - use write_document with catalog tracking
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
        self.check_not_closed()?;
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

        // 🔒 FIX #17: Create batch constraint validator to detect duplicates WITHIN batch
        // This prevents insert_many from bypassing unique constraints when inserting
        // multiple documents with the same unique field value in a single batch.
        let mut batch_validator = {
            let indexes = self.indexes.read();
            BatchConstraintValidator::new(&indexes, &self.name)
        };

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

            // 🔒 FIX #17: Check for duplicates WITHIN the current batch
            let doc_value = serde_json::to_value(&doc)
                .map_err(|e| MongoLiteError::Serialization(e.to_string()))?;
            batch_validator.check_and_track(&doc_value)?;

            // 🔒 FIX #18: Check against EXISTING documents in index BEFORE any writes
            // This ensures atomicity: all constraint checks happen first, then all writes.
            // Previously, batch_add_to_indexes would fail mid-way leaving partial inserts.
            self.check_index_constraints(&doc, None)?;

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
    ///
    /// 🔒 ATOMIC UPDATE: This function holds a write lock for the entire read-modify-write cycle
    /// to prevent lost updates under concurrent access. This is critical for $inc operations.
    fn update_one_raw(&self, query_json: &Value, update_json: &Value) -> Result<(u64, u64)> {
        self.check_not_closed()?;
        let parsed_query = Query::from_json(query_json)?;

        // 🔒 ATOMIC: Acquire write lock FIRST, before reading documents
        // This prevents race conditions where two threads read the same value,
        // both increment, and one overwrites the other's update (lost update anomaly).
        let mut storage = self.storage.write();

        // Clone catalog upfront to avoid borrow checker issues
        // This is necessary because we need to hold the write lock while reading documents
        let catalog = match storage.get_collection_meta(&self.name) {
            Some(m) => m.document_catalog.clone(),
            None => return Ok((0, 0)), // Collection doesn't exist
        };

        // Read documents WITHIN the write lock
        // OPTIMIZATION: O(1) lookup for _id queries, fallback to full scan
        let docs_by_id: HashMap<DocumentId, Value> =
            match try_direct_id_lookup(&mut *storage, &catalog, query_json) {
                Some(map) => map,
                None => inline_scan_with_catalog(&mut *storage, &catalog)?,
            };

        // Find first matching and update (skip tombstones already filtered by catalog scan)
        let mut matched = 0u64;
        let mut modified = 0u64;

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
                let was_modified =
                    super::update_operators::apply_update_operators(&mut document, update_json)?;

                if was_modified {
                    // ✅ Ensure updated document has _collection before constraint check
                    document.set("_collection".to_string(), Value::String(self.name.clone()));

                    // 🔒 CHECK UNIQUE CONSTRAINTS BEFORE ANY CHANGES
                    // exclude_id = Some to allow updating same document's non-key fields
                    self.check_index_constraints(&document, Some(&document.id))?;

                    // 🔒 ATOMIC: Keep storage lock held during index operations!
                    // Previously we dropped the lock here, but that created a race condition:
                    // Thread A: reads doc (value=10), applies $inc (value=11), drops lock
                    // Thread B: acquires lock, reads doc (value=10!), applies $inc (value=11)
                    // Thread A: re-acquires lock, writes value=11
                    // Thread B: writes value=11 (LOST UPDATE!)
                    //
                    // Index operations use their own lock (self.indexes), so we can safely
                    // hold the storage lock while updating indexes.

                    // 📤 REMOVE OLD DOCUMENT FROM INDEXES (uses self.indexes lock)
                    self.remove_from_indexes(&original_document)?;

                    // 📥 ADD UPDATED DOCUMENT TO INDEXES (uses self.indexes lock)
                    self.add_to_indexes(&document)?;

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
        // Delegate to update_many_raw_with_docs and discard the document details
        let (matched, modified, _docs) = self.update_many_raw_with_docs(query_json, update_json)?;
        Ok((matched, modified))
    }

    /// Update many documents (raw, no WAL) - returns actual modified documents
    /// Returns (matched_count, modified_count, Vec<(doc_id, old_doc_value, new_doc_value)>)
    ///
    /// BUG #2 FIX: This method returns the ACTUAL documents that were modified,
    /// eliminating the race condition where a concurrent insert could be updated
    /// but not logged in the WAL.
    fn update_many_raw_with_docs(
        &self,
        query_json: &Value,
        update_json: &Value,
    ) -> Result<(u64, u64, Vec<(DocumentId, Value, Value)>)> {
        self.check_not_closed()?;
        // 🚀 MAJOR OPTIMIZATION: Use index-based query to get matching doc IDs
        // This uses indexes when available (34ms vs 1.8s for 10K matching docs!)
        let doc_ids = self.collect_doc_ids(query_json)?;

        let mut matched = 0u64;
        let mut modified = 0u64;

        // 🚀 OPTIMIZATION: Collect all updates for batch index processing
        let mut index_updates: Vec<(Document, Document)> = Vec::new(); // (original, updated)
        let mut storage_writes: Vec<(DocumentId, Value, String)> = Vec::new(); // (id, tombstone, updated_json)

        // BUG #2 FIX: Track actual modified documents for WAL
        let mut modified_docs: Vec<(DocumentId, Value, Value)> = Vec::new();

        // 🔒 FIX #16: Use BatchConstraintValidator for unified duplicate detection
        // This prevents update_many from bypassing unique constraints when updating
        // multiple documents to the same value in a single batch operation.
        let mut batch_validator = {
            let indexes = self.indexes.read();
            BatchConstraintValidator::new(&indexes, &self.name)
        };

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
            if is_tombstone(&doc) {
                continue;
            }

            matched += 1;

            // Deserialize with proper _id handling
            let doc_json_str = serde_json::to_string(&doc)?;
            let mut document = Document::from_json(&doc_json_str)?;

            // Save original document for index removal and WAL
            let original_document = document.clone();
            let old_doc_value = doc.clone();

            // Apply update operators
            let was_modified =
                super::update_operators::apply_update_operators(&mut document, update_json)?;

            if was_modified {
                // ✅ Ensure updated document has _collection before constraint check
                document.set("_collection".to_string(), Value::String(self.name.clone()));

                // 🔒 FIX #16: Check for duplicates WITHIN the current batch
                // Uses unified BatchConstraintValidator for consistent duplicate detection.
                let doc_value = serde_json::to_value(&document)
                    .map_err(|e| MongoLiteError::Serialization(e.to_string()))?;
                batch_validator.check_and_track(&doc_value)?;

                // 🔒 CHECK UNIQUE CONSTRAINTS against existing index
                self.check_index_constraints(&document, Some(&document.id))?;

                self.validate_document(&document)?;

                // Mark old document as tombstone
                let mut tombstone = doc.clone();
                if let Value::Object(ref mut map) = tombstone {
                    map.insert("_tombstone".to_string(), Value::Bool(true));
                    map.insert("_collection".to_string(), Value::String(self.name.clone()));
                }

                let updated_json = document.to_json()?;

                // BUG #2 FIX: Collect modified document for WAL
                let new_doc_value: Value = serde_json::from_str(&updated_json)?;
                modified_docs.push((doc_id.clone(), old_doc_value, new_doc_value));

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

        Ok((matched, modified, modified_docs))
    }

    /// Delete one document (raw, no WAL) - use DatabaseCore::delete_one for durability
    /// Returns deleted_count
    fn delete_one_raw(&self, query_json: &Value) -> Result<u64> {
        self.check_not_closed()?;
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
        // Delegate to delete_many_raw_with_docs and discard the document details
        let (deleted, _docs) = self.delete_many_raw_with_docs(query_json)?;
        Ok(deleted)
    }

    /// Delete many documents (raw, no WAL) - returns actual deleted documents
    /// Returns (deleted_count, Vec<(doc_id, deleted_doc_value)>)
    ///
    /// BUG #2 FIX: This method returns the ACTUAL documents that were deleted,
    /// eliminating the race condition where a concurrent insert could be deleted
    /// but not logged in the WAL.
    fn delete_many_raw_with_docs(
        &self,
        query_json: &Value,
    ) -> Result<(u64, Vec<(DocumentId, Value)>)> {
        self.check_not_closed()?;
        let parsed_query = Query::from_json(query_json)?;
        let docs_by_id = self.scan_documents_via_catalog()?;
        let mut storage = self.storage.write();

        let mut deleted = 0u64;
        // BUG #2 FIX: Track actual deleted documents for WAL
        let mut deleted_docs: Vec<(DocumentId, Value)> = Vec::new();

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
                // BUG #2 FIX: Collect deleted document for WAL BEFORE deletion
                deleted_docs.push((document.id.clone(), doc.clone()));

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

        Ok((deleted, deleted_docs))
    }

    // ========================================================================
    // PREPARE/PERSIST IMPLEMENTATIONS (BUG #1 FIX - WAL ORDERING)
    // ========================================================================

    /// PREPARE phase for update_many: compute updates in memory, NO storage writes.
    ///
    /// BUG #1 FIX: This method does all the work EXCEPT writing to storage:
    /// - Finds matching documents
    /// - Applies update operators in memory
    /// - Validates constraints (unique indexes, schema)
    /// - Returns prepared data for WAL and persist phase
    fn update_many_prepare(
        &self,
        query_json: &Value,
        update_json: &Value,
    ) -> Result<UpdateManyPrepared> {
        self.check_not_closed()?;

        // Use index-based query to get matching doc IDs
        let doc_ids = self.collect_doc_ids(query_json)?;

        let mut matched = 0u64;
        let mut modified = 0u64;

        // Collect all updates for batch processing (NO WRITES YET!)
        let mut index_updates: Vec<(Document, Document)> = Vec::new();
        let mut storage_writes: Vec<(DocumentId, Value, String)> = Vec::new();
        let mut wal_entries: Vec<(DocumentId, Value, Value)> = Vec::new();

        // Use BatchConstraintValidator for unified duplicate detection
        let mut batch_validator = {
            let indexes = self.indexes.read();
            BatchConstraintValidator::new(&indexes, &self.name)
        };

        // BATCH OPTIMIZATION: Read all documents in a single lock acquisition
        let docs_by_id = self.batch_read_documents_by_ids(&doc_ids)?;

        // Only iterate through matching documents
        for doc_id in doc_ids {
            let doc = match docs_by_id.get(&doc_id) {
                Some(d) => d.clone(),
                None => continue,
            };

            if is_tombstone(&doc) {
                continue;
            }

            matched += 1;

            let doc_json_str = serde_json::to_string(&doc)?;
            let mut document = Document::from_json(&doc_json_str)?;
            let original_document = document.clone();
            let old_doc_value = doc.clone();

            // Apply update operators
            let was_modified =
                super::update_operators::apply_update_operators(&mut document, update_json)?;

            if was_modified {
                document.set("_collection".to_string(), Value::String(self.name.clone()));

                // Check for duplicates WITHIN the current batch
                let doc_value = serde_json::to_value(&document)
                    .map_err(|e| MongoLiteError::Serialization(e.to_string()))?;
                batch_validator.check_and_track(&doc_value)?;

                // Check unique constraints against existing index
                self.check_index_constraints(&document, Some(&document.id))?;
                self.validate_document(&document)?;

                // Prepare tombstone
                let mut tombstone = doc.clone();
                if let Value::Object(ref mut map) = tombstone {
                    map.insert("_tombstone".to_string(), Value::Bool(true));
                    map.insert("_collection".to_string(), Value::String(self.name.clone()));
                }

                let updated_json = document.to_json()?;
                let new_doc_value: Value = serde_json::from_str(&updated_json)?;

                // Collect for WAL (used by database.rs)
                wal_entries.push((doc_id.clone(), old_doc_value, new_doc_value));

                // Collect for persist phase (index updates + storage writes)
                index_updates.push((original_document, document));
                storage_writes.push((doc_id, tombstone, updated_json));

                modified += 1;
            }
        }

        Ok(UpdateManyPrepared {
            matched,
            modified,
            wal_entries,
            index_updates,
            storage_writes,
        })
    }

    /// PERSIST phase for update_many: write to storage AFTER WAL commit.
    ///
    /// BUG #1 FIX: Only call this after WAL is committed!
    /// This method:
    /// - Updates indexes (batch operation)
    /// - Writes tombstones and updated documents to storage
    /// - Invalidates query cache
    fn update_many_persist(&self, prepared: UpdateManyPrepared) -> Result<(u64, u64)> {
        // BATCH INDEX UPDATE: Single lock acquisition for all index operations
        if !prepared.index_updates.is_empty() {
            self.batch_update_indexes(&prepared.index_updates)?;
        }

        // BATCH STORAGE WRITE: Single lock acquisition for all storage operations
        self.batch_write_updates(prepared.storage_writes)?;

        // Invalidate query cache if any document was modified
        if prepared.modified > 0 {
            self.query_cache.invalidate_collection(&self.name);
        }

        Ok((prepared.matched, prepared.modified))
    }

    /// PREPARE phase for delete_many: identify deletions in memory, NO storage writes.
    ///
    /// BUG #1 FIX: This method does all the work EXCEPT writing to storage:
    /// - Finds matching documents
    /// - Validates deletions
    /// - Returns prepared data for WAL and persist phase
    fn delete_many_prepare(&self, query_json: &Value) -> Result<DeleteManyPrepared> {
        self.check_not_closed()?;
        let parsed_query = Query::from_json(query_json)?;
        let docs_by_id = self.scan_documents_via_catalog()?;

        let mut deleted = 0u64;
        let mut wal_entries: Vec<(DocumentId, Value)> = Vec::new();
        let mut index_removals: Vec<Document> = Vec::new();
        let mut tombstone_writes: Vec<(DocumentId, String)> = Vec::new();

        for (_, doc) in docs_by_id {
            if is_tombstone(&doc) {
                continue;
            }

            let doc_json_str = serde_json::to_string(&doc)?;
            let document = Document::from_json(&doc_json_str)?;

            if parsed_query.matches(&document) {
                // Collect for WAL
                wal_entries.push((document.id.clone(), doc.clone()));

                // Collect for index removal in persist phase
                index_removals.push(document.clone());

                // Prepare tombstone for persist phase
                let mut tombstone = doc.clone();
                if let Value::Object(ref mut map) = tombstone {
                    map.insert("_tombstone".to_string(), Value::Bool(true));
                    map.insert("_collection".to_string(), Value::String(self.name.clone()));
                }
                let tombstone_json = serde_json::to_string(&tombstone)?;
                tombstone_writes.push((document.id.clone(), tombstone_json));

                deleted += 1;
            }
        }

        Ok(DeleteManyPrepared {
            deleted,
            wal_entries,
            index_removals,
            tombstone_writes,
        })
    }

    /// PERSIST phase for delete_many: write tombstones AFTER WAL commit.
    ///
    /// BUG #1 FIX: Only call this after WAL is committed!
    /// This method:
    /// - Removes from all indexes
    /// - Writes tombstones to storage
    /// - Invalidates query cache
    fn delete_many_persist(&self, prepared: DeleteManyPrepared) -> Result<u64> {
        // Remove from indexes FIRST
        for document in &prepared.index_removals {
            self.remove_from_indexes(document)?;
        }

        // Write tombstones to storage
        if !prepared.tombstone_writes.is_empty() {
            let mut storage = self.storage.write();
            for (doc_id, tombstone_json) in &prepared.tombstone_writes {
                storage.write_document_raw(&self.name, doc_id, tombstone_json.as_bytes())?;
            }
            storage.adjust_live_count(&self.name, -(prepared.deleted as i64));
        }

        // Invalidate query cache if any document was deleted
        if prepared.deleted > 0 {
            self.query_cache.invalidate_collection(&self.name);
        }

        Ok(prepared.deleted)
    }
}

/// Helper: Scan documents using a pre-cloned catalog and pre-held storage guard
/// This enables atomic read-modify-write operations by keeping the lock held.
///
/// 🔒 ATOMIC: This function is designed to be called while holding a write lock,
/// avoiding the read-then-write race condition that causes lost updates.
fn inline_scan_with_catalog<S: Storage + RawStorage>(
    storage: &mut S,
    catalog: &HashMap<DocumentId, u64>,
) -> Result<HashMap<DocumentId, Value>> {
    let mut docs_by_id: HashMap<DocumentId, Value> = HashMap::new();

    for (doc_id, offset) in catalog {
        if let Ok(doc_bytes) = storage.read_data(*offset) {
            if let Ok(doc) = serde_json::from_slice::<Value>(&doc_bytes) {
                // Skip tombstones (deleted documents)
                if !doc
                    .get("_tombstone")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                {
                    docs_by_id.insert(doc_id.clone(), doc);
                }
            }
        }
    }

    Ok(docs_by_id)
}
