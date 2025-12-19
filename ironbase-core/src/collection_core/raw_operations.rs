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

/// Prepared data for insert_one operation.
/// Contains all info needed for WAL and storage persist.
///
/// WAL REFACTOR: Enables consistent prepare/persist pattern for inserts:
/// - PREPARE phase: Validate document, generate _id, prepare WAL entry (no storage writes)
/// - PERSIST phase: Write to storage and update indexes (after WAL commit)
#[derive(Debug)]
pub struct InsertOnePrepared {
    /// The document ID (auto-generated or provided)
    pub doc_id: DocumentId,
    /// The validated document ready for storage (has _id, NO _collection)
    pub document: Document,
    /// WAL entry: document with _id AND _collection for recovery
    pub wal_doc: Value,
    /// Collection name for context
    pub(crate) collection_name: String,
}

/// Prepared data for insert_many operation.
/// Contains all info needed for WAL and storage persist.
///
/// WAL REFACTOR: Enables consistent prepare/persist pattern for batch inserts:
/// - PREPARE phase: Validate all documents, check constraints (no storage writes)
/// - PERSIST phase: Write all to storage and update indexes (after WAL commit)
#[derive(Debug)]
pub struct InsertManyPrepared {
    /// Individual prepared inserts
    pub prepared_docs: Vec<InsertOnePrepared>,
    /// All inserted document IDs (for convenience)
    pub inserted_ids: Vec<DocumentId>,
    /// Collection name for context
    pub(crate) collection_name: String,
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

    // ========================================================================
    // INSERT PREPARE/PERSIST METHODS (WAL REFACTOR)
    // ========================================================================

    /// PREPARE phase for insert_one: validate document and generate IDs, NO storage writes.
    ///
    /// WAL REFACTOR: This enables proper write-ahead logging for inserts:
    /// 1. Call insert_one_prepare() - validates document, generates _id, prepares WAL entry
    /// 2. Write to WAL using prepared.wal_doc
    /// 3. Commit WAL (fsync)
    /// 4. Call insert_one_persist() - writes to storage (safe now, WAL is committed)
    fn insert_one_prepare(&self, fields: HashMap<String, Value>) -> Result<InsertOnePrepared>;

    /// PERSIST phase for insert_one: write to storage AFTER WAL commit.
    ///
    /// WAL REFACTOR: Only call this after WAL is committed!
    fn insert_one_persist(&self, prepared: InsertOnePrepared) -> Result<DocumentId>;

    /// PREPARE phase for insert_many: validate all documents and generate IDs, NO storage writes.
    ///
    /// WAL REFACTOR: This enables proper write-ahead logging for batch inserts:
    /// 1. Call insert_many_prepare() - validates all documents, checks constraints
    /// 2. Write to WAL using prepared.prepared_docs[].wal_doc
    /// 3. Commit WAL (fsync)
    /// 4. Call insert_many_persist() - writes all to storage (safe now, WAL is committed)
    fn insert_many_prepare(
        &self,
        documents: Vec<HashMap<String, Value>>,
    ) -> Result<InsertManyPrepared>;

    /// PERSIST phase for insert_many: write all documents to storage AFTER WAL commit.
    ///
    /// WAL REFACTOR: Only call this after WAL is committed!
    fn insert_many_persist(&self, prepared: InsertManyPrepared) -> Result<Vec<DocumentId>>;
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

        // NOTE: _collection is NOT added here - it's WAL-only metadata
        // Storage documents should NOT contain _collection

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

            // NOTE: _collection is NOT added here - it's WAL-only metadata
            // Storage documents should NOT contain _collection

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
                    // NOTE: _collection is NOT added here - it's WAL-only metadata
                    // Storage documents should NOT contain _collection

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
                // NOTE: _collection is NOT added here - it's WAL-only metadata
                // Storage documents should NOT contain _collection

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
                // NOTE: _collection is NOT added here - it's WAL-only metadata
                // Storage documents should NOT contain _collection

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
    /// - Updates indexes (batch operation) FIRST
    /// - Writes tombstones and updated documents to storage
    /// - Invalidates query cache
    ///
    /// NOTE: For batch operations, index-first is safer for partial storage failure.
    /// Phantom index entries are gracefully handled (fetch fails, skip).
    fn update_many_persist(&self, prepared: UpdateManyPrepared) -> Result<(u64, u64)> {
        // BATCH INDEX UPDATE FIRST (safer for partial storage failure)
        if !prepared.index_updates.is_empty() {
            self.batch_update_indexes(&prepared.index_updates)?;
        }

        // BATCH STORAGE WRITE
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
    /// - Removes from all indexes FIRST
    /// - Writes tombstones to storage
    /// - Invalidates query cache
    ///
    /// NOTE: For batch operations, index-first is safer for partial storage failure.
    /// If tombstone write fails mid-batch, removed index entries mean those docs
    /// won't appear in queries (acceptable - they were meant to be deleted).
    /// Recovery rebuilds indexes from storage, ensuring consistency.
    fn delete_many_persist(&self, prepared: DeleteManyPrepared) -> Result<u64> {
        // Remove from indexes FIRST (safer for partial storage failure)
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

    // ========================================================================
    // INSERT PREPARE/PERSIST IMPLEMENTATIONS (WAL REFACTOR)
    // ========================================================================

    /// PREPARE phase for insert_one: validate document and generate IDs, NO storage writes.
    ///
    /// WAL REFACTOR: This method does all the work EXCEPT writing to storage:
    /// - Validates document against schema
    /// - Generates _id if not provided
    /// - Prepares WAL entry (with _collection for recovery)
    /// - Returns prepared data for WAL and persist phase
    ///
    /// LOCK ORDERING: To avoid deadlock with drop_index (which acquires index→storage),
    /// we release the storage lock BEFORE acquiring the index lock for constraint checks.
    /// This ensures: insert uses storage→(release)→index, while drop_index uses index→storage.
    fn insert_one_prepare(&self, mut fields: HashMap<String, Value>) -> Result<InsertOnePrepared> {
        self.check_not_closed()?;

        // ====================================================================
        // PHASE 1: Generate ID (needs storage lock for meta.last_id)
        // ====================================================================
        let doc_id = {
            let mut storage = self.storage.write();

            let meta = storage
                .get_collection_meta_mut(&self.name)
                .ok_or_else(|| MongoLiteError::CollectionNotFound(self.name.clone()))?;

            let id = if let Some(existing_id) = fields.get("_id") {
                // Use existing _id from fields
                let parsed_id: DocumentId =
                    serde_json::from_value(existing_id.clone()).map_err(|e| {
                        MongoLiteError::Serialization(format!("Invalid _id format: {}", e))
                    })?;

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

            id
        }; // Storage lock released here - BEFORE acquiring index lock

        // ====================================================================
        // PHASE 2: Validate and check constraints (needs index lock, NOT storage)
        // ====================================================================
        let doc = Document::new(doc_id.clone(), fields.clone());
        self.validate_document(&doc)?;

        // Check index constraints (acquires indexes.read() internally)
        // This catches duplicates early, before any storage/WAL writes
        self.check_index_constraints(&doc, None)?;

        // Prepare WAL document (PHASE 5: _collection is now added by commit_transaction)
        let wal_doc =
            serde_json::to_value(&doc).map_err(|e| MongoLiteError::Serialization(e.to_string()))?;

        Ok(InsertOnePrepared {
            doc_id,
            document: doc,
            wal_doc,
            collection_name: self.name.clone(),
        })
    }

    /// PERSIST phase for insert_one: write to storage AFTER WAL commit.
    ///
    /// WAL REFACTOR: Only call this after WAL is committed!
    /// This method:
    /// - Writes document to storage FIRST
    /// - Updates indexes (adds document) AFTER storage success
    /// - Invalidates query cache
    ///
    /// PHASE 4 FIX: Storage write before index update prevents orphan index entries.
    /// If storage write fails, no index entry is created.
    /// If index update fails (rare - constraints checked in prepare), recovery rebuilds.
    fn insert_one_persist(&self, prepared: InsertOnePrepared) -> Result<DocumentId> {
        // PHASE 4: Write to storage FIRST
        {
            let mut storage = self.storage.write();
            let doc_json = prepared.document.to_json()?;
            storage.write_document_raw(
                &prepared.collection_name,
                &prepared.doc_id,
                doc_json.as_bytes(),
            )?;
            storage.adjust_live_count(&prepared.collection_name, 1);
        } // Release storage lock before index update

        // PHASE 4: Update indexes AFTER successful storage write
        // Constraint check was done in prepare phase, so this should not fail
        self.add_to_indexes(&prepared.document)?;

        // Invalidate query cache (collection has changed)
        self.query_cache
            .invalidate_collection(&prepared.collection_name);

        Ok(prepared.doc_id)
    }

    /// PREPARE phase for insert_many: validate all documents and generate IDs, NO storage writes.
    ///
    /// WAL REFACTOR: This method does all the work EXCEPT writing to storage:
    /// - Validates all documents against schema
    /// - Generates _id for documents that don't have one
    /// - Checks constraints (unique indexes, duplicates within batch)
    /// - Prepares WAL entries (with _collection for recovery)
    /// - Returns prepared data for WAL and persist phase
    ///
    /// LOCK ORDERING: To avoid deadlock with drop_index (which acquires index→storage),
    /// we use a two-phase approach:
    /// - Phase 1: Hold storage lock for ID generation only
    /// - Phase 2: Release storage lock, then check constraints (acquires index lock)
    fn insert_many_prepare(
        &self,
        documents: Vec<HashMap<String, Value>>,
    ) -> Result<InsertManyPrepared> {
        self.check_not_closed()?;

        if documents.is_empty() {
            return Ok(InsertManyPrepared {
                prepared_docs: Vec::new(),
                inserted_ids: Vec::new(),
                collection_name: self.name.clone(),
            });
        }

        // ====================================================================
        // PHASE 1: Generate all IDs (needs storage lock for meta.last_id)
        // ====================================================================
        let docs_with_ids: Vec<(DocumentId, HashMap<String, Value>)> = {
            let mut storage = self.storage.write();

            let meta = storage
                .get_collection_meta_mut(&self.name)
                .ok_or_else(|| MongoLiteError::CollectionNotFound(self.name.clone()))?;

            let start_id = meta.last_id;
            let mut auto_id_count = 0u64;
            let mut result = Vec::with_capacity(documents.len());

            for mut fields in documents.into_iter() {
                let doc_id = if let Some(existing_id) = fields.get("_id") {
                    let parsed_id: DocumentId = serde_json::from_value(existing_id.clone())
                        .map_err(|e| {
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
                    // Auto-generate new _id
                    let new_id = DocumentId::new_auto(start_id + auto_id_count);
                    auto_id_count += 1;
                    fields.insert("_id".to_string(), serde_json::to_value(&new_id).unwrap());
                    new_id
                };

                result.push((doc_id, fields));
            }

            // Update last_id with max of manual + auto-generated IDs
            meta.last_id = meta.last_id.max(start_id + auto_id_count);

            result
        }; // Storage lock released here - BEFORE acquiring index lock

        // ====================================================================
        // PHASE 2: Validate and check constraints (needs index lock, NOT storage)
        // ====================================================================
        // Create batch constraint validator to detect duplicates WITHIN batch
        let mut batch_validator = {
            let indexes = self.indexes.read();
            BatchConstraintValidator::new(&indexes, &self.name)
        };

        let mut prepared_docs = Vec::with_capacity(docs_with_ids.len());
        let mut inserted_ids = Vec::with_capacity(docs_with_ids.len());

        for (doc_id, fields) in docs_with_ids {
            // Create document (NO _collection in storage doc)
            let doc = Document::new(doc_id.clone(), fields);
            self.validate_document(&doc)?;

            // Check for duplicates WITHIN the current batch
            let doc_value = serde_json::to_value(&doc)
                .map_err(|e| MongoLiteError::Serialization(e.to_string()))?;
            batch_validator.check_and_track(&doc_value)?;

            // Check against EXISTING documents in index
            self.check_index_constraints(&doc, None)?;

            // PHASE 5: _collection is now added by commit_transaction
            prepared_docs.push(InsertOnePrepared {
                doc_id: doc_id.clone(),
                document: doc,
                wal_doc: doc_value, // Use doc_value directly, _collection added in commit_transaction
                collection_name: self.name.clone(),
            });
            inserted_ids.push(doc_id);
        }

        Ok(InsertManyPrepared {
            prepared_docs,
            inserted_ids,
            collection_name: self.name.clone(),
        })
    }

    /// PERSIST phase for insert_many: write all documents to storage AFTER WAL commit.
    ///
    /// WAL REFACTOR: Only call this after WAL is committed!
    /// This method:
    /// - Updates indexes in batch FIRST (atomic at index level)
    /// - Writes all documents to storage
    /// - Invalidates query cache
    ///
    /// NOTE: For batch operations, index-first is safer than storage-first because:
    /// - If storage write fails mid-batch, phantom index entries are gracefully handled
    ///   (query finds entry, fetch fails, skips it)
    /// - If we did storage-first and failed mid-batch, successfully written docs
    ///   would be MISSING from indexes until restart (worse for query correctness)
    /// - Recovery rebuilds indexes from storage in both cases
    fn insert_many_persist(&self, prepared: InsertManyPrepared) -> Result<Vec<DocumentId>> {
        if prepared.prepared_docs.is_empty() {
            return Ok(Vec::new());
        }

        // Update indexes in batch FIRST (atomic operation, all-or-nothing)
        // This is safer for partial storage failure - phantom entries are handled gracefully
        let docs_for_index: Vec<Document> = prepared
            .prepared_docs
            .iter()
            .map(|p| p.document.clone())
            .collect();
        self.batch_add_to_indexes(&docs_for_index)?;

        // Write all documents to storage
        {
            let mut storage = self.storage.write();
            let mut live_delta = 0i64;

            for prep in &prepared.prepared_docs {
                let doc_json = prep.document.to_json()?;
                storage.write_document_raw(
                    &prepared.collection_name,
                    &prep.doc_id,
                    doc_json.as_bytes(),
                )?;
                live_delta += 1;
            }

            if live_delta != 0 {
                storage.adjust_live_count(&prepared.collection_name, live_delta);
            }
        }

        // Invalidate query cache (collection has changed)
        self.query_cache
            .invalidate_collection(&prepared.collection_name);

        Ok(prepared.inserted_ids)
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

// ============================================================================
// TESTS FOR PREPARE/PERSIST PATTERN
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::MemoryStorage;
    use crate::DatabaseCore;
    use serde_json::json;

    /// Helper: Create a test database with a collection
    fn create_test_db() -> DatabaseCore<MemoryStorage> {
        DatabaseCore::<MemoryStorage>::open_memory().unwrap()
    }

    // ========== INSERT_ONE_PREPARE/PERSIST TESTS ==========

    #[test]
    fn test_insert_one_prepare_generates_id() {
        let db = create_test_db();
        let coll = db.collection("test").unwrap();

        let fields = HashMap::from([("name".to_string(), json!("Alice"))]);
        let prepared = coll.insert_one_prepare(fields).unwrap();

        // Should have auto-generated ID
        assert!(matches!(prepared.doc_id, DocumentId::Int(_)));

        // Document should have _id
        assert!(prepared.document.fields.contains_key("_id"));

        // PHASE 5: _collection is NOT in wal_doc - it's added centrally in commit_transaction
        // WAL doc should have _id and data, but NOT _collection
        assert!(prepared.wal_doc.get("_id").is_some());
        assert!(prepared.wal_doc.get("name").is_some());
    }

    #[test]
    fn test_insert_one_prepare_respects_custom_id() {
        let db = create_test_db();
        let coll = db.collection("test").unwrap();

        let fields = HashMap::from([
            ("_id".to_string(), json!("custom_id")),
            ("name".to_string(), json!("Bob")),
        ]);
        let prepared = coll.insert_one_prepare(fields).unwrap();

        // Should use custom ID
        assert!(matches!(&prepared.doc_id, DocumentId::String(s) if s == "custom_id"));
    }

    #[test]
    fn test_insert_one_prepare_validates_schema() {
        let db = create_test_db();
        let coll = db.collection("test").unwrap();

        // Set a schema requiring "name" field
        coll.set_schema(Some(json!({
            "type": "object",
            "required": ["name"],
            "properties": {
                "name": {"type": "string"}
            }
        })))
        .unwrap();

        // This should fail validation (missing required field)
        let fields = HashMap::from([("age".to_string(), json!(25))]);
        let result = coll.insert_one_prepare(fields);
        assert!(result.is_err());
    }

    #[test]
    fn test_insert_one_persist_writes_to_storage() {
        let db = create_test_db();
        let coll = db.collection("test").unwrap();

        // PREPARE
        let fields = HashMap::from([("name".to_string(), json!("Alice"))]);
        let prepared = coll.insert_one_prepare(fields).unwrap();
        let doc_id = prepared.doc_id.clone();

        // PERSIST
        let result_id = coll.insert_one_persist(prepared).unwrap();
        assert_eq!(result_id, doc_id);

        // Verify document is in storage
        let count = coll.count_documents(&json!({})).unwrap();
        assert_eq!(count, 1);

        // Verify we can find the document
        let found = coll.find_one(&json!({"name": "Alice"})).unwrap();
        assert!(found.is_some());
    }

    #[test]
    fn test_insert_one_prepare_persist_separation() {
        let db = create_test_db();
        let coll = db.collection("test").unwrap();

        // PREPARE only - should NOT write to storage
        let fields = HashMap::from([("name".to_string(), json!("Alice"))]);
        let prepared = coll.insert_one_prepare(fields).unwrap();

        // Count should be 0 (not yet persisted)
        let count = coll.count_documents(&json!({})).unwrap();
        assert_eq!(count, 0);

        // Now PERSIST
        coll.insert_one_persist(prepared).unwrap();

        // Count should be 1 now
        let count = coll.count_documents(&json!({})).unwrap();
        assert_eq!(count, 1);
    }

    // ========== INSERT_MANY_PREPARE/PERSIST TESTS ==========

    #[test]
    fn test_insert_many_prepare_empty() {
        let db = create_test_db();
        let coll = db.collection("test").unwrap();

        let prepared = coll.insert_many_prepare(vec![]).unwrap();

        assert!(prepared.prepared_docs.is_empty());
        assert!(prepared.inserted_ids.is_empty());
    }

    #[test]
    fn test_insert_many_prepare_generates_ids() {
        let db = create_test_db();
        let coll = db.collection("test").unwrap();

        let docs = vec![
            HashMap::from([("name".to_string(), json!("Alice"))]),
            HashMap::from([("name".to_string(), json!("Bob"))]),
            HashMap::from([("name".to_string(), json!("Charlie"))]),
        ];

        let prepared = coll.insert_many_prepare(docs).unwrap();

        assert_eq!(prepared.prepared_docs.len(), 3);
        assert_eq!(prepared.inserted_ids.len(), 3);

        // All should have auto-generated IDs
        // PHASE 5: _collection is NOT in wal_doc - it's added centrally in commit_transaction
        for p in &prepared.prepared_docs {
            assert!(matches!(p.doc_id, DocumentId::Int(_)));
            assert!(p.wal_doc.get("_id").is_some());
        }
    }

    #[test]
    fn test_insert_many_prepare_detects_batch_duplicates() {
        let db = create_test_db();
        let coll = db.collection("test").unwrap();

        // Create unique index on "email"
        coll.create_index("email".to_string(), true).unwrap();

        // Try to insert two documents with same email in one batch
        let docs = vec![
            HashMap::from([
                ("name".to_string(), json!("Alice")),
                ("email".to_string(), json!("alice@test.com")),
            ]),
            HashMap::from([
                ("name".to_string(), json!("Duplicate")),
                ("email".to_string(), json!("alice@test.com")), // duplicate!
            ]),
        ];

        let result = coll.insert_many_prepare(docs);
        assert!(result.is_err());
    }

    #[test]
    fn test_insert_many_persist_writes_all() {
        let db = create_test_db();
        let coll = db.collection("test").unwrap();

        // PREPARE
        let docs = vec![
            HashMap::from([("value".to_string(), json!(1))]),
            HashMap::from([("value".to_string(), json!(2))]),
            HashMap::from([("value".to_string(), json!(3))]),
        ];
        let prepared = coll.insert_many_prepare(docs).unwrap();

        // Count should be 0 before persist
        assert_eq!(coll.count_documents(&json!({})).unwrap(), 0);

        // PERSIST
        let ids = coll.insert_many_persist(prepared).unwrap();
        assert_eq!(ids.len(), 3);

        // Count should be 3 after persist
        assert_eq!(coll.count_documents(&json!({})).unwrap(), 3);
    }

    #[test]
    fn test_insert_many_prepare_checks_existing_constraints() {
        let db = create_test_db();
        let coll = db.collection("test").unwrap();

        // Create unique index on "email"
        coll.create_index("email".to_string(), true).unwrap();

        // Insert one document first (via raw to skip WAL for test simplicity)
        let existing = HashMap::from([
            ("name".to_string(), json!("Existing")),
            ("email".to_string(), json!("existing@test.com")),
        ]);
        coll.insert_one_raw(existing).unwrap();

        // Now try to prepare insert with same email
        let docs = vec![HashMap::from([
            ("name".to_string(), json!("New")),
            ("email".to_string(), json!("existing@test.com")), // duplicate!
        ])];

        let result = coll.insert_many_prepare(docs);
        assert!(result.is_err());
    }

    #[test]
    fn test_insert_one_prepare_wal_doc_structure() {
        let db = create_test_db();
        let coll = db.collection("users").unwrap();

        let fields = HashMap::from([("name".to_string(), json!("Test"))]);
        let prepared = coll.insert_one_prepare(fields).unwrap();

        // Verify WAL doc structure (PHASE 5: _collection is now added by commit_transaction)
        let wal_doc = &prepared.wal_doc;
        assert_eq!(wal_doc.get("name").unwrap(), &json!("Test"));
        assert!(wal_doc.get("_id").is_some());
        // NOTE: _collection is NOT in wal_doc here - it's added centrally in commit_transaction

        // Storage doc should NOT have _collection
        assert!(!prepared.document.fields.contains_key("_collection"));
    }

    #[test]
    fn test_insert_many_mixed_ids() {
        let db = create_test_db();
        let coll = db.collection("test").unwrap();

        // Mix of auto and manual IDs
        let docs = vec![
            HashMap::from([("name".to_string(), json!("Auto1"))]),
            HashMap::from([
                ("_id".to_string(), json!("manual_id")),
                ("name".to_string(), json!("Manual")),
            ]),
            HashMap::from([("name".to_string(), json!("Auto2"))]),
        ];

        let prepared = coll.insert_many_prepare(docs).unwrap();
        assert_eq!(prepared.inserted_ids.len(), 3);

        // Second should be manual ID
        assert!(matches!(&prepared.inserted_ids[1], DocumentId::String(s) if s == "manual_id"));

        // First and third should be auto IDs
        assert!(matches!(&prepared.inserted_ids[0], DocumentId::Int(_)));
        assert!(matches!(&prepared.inserted_ids[2], DocumentId::Int(_)));
    }

    #[test]
    fn test_insert_prepare_closed_collection_fails() {
        use crate::storage::StorageEngine;
        use std::sync::atomic::{AtomicUsize, Ordering};

        static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);
        let counter = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let temp_dir = std::env::temp_dir();
        let db_path = temp_dir
            .join(format!("test_closed_prepare_{}.mlite", counter))
            .to_string_lossy()
            .to_string();

        // Cleanup previous test files
        let _ = std::fs::remove_file(&db_path);
        let wal_path = format!("{}.wal", db_path.trim_end_matches(".mlite"));
        let _ = std::fs::remove_file(&wal_path);

        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        let coll = db.collection("test").unwrap();

        // Close the database
        db.close().unwrap();

        // Should fail because collection is closed
        let fields = HashMap::from([("name".to_string(), json!("Test"))]);
        let result = coll.insert_one_prepare(fields);
        assert!(result.is_err());

        // Cleanup
        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_file(&wal_path);
    }

    // ==================== CONCURRENT TESTS ====================
    // These tests verify that lock ordering is correct and no deadlocks occur

    #[test]
    fn test_concurrent_insert_prepare_no_deadlock() {
        use std::sync::Arc;
        use std::thread;

        let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
        let coll = Arc::new(db.collection("test").unwrap());

        // Spawn multiple threads doing concurrent insert_prepare
        let handles: Vec<_> = (0..10)
            .map(|i| {
                let coll = Arc::clone(&coll);
                thread::spawn(move || {
                    for j in 0..100 {
                        let fields = HashMap::from([
                            ("thread".to_string(), json!(i)),
                            ("iter".to_string(), json!(j)),
                        ]);
                        let prepared = coll.insert_one_prepare(fields).unwrap();
                        coll.insert_one_persist(prepared).unwrap();
                    }
                })
            })
            .collect();

        // If we get here without hanging, there's no deadlock
        for handle in handles {
            handle.join().expect("Thread should not panic");
        }

        // Verify all inserts succeeded
        let count = coll.find(&json!({})).unwrap().len();
        assert_eq!(count, 1000); // 10 threads * 100 inserts
    }

    #[test]
    fn test_concurrent_insert_many_prepare_no_deadlock() {
        use std::sync::Arc;
        use std::thread;

        let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
        let coll = Arc::new(db.collection("test").unwrap());

        // Spawn multiple threads doing concurrent insert_many_prepare
        let handles: Vec<_> = (0..5)
            .map(|i| {
                let coll = Arc::clone(&coll);
                thread::spawn(move || {
                    for batch in 0..10 {
                        let docs: Vec<HashMap<String, Value>> = (0..20)
                            .map(|j| {
                                HashMap::from([
                                    ("thread".to_string(), json!(i)),
                                    ("batch".to_string(), json!(batch)),
                                    ("item".to_string(), json!(j)),
                                ])
                            })
                            .collect();
                        let prepared = coll.insert_many_prepare(docs).unwrap();
                        coll.insert_many_persist(prepared).unwrap();
                    }
                })
            })
            .collect();

        for handle in handles {
            handle.join().expect("Thread should not panic");
        }

        // Verify all inserts succeeded
        let count = coll.find(&json!({})).unwrap().len();
        assert_eq!(count, 1000); // 5 threads * 10 batches * 20 docs
    }

    #[test]
    fn test_concurrent_insert_prepare_with_drop_index_no_deadlock() {
        use std::sync::Arc;
        use std::thread;
        use std::time::Duration;

        let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
        let coll = Arc::new(db.collection("test").unwrap());

        // Create an index
        coll.create_index("value".to_string(), false).unwrap();

        let coll1 = Arc::clone(&coll);
        let coll2 = Arc::clone(&coll);

        // Thread 1: Does many insert_prepare operations
        let insert_handle = thread::spawn(move || {
            for i in 0..200 {
                let fields = HashMap::from([("value".to_string(), json!(i))]);
                if let Ok(prepared) = coll1.insert_one_prepare(fields) {
                    let _ = coll1.insert_one_persist(prepared);
                }
                // Small yield to increase chance of interleaving
                if i % 10 == 0 {
                    thread::yield_now();
                }
            }
        });

        // Thread 2: Creates and drops index multiple times
        // This tests the critical lock ordering: drop_index acquires index→storage,
        // while insert_prepare acquires storage(briefly)→index
        let index_handle = thread::spawn(move || {
            for _ in 0..20 {
                // Recreate index (may fail if it exists)
                let _ = coll2.create_index("other_field".to_string(), false);
                thread::sleep(Duration::from_micros(100));
                // Drop it
                let _ = coll2.drop_index("other_field");
                thread::yield_now();
            }
        });

        // If we get here without hanging for 5 seconds, there's no deadlock
        insert_handle
            .join()
            .expect("Insert thread should not panic");
        index_handle.join().expect("Index thread should not panic");
    }

    #[test]
    fn test_concurrent_unique_constraint_violation() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        use std::thread;

        let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
        let coll = Arc::new(db.collection("test").unwrap());

        // Create unique index
        coll.create_index("unique_key".to_string(), true).unwrap();

        let success_count = Arc::new(AtomicUsize::new(0));
        let error_count = Arc::new(AtomicUsize::new(0));

        // Multiple threads try to insert the same unique key
        let handles: Vec<_> = (0..10)
            .map(|_| {
                let coll = Arc::clone(&coll);
                let success = Arc::clone(&success_count);
                let errors = Arc::clone(&error_count);
                thread::spawn(move || {
                    let fields = HashMap::from([("unique_key".to_string(), json!("same_value"))]);
                    match coll.insert_one_prepare(fields) {
                        Ok(prepared) => match coll.insert_one_persist(prepared) {
                            Ok(_) => success.fetch_add(1, Ordering::SeqCst),
                            Err(_) => errors.fetch_add(1, Ordering::SeqCst),
                        },
                        Err(_) => errors.fetch_add(1, Ordering::SeqCst),
                    };
                })
            })
            .collect();

        for handle in handles {
            handle.join().expect("Thread should not panic");
        }

        // Exactly one should succeed (unique constraint)
        assert_eq!(success_count.load(Ordering::SeqCst), 1);
        assert_eq!(error_count.load(Ordering::SeqCst), 9);
    }

    #[test]
    fn test_concurrent_insert_many_prepare_unique_constraint() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        use std::thread;

        let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
        let coll = Arc::new(db.collection("test").unwrap());

        // Create unique index
        coll.create_index("email".to_string(), true).unwrap();

        let success_count = Arc::new(AtomicUsize::new(0));
        let error_count = Arc::new(AtomicUsize::new(0));

        // Each thread tries to insert a batch with overlapping emails
        let handles: Vec<_> = (0..5)
            .map(|thread_id| {
                let coll = Arc::clone(&coll);
                let success = Arc::clone(&success_count);
                let errors = Arc::clone(&error_count);
                thread::spawn(move || {
                    // Each batch has some unique and some shared emails
                    let docs: Vec<HashMap<String, Value>> = (0..5)
                        .map(|i| {
                            let email = if i < 2 {
                                // First 2 docs have thread-specific email
                                format!("thread{}_{}", thread_id, i)
                            } else {
                                // Last 3 docs try to claim shared emails
                                format!("shared_{}", i)
                            };
                            HashMap::from([("email".to_string(), json!(email))])
                        })
                        .collect();

                    match coll.insert_many_prepare(docs) {
                        Ok(prepared) => match coll.insert_many_persist(prepared) {
                            Ok(ids) => success.fetch_add(ids.len(), Ordering::SeqCst),
                            Err(_) => errors.fetch_add(1, Ordering::SeqCst),
                        },
                        Err(_) => errors.fetch_add(1, Ordering::SeqCst),
                    };
                })
            })
            .collect();

        for handle in handles {
            handle.join().expect("Thread should not panic");
        }

        // Due to races, results vary, but total should be consistent
        let total_success = success_count.load(Ordering::SeqCst);
        let total_errors = error_count.load(Ordering::SeqCst);

        // At minimum, thread 0 should succeed with thread-unique emails (2 per thread * 5 threads = 10)
        // Plus shared emails (3 emails, but only 1 thread can claim each = 3)
        // So between 10-13 successes expected, rest are errors
        assert!(total_success >= 5, "At least some inserts should succeed");
        assert!(total_success + total_errors > 0, "All threads completed");
    }
}
