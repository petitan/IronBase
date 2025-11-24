// ironbase-core/src/collection_core.rs
// Pure Rust collection logic - NO PyO3 dependencies
//
// FILE STRUCTURE (1,244 lines):
// ├── Constructor (lines 25-125)
// ├── CRUD Operations (lines 127-595)
// │   ├── insert_one, update_one, update_many
// │   ├── delete_one, delete_many
// │   └── distinct
// ├── Query Operations (lines 186-664)
// │   ├── find, find_one, count_documents
// │   ├── find_with_options, find_with_hint
// │   └── explain
// ├── Aggregation (lines 906-917)
// ├── Index Operations (lines 922-1004)
// │   ├── create_index, drop_index, list_indexes
// ├── Transaction Operations (lines 1012-1124)
// │   ├── insert_one_tx, update_one_tx, delete_one_tx
// └── Private Helpers (lines 1126-1244)
//     ├── read_document_by_id, scan_documents_via_catalog
//     ├── filter_documents, find_with_index
//     └── apply_update_operators
//
// FUTURE REFACTOR: See COLLECTION_DESIGN.md for modular architecture plan

use std::sync::Arc;

use parking_lot::RwLock;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

use crate::document::{Document, DocumentId};
use crate::error::{MongoLiteError, Result};
use crate::index::{IndexKey, IndexManager};
use crate::query::Query;
use crate::query_cache::{QueryCache, QueryHash};
use crate::query_planner::{QueryPlan, QueryPlanner};
use crate::storage::{RawStorage, Storage};
use crate::{log_debug, log_trace, log_warn};

mod index_persistence;
mod schema;

use self::index_persistence::persist_index_to_disk;
use self::schema::CompiledSchema;

/// Result of insert_many operation
#[derive(Debug, Clone)]
pub struct InsertManyResult {
    pub inserted_ids: Vec<DocumentId>,
    pub inserted_count: usize,
}

/// Pure Rust Collection - language-independent core logic
///
/// Generic over Storage backend:
/// - `CollectionCore<StorageEngine>` - Production file-based storage
/// - `CollectionCore<MemoryStorage>` - Fast in-memory storage for testing
///
/// Requires `RawStorage` for low-level document operations (write_document_raw, read_document_at)
pub struct CollectionCore<S: Storage + RawStorage> {
    pub name: String,
    pub storage: Arc<RwLock<S>>,
    /// Index manager for B+ tree indexes
    pub indexes: Arc<RwLock<IndexManager>>,
    /// Query result cache with LRU eviction (capacity: 1000 queries)
    pub query_cache: Arc<QueryCache>,
    schema: Arc<RwLock<Option<CompiledSchema>>>,
}

impl<S: Storage + RawStorage> CollectionCore<S> {
    // ========== CONSTRUCTOR ==========

    /// Create new collection (or get existing)
    pub fn new(name: String, storage: Arc<RwLock<S>>) -> Result<Self> {
        // Collection létrehozása, ha nem létezik
        {
            let mut storage_guard = storage.write();
            if storage_guard.get_collection_meta(&name).is_none() {
                storage_guard.create_collection(&name)?;
            }
        }

        // Initialize index manager with automatic _id index
        let mut index_manager = IndexManager::new();

        // Create automatic _id index (unique)
        let id_index_name = format!("{}_id", name);
        index_manager.create_btree_index(
            id_index_name.clone(),
            "_id".to_string(),
            true, // unique
        )?;

        // PERSISTENCE FIX: Load persisted indexes and rebuild from document catalog
        let schema_definition = {
            let storage_guard = storage.write();
            let meta = storage_guard
                .get_collection_meta(&name)
                .ok_or_else(|| MongoLiteError::CollectionNotFound(name.clone()))?;
            meta.schema.clone()
        };

        {
            let storage_guard = storage.write();
            let meta = storage_guard
                .get_collection_meta(&name)
                .ok_or_else(|| MongoLiteError::CollectionNotFound(name.clone()))?;

            // Clone metadata to avoid borrow issues
            let catalog = meta.document_catalog.clone();
            let persisted_indexes = meta.indexes.clone();

            log_debug!(
                "Collection '{}' - catalog size: {}, persisted indexes: {}",
                name,
                catalog.len(),
                persisted_indexes.len()
            );

            drop(storage_guard); // Release write lock before rebuilding

            // Load persisted custom indexes (if any)
            for index_meta in &persisted_indexes {
                // Skip _id index (already created)
                if index_meta.name == id_index_name {
                    continue;
                }

                log_debug!(
                    "Creating index '{}' on field '{}'",
                    index_meta.name,
                    index_meta.field
                );

                // Create index
                index_manager.create_btree_index(
                    index_meta.name.clone(),
                    index_meta.field.clone(),
                    index_meta.unique,
                )?;
            }

            // Rebuild all indexes from document catalog
            log_debug!(
                "Starting index rebuild from {} catalog entries",
                catalog.len()
            );
            let mut storage_guard = storage.write();
            let mut rebuilt_count = 0;
            for (_id_key, offset) in catalog.iter() {
                // Read document from disk (absolute offset)
                match storage_guard.read_document_at(&name, *offset) {
                    Ok(doc_bytes) => {
                        match serde_json::from_slice::<Value>(&doc_bytes) {
                            Ok(doc) => {
                                // Skip tombstones
                                if doc
                                    .get("_tombstone")
                                    .and_then(|v| v.as_bool())
                                    .unwrap_or(false)
                                {
                                    continue;
                                }

                                // Rebuild ALL indexes
                                if let Some(id_value) = doc.get("_id") {
                                    if let Ok(doc_id) =
                                        serde_json::from_value::<DocumentId>(id_value.clone())
                                    {
                                        // Rebuild _id index
                                        let index_key = IndexKey::from(id_value);
                                        if let Some(id_index) =
                                            index_manager.get_btree_index_mut(&id_index_name)
                                        {
                                            let _ = id_index.insert(index_key, doc_id.clone());
                                        }

                                        // Rebuild custom indexes
                                        for index_meta in &persisted_indexes {
                                            if index_meta.name == id_index_name {
                                                continue;
                                            }

                                            if let Some(field_value) = doc.get(&index_meta.field) {
                                                let key = IndexKey::from(field_value);
                                                if let Some(index) = index_manager
                                                    .get_btree_index_mut(&index_meta.name)
                                                {
                                                    let _ = index.insert(key, doc_id.clone());
                                                    rebuilt_count += 1;
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                log_warn!(
                                    "Failed to parse document JSON during index rebuild: {:?}",
                                    e
                                );
                                continue;
                            }
                        }
                    }
                    Err(e) => {
                        log_warn!(
                            "Failed to read document at offset during index rebuild: {:?}",
                            e
                        );
                        continue;
                    }
                }
            }
            log_debug!(
                "Index rebuild completed - {} index entries rebuilt",
                rebuilt_count
            );
        }

        let compiled_schema = if let Some(raw_schema) = schema_definition {
            Some(Self::compile_schema(&raw_schema)?)
        } else {
            None
        };

        Ok(CollectionCore {
            name,
            storage,
            indexes: Arc::new(RwLock::new(index_manager)),
            query_cache: Arc::new(QueryCache::new(1000)), // LRU cache with 1000 query capacity
            schema: Arc::new(RwLock::new(compiled_schema)),
        })
    }

    fn compile_schema(schema: &Value) -> Result<CompiledSchema> {
        CompiledSchema::from_value(schema)
    }

    fn validate_value_against_schema(&self, value: &Value) -> Result<()> {
        let guard = self.schema.read();
        if let Some(schema) = guard.as_ref() {
            schema.validate(value)?;
        }
        Ok(())
    }

    fn validate_document(&self, document: &Document) -> Result<()> {
        let value = serde_json::to_value(document)
            .map_err(|e| MongoLiteError::Serialization(e.to_string()))?;
        self.validate_value_against_schema(&value)
    }

    /// Set or clear the JSON schema for this collection.
    pub fn set_schema(&self, schema: Option<Value>) -> Result<()> {
        let compiled = if let Some(ref raw) = schema {
            Some(Self::compile_schema(raw)?)
        } else {
            None
        };

        {
            let mut storage = self.storage.write();
            let meta = storage
                .get_collection_meta_mut(&self.name)
                .ok_or_else(|| MongoLiteError::CollectionNotFound(self.name.clone()))?;
            meta.schema = schema;
            storage.flush()?;
        }

        let mut guard = self.schema.write();
        *guard = compiled;
        Ok(())
    }

    // ========== CRUD OPERATIONS ==========

    /// Insert one document - returns inserted DocumentId
    pub fn insert_one(&self, mut fields: HashMap<String, Value>) -> Result<DocumentId> {
        let mut storage = self.storage.write();

        // Get mutable reference to collection metadata
        let meta = storage
            .get_collection_meta_mut(&self.name)
            .ok_or_else(|| MongoLiteError::CollectionNotFound(self.name.clone()))?;

        // Check if _id already exists in fields
        let doc_id = if let Some(existing_id) = fields.get("_id") {
            // Use existing _id from fields
            serde_json::from_value(existing_id.clone())
                .map_err(|e| MongoLiteError::Serialization(format!("Invalid _id format: {}", e)))?
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
        {
            let mut indexes = self.indexes.write();

            // Update _id index
            let id_index_name = format!("{}_id", self.name);
            if let Some(id_index) = indexes.get_btree_index_mut(&id_index_name) {
                let id_key = match &doc_id {
                    DocumentId::Int(i) => IndexKey::Int(*i),
                    DocumentId::String(s) => IndexKey::String(s.clone()),
                    DocumentId::ObjectId(oid) => IndexKey::String(oid.clone()),
                };
                id_index.insert(id_key, doc_id.clone())?;
            }

            // Update all other indexes
            for index_name in indexes.list_indexes() {
                if index_name == id_index_name {
                    continue; // Already handled
                }

                if let Some(index) = indexes.get_btree_index_mut(&index_name) {
                    let field = &index.metadata.field;
                    if let Some(field_value) = doc.get(field) {
                        let index_key = IndexKey::from(field_value);
                        index.insert(index_key, doc_id.clone())?;
                    }
                }
            }
        }

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

    /// Insert many documents - optimized batch insert
    /// Returns InsertManyResult with all inserted document IDs
    pub fn insert_many(&self, documents: Vec<HashMap<String, Value>>) -> Result<InsertManyResult> {
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

        // Generate all IDs upfront
        let start_id = meta.last_id;
        meta.last_id += documents.len() as u64;

        // Prepare all documents with IDs
        let mut prepared_docs = Vec::with_capacity(documents.len());
        for (idx, mut fields) in documents.into_iter().enumerate() {
            // new_auto adds 1 internally, so we pass start_id + idx
            let doc_id = DocumentId::new_auto(start_id + idx as u64);

            // Add _id to fields
            fields.insert("_id".to_string(), serde_json::to_value(&doc_id).unwrap());

            // Add _collection field
            fields.insert("_collection".to_string(), Value::String(self.name.clone()));

            // Create document
            let doc = Document::new(doc_id.clone(), fields);
            self.validate_document(&doc)?;
            prepared_docs.push((doc_id.clone(), doc));
            inserted_ids.push(doc_id);
        }

        // Update indexes in batch BEFORE writing to storage
        {
            let mut indexes = self.indexes.write();
            let id_index_name = format!("{}_id", self.name);

            for (doc_id, doc) in &prepared_docs {
                // Update _id index
                if let Some(id_index) = indexes.get_btree_index_mut(&id_index_name) {
                    let id_key = match &doc_id {
                        DocumentId::Int(i) => IndexKey::Int(*i),
                        DocumentId::String(s) => IndexKey::String(s.clone()),
                        DocumentId::ObjectId(oid) => IndexKey::String(oid.clone()),
                    };
                    id_index.insert(id_key, doc_id.clone())?;
                }

                // Update all other indexes
                for index_name in indexes.list_indexes() {
                    if index_name == id_index_name {
                        continue;
                    }

                    if let Some(index) = indexes.get_btree_index_mut(&index_name) {
                        let field = &index.metadata.field;
                        if let Some(field_value) = doc.get(field) {
                            let index_key = IndexKey::from(field_value);
                            index.insert(index_key, doc_id.clone())?;
                        }
                    }
                }
            }
        }

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

    // ========== QUERY OPERATIONS ==========

    /// Find documents matching query
    pub fn find(&self, query_json: &Value) -> Result<Vec<Value>> {
        log_debug!("find() called with query: {:?}", query_json);

        let doc_ids = self.collect_doc_ids(query_json)?;
        let mut results = Vec::with_capacity(doc_ids.len());
        for doc_id in doc_ids {
            if let Some(doc) = self.read_document_by_id(&doc_id)? {
                results.push(doc);
            }
        }
        Ok(results)
    }

    /// Find documents with options (projection, sort, limit, skip)
    pub fn find_with_options(
        &self,
        query_json: &Value,
        options: crate::find_options::FindOptions,
    ) -> Result<Vec<Value>> {
        use crate::find_options::{apply_limit_skip, apply_projection, apply_sort};

        let original_skip = options.skip.unwrap_or(0);
        let original_limit = options.limit;

        let mut sort_field_ref: Option<&str> = None;
        let mut sort_desc = false;
        if let Some(ref sort_spec) = options.sort {
            if sort_spec.len() == 1 {
                sort_field_ref = Some(sort_spec[0].0.as_str());
                sort_desc = sort_spec[0].1 < 0;
            }
        }

        let apply_limit_after_sort = options.sort.is_some();
        let fetch_skip = if apply_limit_after_sort {
            0
        } else {
            original_skip
        };
        let fetch_limit = if apply_limit_after_sort {
            None
        } else {
            original_limit
        };

        let (doc_ids, index_sorted) = self.collect_doc_ids_with_options(
            query_json,
            None,
            sort_field_ref,
            sort_desc,
            fetch_skip,
            fetch_limit,
            sort_field_ref.is_none(),
        )?;

        let mut docs = Vec::with_capacity(doc_ids.len());
        for doc_id in doc_ids {
            if let Some(doc) = self.read_document_by_id(&doc_id)? {
                docs.push(doc);
            }
        }

        // 2. Apply sort
        if let Some(ref sort) = options.sort {
            if !(index_sorted && sort.len() == 1) {
                apply_sort(&mut docs, sort);
            }
        }

        // 3. Apply post-sort limit/skip if needed
        if apply_limit_after_sort {
            docs = apply_limit_skip(docs, original_limit, options.skip);
        }

        // 4. Apply projection
        if let Some(ref projection) = options.projection {
            docs = docs
                .into_iter()
                .map(|doc| apply_projection(&doc, projection))
                .collect();
        }

        Ok(docs)
    }

    /// Streaming cursor for large result sets
    pub fn find_streaming(&self, query_json: &Value) -> Result<FindCursor<'_, S>> {
        let (doc_ids, _) =
            self.collect_doc_ids_with_options(query_json, None, None, false, 0, None, true)?;
        Ok(FindCursor {
            collection: self,
            doc_ids,
            position: 0,
        })
    }

    /// Find one document matching query
    pub fn find_one(&self, query_json: &Value) -> Result<Option<Value>> {
        let parsed_query = Query::from_json(query_json)?;

        // OPTIMIZATION: Check if this is an _id equality query (O(1) lookup)
        if let Some(query_obj) = query_json.as_object() {
            if query_obj.len() == 1 && query_obj.contains_key("_id") {
                if let Some(id_val) = query_obj.get("_id") {
                    // Direct O(1) lookup using document_catalog (direct DocumentId conversion!)
                    if let Ok(doc_id) = serde_json::from_value::<DocumentId>(id_val.clone()) {
                        if let Some(doc) = self.read_document_by_id(&doc_id)? {
                            // Verify query still matches (for consistency)
                            let doc_json_str = serde_json::to_string(&doc)?;
                            let document = Document::from_json(&doc_json_str)?;

                            if parsed_query.matches(&document) {
                                return Ok(Some(doc));
                            }
                        }
                    }
                    return Ok(None);
                }
            }
        }

        // Fallback: Full scan using catalog iteration (still faster than file scan)
        let docs_by_id = self.scan_documents_via_catalog()?;

        // Find first matching document (skip tombstones)
        for (_, doc) in docs_by_id {
            let doc_json_str = match serde_json::to_string(&doc) {
                Ok(json) => json,
                Err(_) => continue,
            };
            let document = match Document::from_json(&doc_json_str) {
                Ok(doc) => doc,
                Err(_) => continue,
            };

            if parsed_query.matches(&document) {
                return Ok(Some(doc));
            }
        }

        Ok(None)
    }

    /// Count documents matching query
    pub fn count_documents(&self, query_json: &Value) -> Result<u64> {
        if Self::query_matches_all(query_json) {
            let storage = self.storage.read();
            return Ok(storage.get_live_count(&self.name).unwrap_or(0));
        }

        if let Some(doc_id) = Self::extract_id_query(query_json) {
            return Ok(if self.read_document_by_id(&doc_id)?.is_some() {
                1
            } else {
                0
            });
        }

        let parsed_query = Query::from_json(query_json)?;

        // OPTIMIZATION: Use catalog iteration instead of full file scan
        let docs_by_id = self.scan_documents_via_catalog()?;

        // Count matching documents (skip tombstones already filtered by catalog scan)
        let mut count = 0u64;
        for (_, doc) in docs_by_id {
            let doc_json_str = serde_json::to_string(&doc)?;
            let document = Document::from_json(&doc_json_str)?;

            if parsed_query.matches(&document) {
                count += 1;
            }
        }

        Ok(count)
    }

    /// Update one document - returns (matched_count, modified_count)
    pub fn update_one(&self, query_json: &Value, update_json: &Value) -> Result<(u64, u64)> {
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

                // Apply update operators
                let was_modified = self.apply_update_operators(&mut document, update_json)?;

                if was_modified {
                    // Mark old document as tombstone
                    let mut tombstone = doc.clone();
                    if let Value::Object(ref mut map) = tombstone {
                        map.insert("_tombstone".to_string(), Value::Bool(true));
                        map.insert("_collection".to_string(), Value::String(self.name.clone()));
                    }
                    let tombstone_json = serde_json::to_string(&tombstone)?;

                    // Write tombstone (no catalog tracking for tombstones)
                    storage.write_data(tombstone_json.as_bytes())?;

                    // ✅ Ensure updated document has _collection
                    document.set("_collection".to_string(), Value::String(self.name.clone()));
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

    /// Update many documents - returns (matched_count, modified_count)
    pub fn update_many(&self, query_json: &Value, update_json: &Value) -> Result<(u64, u64)> {
        let parsed_query = Query::from_json(query_json)?;

        // Catalog-based scan to get latest version per _id
        let docs_by_id = self.scan_documents_via_catalog()?;

        // Second pass: find all matching and update (skip tombstones)
        let mut matched = 0u64;
        let mut modified = 0u64;

        let mut storage = self.storage.write();

        for (_, doc) in docs_by_id {
            // Skip tombstones (deleted documents)
            if doc
                .get("_tombstone")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                continue;
            }

            // Deserialize with proper _id handling
            let doc_json_str = serde_json::to_string(&doc)?;
            let mut document = Document::from_json(&doc_json_str)?;
            let doc_id = document.id.clone();

            // Check if matches query
            if parsed_query.matches(&document) {
                matched += 1;

                // Apply update operators
                let was_modified = self.apply_update_operators(&mut document, update_json)?;

                if was_modified {
                    // Mark old document as tombstone
                    let mut tombstone = doc.clone();
                    if let Value::Object(ref mut map) = tombstone {
                        map.insert("_tombstone".to_string(), Value::Bool(true));
                        map.insert("_collection".to_string(), Value::String(self.name.clone()));
                    }
                    let tombstone_json = serde_json::to_string(&tombstone)?;

                    // Write tombstone (no catalog tracking for tombstones)
                    storage.write_data(tombstone_json.as_bytes())?;

                    // ✅ Ensure updated document has _collection
                    document.set("_collection".to_string(), Value::String(self.name.clone()));
                    self.validate_document(&document)?;

                    // Write updated document WITH catalog tracking
                    let updated_json = document.to_json()?;
                    storage.write_document_raw(&self.name, &doc_id, updated_json.as_bytes())?;
                    storage.adjust_live_count(&self.name, -1);
                    storage.adjust_live_count(&self.name, 1);

                    modified += 1;
                }
            }
        }

        // Invalidate query cache if any document was modified
        if modified > 0 {
            self.query_cache.invalidate_collection(&self.name);
        }

        Ok((matched, modified))
    }

    /// Delete one document - returns deleted_count
    pub fn delete_one(&self, query_json: &Value) -> Result<u64> {
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

    /// Delete many documents - returns deleted_count
    pub fn delete_many(&self, query_json: &Value) -> Result<u64> {
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

    /// Distinct values for a field
    pub fn distinct(&self, field: &str, query_json: &Value) -> Result<Vec<Value>> {
        if let Some(doc_id) = Self::extract_id_query(query_json) {
            if let Some(doc) = self.read_document_by_id(&doc_id)? {
                if let Some(value) = doc.get(field) {
                    return Ok(vec![value.clone()]);
                }
            }
            return Ok(Vec::new());
        }

        let match_all = Self::query_matches_all(query_json);
        let parsed_query = if match_all {
            None
        } else {
            Some(Query::from_json(query_json)?)
        };

        let docs_by_id = self.scan_documents_via_catalog()?;

        // Collect distinct values from matching documents (skip tombstones)
        let mut seen_values: HashSet<String> = HashSet::new();
        let mut distinct_values = Vec::new();

        for (_, doc) in docs_by_id {
            // Skip tombstones (deleted documents)
            if doc
                .get("_tombstone")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                continue;
            }

            let matches = if let Some(query) = &parsed_query {
                let doc_json_str = serde_json::to_string(&doc)?;
                let document = Document::from_json(&doc_json_str)?;
                query.matches(&document)
            } else {
                true
            };

            if matches {
                if let Some(field_value) = doc.get(field) {
                    let value_key =
                        serde_json::to_string(field_value).unwrap_or_else(|_| "null".to_string());

                    if seen_values.insert(value_key) {
                        distinct_values.push(field_value.clone());
                    }
                }
            }
        }

        Ok(distinct_values)
    }

    // ========== PRIVATE HELPER METHODS ==========

    /// Extract field name from index name (e.g., "users_age" -> "age")
    fn extract_field_from_index_name(&self, index_name: &str) -> String {
        // Remove collection prefix: "users_age" -> "age"
        let prefix = format!("{}_", self.name);
        index_name
            .strip_prefix(&prefix)
            .unwrap_or(index_name)
            .to_string()
    }

    /// Create a query plan for a hinted index
    fn create_plan_for_hint(
        &self,
        query_json: &Value,
        index_name: &str,
        field: &str,
    ) -> Result<QueryPlan> {
        // Parse the query to understand what we're looking for
        if let Value::Object(ref map) = query_json {
            // Check if querying this field
            if let Some(value) = map.get(field) {
                // Check for operators
                if let Value::Object(ref ops) = value {
                    // Range query
                    let has_gt = ops.contains_key("$gt");
                    let has_gte = ops.contains_key("$gte");
                    let has_lt = ops.contains_key("$lt");
                    let has_lte = ops.contains_key("$lte");

                    if has_gt || has_gte || has_lt || has_lte {
                        let start = if has_gte {
                            ops.get("$gte").map(IndexKey::from)
                        } else if has_gt {
                            ops.get("$gt").map(IndexKey::from)
                        } else {
                            None
                        };

                        let end = if has_lte {
                            ops.get("$lte").map(IndexKey::from)
                        } else if has_lt {
                            ops.get("$lt").map(IndexKey::from)
                        } else {
                            None
                        };

                        return Ok(QueryPlan::IndexRangeScan {
                            index_name: index_name.to_string(),
                            field: field.to_string(),
                            start,
                            end,
                            inclusive_start: has_gte || (!has_gt && !has_gte),
                            inclusive_end: has_lte || (!has_lt && !has_lte),
                        });
                    }
                }

                // Equality query
                let key = IndexKey::from(value);
                return Ok(QueryPlan::IndexScan {
                    index_name: index_name.to_string(),
                    field: field.to_string(),
                    key,
                });
            }
        }

        Err(MongoLiteError::IndexError(format!(
            "Cannot use index '{}' for this query",
            index_name
        )))
    }

    /// Execute query using an index
    fn find_with_index(&self, parsed_query: Query, plan: QueryPlan) -> Result<Vec<Value>> {
        let (doc_ids, _) =
            self.collect_doc_ids_from_plan(&parsed_query, plan, None, false, 0, None)?;
        let mut results = Vec::with_capacity(doc_ids.len());
        for doc_id in doc_ids {
            if let Some(doc) = self.read_document_by_id(&doc_id)? {
                results.push(doc);
            }
        }
        Ok(results)
    }

    /// Apply update operators to document - returns whether document was modified
    fn apply_update_operators(&self, document: &mut Document, update_json: &Value) -> Result<bool> {
        let mut was_modified = false;

        if let Value::Object(ref update_ops) = update_json {
            for (op, fields) in update_ops {
                match op.as_str() {
                    "$set" => {
                        if let Value::Object(ref field_values) = fields {
                            for (field, value) in field_values {
                                document.set(field.clone(), value.clone());
                                was_modified = true;
                            }
                        }
                    }
                    "$inc" => {
                        if let Value::Object(ref field_values) = fields {
                            for (field, inc_value) in field_values {
                                if let Some(current) = document.get(field) {
                                    // Try int first to preserve integer types
                                    if let (Some(curr_int), Some(inc_int)) =
                                        (current.as_i64(), inc_value.as_i64())
                                    {
                                        document
                                            .set(field.clone(), Value::from(curr_int + inc_int));
                                        was_modified = true;
                                    } else if let (Some(curr_num), Some(inc_num)) =
                                        (current.as_f64(), inc_value.as_f64())
                                    {
                                        document
                                            .set(field.clone(), Value::from(curr_num + inc_num));
                                        was_modified = true;
                                    }
                                }
                            }
                        }
                    }
                    "$unset" => {
                        if let Value::Object(ref field_values) = fields {
                            for (field, _) in field_values {
                                document.remove(field);
                                was_modified = true;
                            }
                        }
                    }
                    "$push" => {
                        if let Value::Object(ref field_values) = fields {
                            for (field, value) in field_values {
                                // Handle modifiers: $each, $position, $slice
                                let (items, position, slice) = if let Value::Object(ref modifiers) =
                                    value
                                {
                                    let items = if let Some(each_val) = modifiers.get("$each") {
                                        // $each: push multiple items
                                        if let Value::Array(ref arr) = each_val {
                                            arr.clone()
                                        } else {
                                            vec![each_val.clone()]
                                        }
                                    } else {
                                        // No $each, treat entire value as single item
                                        vec![value.clone()]
                                    };

                                    let position = modifiers
                                        .get("$position")
                                        .and_then(|v| v.as_i64())
                                        .map(|p| p as usize);

                                    let slice = modifiers.get("$slice").and_then(|v| v.as_i64());

                                    (items, position, slice)
                                } else {
                                    // Simple push: single value
                                    (vec![value.clone()], None, None)
                                };

                                // Get or create array
                                let mut array = match document.get(field) {
                                    Some(Value::Array(arr)) => arr.clone(),
                                    Some(_) => {
                                        return Err(MongoLiteError::InvalidQuery(format!(
                                            "$push: field '{}' is not an array",
                                            field
                                        )));
                                    }
                                    None => vec![],
                                };

                                // Insert items at position or append
                                if let Some(pos) = position {
                                    let insert_pos = pos.min(array.len());
                                    for (i, item) in items.into_iter().enumerate() {
                                        array.insert(insert_pos + i, item);
                                    }
                                } else {
                                    array.extend(items);
                                }

                                // Apply $slice if specified
                                if let Some(slice_val) = slice {
                                    if slice_val < 0 {
                                        // Keep last N elements
                                        let keep = (-slice_val) as usize;
                                        let len = array.len();
                                        if len > keep {
                                            array = array.into_iter().skip(len - keep).collect();
                                        }
                                    } else {
                                        // Keep first N elements
                                        array.truncate(slice_val as usize);
                                    }
                                }

                                document.set(field.clone(), Value::Array(array));
                                was_modified = true;
                            }
                        }
                    }
                    "$pull" => {
                        if let Value::Object(ref field_values) = fields {
                            for (field, condition) in field_values {
                                if let Some(Value::Array(ref arr)) = document.get(field) {
                                    // Filter out matching elements
                                    let filtered: Vec<Value> = arr
                                        .iter()
                                        .filter(|item| {
                                            !self.value_matches_condition(item, condition)
                                        })
                                        .cloned()
                                        .collect();

                                    if filtered.len() != arr.len() {
                                        document.set(field.clone(), Value::Array(filtered));
                                        was_modified = true;
                                    }
                                } else if document.get(field).is_some() {
                                    return Err(MongoLiteError::InvalidQuery(format!(
                                        "$pull: field '{}' is not an array",
                                        field
                                    )));
                                }
                            }
                        }
                    }
                    "$addToSet" => {
                        if let Value::Object(ref field_values) = fields {
                            for (field, value) in field_values {
                                // Handle $each modifier
                                let items = if let Value::Object(ref modifiers) = value {
                                    if let Some(each_val) = modifiers.get("$each") {
                                        if let Value::Array(ref arr) = each_val {
                                            arr.clone()
                                        } else {
                                            vec![each_val.clone()]
                                        }
                                    } else {
                                        vec![value.clone()]
                                    }
                                } else {
                                    vec![value.clone()]
                                };

                                // Get or create array
                                let mut array = match document.get(field) {
                                    Some(Value::Array(arr)) => arr.clone(),
                                    Some(_) => {
                                        return Err(MongoLiteError::InvalidQuery(format!(
                                            "$addToSet: field '{}' is not an array",
                                            field
                                        )));
                                    }
                                    None => vec![],
                                };

                                // Add items if not already present
                                for item in items {
                                    if !array.contains(&item) {
                                        array.push(item);
                                        was_modified = true;
                                    }
                                }

                                document.set(field.clone(), Value::Array(array));
                            }
                        }
                    }
                    "$pop" => {
                        if let Value::Object(ref field_values) = fields {
                            for (field, direction) in field_values {
                                if let Some(Value::Array(ref arr)) = document.get(field) {
                                    if arr.is_empty() {
                                        continue; // No-op on empty array
                                    }

                                    let mut new_array = arr.clone();

                                    // -1 = remove first, 1 = remove last
                                    match direction.as_i64() {
                                        Some(-1) => {
                                            new_array.remove(0);
                                            was_modified = true;
                                        }
                                        Some(1) => {
                                            new_array.pop();
                                            was_modified = true;
                                        }
                                        _ => {
                                            return Err(MongoLiteError::InvalidQuery(format!(
                                                "$pop: value must be -1 or 1, got {:?}",
                                                direction
                                            )));
                                        }
                                    }

                                    document.set(field.clone(), Value::Array(new_array));
                                } else if document.get(field).is_some() {
                                    return Err(MongoLiteError::InvalidQuery(format!(
                                        "$pop: field '{}' is not an array",
                                        field
                                    )));
                                }
                            }
                        }
                    }
                    _ => {
                        return Err(MongoLiteError::InvalidQuery(format!(
                            "Unsupported update operator: {}",
                            op
                        )));
                    }
                }
            }
        }

        Ok(was_modified)
    }

    /// Helper function for $pull: check if a value matches a condition
    ///
    /// Supports:
    /// - Direct equality: `{"tags": "obsolete"}` removes "obsolete"
    /// - Query operators: `{"score": {"$lt": 5}}` removes items < 5
    fn value_matches_condition(&self, value: &Value, condition: &Value) -> bool {
        // If condition is an object with operators, evaluate them
        if let Value::Object(ref cond_obj) = condition {
            // Check if it contains query operators
            let has_operators = cond_obj.keys().any(|k| k.starts_with('$'));

            if has_operators {
                // Evaluate query operators
                for (op, op_value) in cond_obj {
                    match op.as_str() {
                        "$eq" => {
                            if value != op_value {
                                return false;
                            }
                        }
                        "$ne" => {
                            if value == op_value {
                                return false;
                            }
                        }
                        "$gt" => {
                            use std::cmp::Ordering;
                            if !Self::compare_values(value, op_value)
                                .map(|cmp| cmp == Ordering::Greater)
                                .unwrap_or(false)
                            {
                                return false;
                            }
                        }
                        "$gte" => {
                            use std::cmp::Ordering;
                            if !Self::compare_values(value, op_value)
                                .map(|cmp| matches!(cmp, Ordering::Greater | Ordering::Equal))
                                .unwrap_or(false)
                            {
                                return false;
                            }
                        }
                        "$lt" => {
                            use std::cmp::Ordering;
                            if !Self::compare_values(value, op_value)
                                .map(|cmp| cmp == Ordering::Less)
                                .unwrap_or(false)
                            {
                                return false;
                            }
                        }
                        "$lte" => {
                            use std::cmp::Ordering;
                            if !Self::compare_values(value, op_value)
                                .map(|cmp| matches!(cmp, Ordering::Less | Ordering::Equal))
                                .unwrap_or(false)
                            {
                                return false;
                            }
                        }
                        "$in" => {
                            if let Value::Array(ref arr) = op_value {
                                if !arr.contains(value) {
                                    return false;
                                }
                            }
                        }
                        "$nin" => {
                            if let Value::Array(ref arr) = op_value {
                                if arr.contains(value) {
                                    return false;
                                }
                            }
                        }
                        _ => {} // Unknown operator, ignore
                    }
                }
                return true; // All operators matched
            }
        }

        // Direct equality comparison
        value == condition
    }

    /// Helper to compare two JSON values for ordering
    fn compare_values(a: &Value, b: &Value) -> Option<std::cmp::Ordering> {
        match (a, b) {
            (Value::Number(n1), Value::Number(n2)) => {
                let f1 = n1.as_f64()?;
                let f2 = n2.as_f64()?;
                f1.partial_cmp(&f2)
            }
            (Value::String(s1), Value::String(s2)) => Some(s1.cmp(s2)),
            (Value::Bool(b1), Value::Bool(b2)) => Some(b1.cmp(b2)),
            _ => None,
        }
    }

    // ========== QUERY OPTIMIZATION OPERATIONS ==========

    /// Explain query execution plan without executing
    pub fn explain(&self, query_json: &Value) -> Result<Value> {
        let indexes = self.indexes.read();
        let available_indexes = indexes.list_indexes();

        let plan = QueryPlanner::explain_query(query_json, &available_indexes);
        Ok(plan)
    }

    /// Find with manual index hint
    pub fn find_with_hint(&self, query_json: &Value, hint: &str) -> Result<Vec<Value>> {
        let parsed_query = Query::from_json(query_json)?;

        // Verify hint index exists
        {
            let indexes = self.indexes.read();
            if indexes.get_btree_index(hint).is_none() {
                return Err(MongoLiteError::IndexError(format!(
                    "Index '{}' not found (hint)",
                    hint
                )));
            }
        }

        // Try to create a plan using the hinted index
        // For now, we try to match the query to the index field
        let field = self.extract_field_from_index_name(hint);

        // Create a forced plan
        let plan = self.create_plan_for_hint(query_json, hint, &field)?;

        // Execute with the forced plan
        self.find_with_index(parsed_query, plan)
    }

    // ========== AGGREGATION ==========

    /// Execute aggregation pipeline
    ///
    /// # Arguments
    /// * `pipeline_json` - JSON array of pipeline stages
    ///
    /// # Example
    /// ```no_run
    /// use ironbase_core::{DatabaseCore, Document};
    /// use serde_json::json;
    ///
    /// let db = DatabaseCore::open("test.db").unwrap();
    /// let collection = db.collection("users").unwrap();
    ///
    /// let results = collection.aggregate(&json!([
    ///     {"$match": {"age": {"$gte": 18}}},
    ///     {"$group": {"_id": "$city", "count": {"$sum": 1}}},
    ///     {"$sort": {"count": -1}}
    /// ])).unwrap();
    /// ```
    pub fn aggregate(&self, pipeline_json: &Value) -> Result<Vec<Value>> {
        use crate::aggregation::Pipeline;

        // Parse pipeline
        let pipeline = Pipeline::from_json(pipeline_json)?;

        // OPTIMIZATION: Use index if $match is first stage (query optimizer)
        //
        // Current: Always full collection scan (self.find on empty query)
        // Impact: Aggregation pipelines with selective $match are slow
        //
        // Index-based optimization:
        // 1. Check if pipeline[0] is $match stage
        // 2. Extract query from $match (e.g., {"age": {"$gt": 25}})
        // 3. Use query optimizer to select best index (see IMPLEMENTATION_QUERY_OPTIMIZER.md)
        // 4. Use index.search() or range_scan() to get filtered doc IDs
        // 5. Load only matching documents (not entire collection)
        //
        // Benefit: 10-1000x speedup for selective aggregations
        // Example: db.collection.aggregate([{$match: {email: "foo@bar.com"}}, {$group: ...}])
        //          - Without index: scan 650K docs → 33 seconds
        //          - With index: 1 B+ tree lookup → <1ms
        //
        // Prerequisites:
        // - Index child loading (index.rs:195 - documented in commit 90045d8)
        // - Query optimizer (see IMPLEMENTATION_QUERY_OPTIMIZER.md)
        // - Range scan support (B+ tree leaf sibling pointers)
        //
        // Priority: Medium (correctness unaffected, but significant performance gain)
        let docs = self.find(&serde_json::json!({}))?;

        // Execute pipeline
        pipeline.execute(docs)
    }

    // ========== INDEX OPERATIONS ==========

    /// Create a B+ tree index on a field
    pub fn create_index(&self, field: String, unique: bool) -> Result<String> {
        let index_name = format!("{}_{}", self.name, field);

        let mut indexes = self.indexes.write();
        indexes.create_btree_index(index_name.clone(), field.clone(), unique)?;

        // Populate index with existing documents
        let docs_by_id = {
            drop(indexes); // Release write lock before acquiring storage lock
            self.scan_documents_via_catalog()?
        };

        // Re-acquire write lock to populate index
        let mut indexes = self.indexes.write();

        for (doc_id, doc) in &docs_by_id {
            // Extract field value and add to index (no DocumentId parsing needed!)
            if let Some(field_value) = doc.get(&field) {
                let key = IndexKey::from(field_value);

                if let Some(index) = indexes.get_btree_index_mut(&index_name) {
                    let _ = index.insert(key, doc_id.clone());
                }
            }
        }

        drop(indexes); // Release index lock

        // PERSIST index metadata to collection metadata
        {
            let mut storage = self.storage.write();
            if let Some(meta) = storage.get_collection_meta_mut(&self.name) {
                // Create IndexMetadata
                use crate::index::IndexMetadata;
                let index_meta = IndexMetadata {
                    name: index_name.clone(),
                    field: field.clone(),
                    unique,
                    sparse: false,
                    num_keys: 0,
                    tree_height: 1,
                    root_offset: 0,
                };

                // Add to persisted indexes list
                meta.indexes.push(index_meta);

                // Save metadata to disk
                storage.flush()?;

                // PERSIST index data to .idx file
                let db_file_path = storage.get_file_path().to_string();
                drop(storage); // Release storage lock before acquiring index lock

                if !db_file_path.is_empty() {
                    let mut indexes = self.indexes.write();
                    if let Some(index) = indexes.get_btree_index_mut(&index_name) {
                        persist_index_to_disk(&db_file_path, &index_name, |file| {
                            index.save_to_file(file)
                        })?;
                    }
                }
            }
        }

        Ok(index_name)
    }

    /// Drop an index
    pub fn drop_index(&self, index_name: &str) -> Result<()> {
        let mut indexes = self.indexes.write();
        indexes.drop_index(index_name)?;

        drop(indexes); // Release lock

        // Remove from persisted metadata
        {
            let mut storage = self.storage.write();
            if let Some(meta) = storage.get_collection_meta_mut(&self.name) {
                meta.indexes.retain(|idx| idx.name != index_name);
                storage.flush()?;
            }
        }

        Ok(())
    }

    /// List all indexes
    pub fn list_indexes(&self) -> Vec<String> {
        let indexes = self.indexes.read();
        indexes.list_indexes()
    }

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
            .ok_or_else(|| MongoLiteError::CollectionNotFound(self.name.clone()))?;

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
        tx.add_operation(Operation::Insert {
            collection: self.name.clone(),
            doc_id: doc_id.clone(),
            doc: serde_json::json!(doc_with_id),
        })?;

        // Track index changes for two-phase commit
        let indexes = self.indexes.read();
        for index_name in indexes.list_indexes() {
            // Get the index to extract field name
            if let Some(btree_index) = indexes.get_btree_index(&index_name) {
                let field_name = &btree_index.metadata.field;

                // Get the field value from the document
                if let Some(key_value) = doc_with_id.get(field_name) {
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
            let id_value = old_doc
                .get("_id")
                .ok_or_else(|| MongoLiteError::DocumentNotFound)?;

            let doc_id = match id_value {
                Value::Number(n) if n.is_i64() => DocumentId::Int(n.as_i64().unwrap()),
                Value::Number(n) if n.is_u64() => DocumentId::Int(n.as_u64().unwrap() as i64),
                Value::String(s) => DocumentId::String(s.clone()),
                _ => {
                    return Err(MongoLiteError::Serialization(
                        "Invalid _id type".to_string(),
                    ))
                }
            };

            // Ensure new_doc has _id and _collection fields
            let new_doc_with_meta = if let Value::Object(mut map) = new_doc {
                map.insert("_id".to_string(), id_value.clone());
                map.insert("_collection".to_string(), Value::String(self.name.clone()));
                Value::Object(map)
            } else {
                return Err(MongoLiteError::Serialization(
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
                    let old_value = old_doc.get(field_name);
                    let new_value = if let Value::Object(ref map) = new_doc_for_tracking {
                        map.get(field_name)
                    } else {
                        None
                    };

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
            let id_value = old_doc
                .get("_id")
                .ok_or_else(|| MongoLiteError::DocumentNotFound)?;

            let doc_id = match id_value {
                Value::Number(n) if n.is_i64() => DocumentId::Int(n.as_i64().unwrap()),
                Value::Number(n) if n.is_u64() => DocumentId::Int(n.as_u64().unwrap() as i64),
                Value::String(s) => DocumentId::String(s.clone()),
                _ => {
                    return Err(MongoLiteError::Serialization(
                        "Invalid _id type".to_string(),
                    ))
                }
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
                    if let Some(old_val) = old_doc.get(field_name) {
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

    // ========== PRIVATE HELPER METHODS ==========
    // These methods provide internal utility functions for CRUD and query operations

    /// Read a single document by _id using document_catalog (O(1) lookup)
    /// Returns None if document not found or is tombstone
    fn read_document_by_id(&self, doc_id: &DocumentId) -> Result<Option<Value>> {
        let mut storage = self.storage.write();
        let meta = storage
            .get_collection_meta(&self.name)
            .ok_or_else(|| MongoLiteError::CollectionNotFound(self.name.clone()))?;

        log_trace!(
            "read_document_by_id({:?}) - catalog has {} entries",
            doc_id,
            meta.document_catalog.len()
        );

        // O(1) lookup in document_catalog (direct DocumentId lookup - no serialization!)
        if let Some(&offset) = meta.document_catalog.get(doc_id) {
            log_trace!("Found doc_id {:?} at offset {}", doc_id, offset);
            let doc_bytes = storage.read_data(offset)?;
            let doc: Value = serde_json::from_slice(&doc_bytes)?;

            // Check if document is a tombstone (deleted)
            if doc
                .get("_tombstone")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                log_trace!("Document is tombstone");
                return Ok(None);
            }

            Ok(Some(doc))
        } else {
            log_trace!(
                "doc_id {:?} NOT in catalog! Catalog keys: {:?}",
                doc_id,
                meta.document_catalog.keys().collect::<Vec<_>>()
            );
            Ok(None)
        }
    }

    /// Scan documents via document_catalog instead of full file scan
    /// Much faster than scan_documents() for large collections
    fn scan_documents_via_catalog(&self) -> Result<HashMap<DocumentId, Value>> {
        let mut storage = self.storage.write();

        // Clone the catalog to avoid borrow checker issues
        let catalog = {
            let meta = storage
                .get_collection_meta(&self.name)
                .ok_or_else(|| MongoLiteError::CollectionNotFound(self.name.clone()))?;
            log_debug!(
                "Collection '{}' has {} documents in catalog",
                self.name,
                meta.document_catalog.len()
            );
            meta.document_catalog.clone()
        };

        let mut docs_by_id: HashMap<DocumentId, Value> = HashMap::new();

        // Iterate over catalog instead of sequential file scan (direct DocumentId iteration!)
        for (doc_id, offset) in &catalog {
            match storage.read_data(*offset) {
                Ok(doc_bytes) => {
                    // Try to deserialize JSON - skip if corrupt
                    match serde_json::from_slice::<Value>(&doc_bytes) {
                        Ok(doc) => {
                            // Skip tombstones (deleted documents)
                            if !doc
                                .get("_tombstone")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false)
                            {
                                docs_by_id.insert(doc_id.clone(), doc);
                            }
                        }
                        Err(_) => continue, // Skip corrupted JSON
                    }
                }
                Err(_) => continue, // Skip corrupted entries
            }
        }

        Ok(docs_by_id)
    }

    fn collect_doc_ids(&self, query_json: &Value) -> Result<Vec<DocumentId>> {
        let (ids, _) =
            self.collect_doc_ids_with_options(query_json, None, None, false, 0, None, true)?;
        Ok(ids)
    }

    fn collect_doc_ids_with_options(
        &self,
        query_json: &Value,
        hint: Option<&str>,
        sort_field: Option<&str>,
        sort_desc: bool,
        skip: usize,
        limit: Option<usize>,
        use_cache: bool,
    ) -> Result<(Vec<DocumentId>, bool)> {
        let cache_hash = if use_cache
            && hint.is_none()
            && sort_field.is_none()
            && skip == 0
            && limit.is_none()
        {
            Some(QueryHash::new(&self.name, query_json))
        } else {
            None
        };

        if let Some(hash) = cache_hash {
            if let Some(cached) = self.query_cache.get(&hash) {
                return Ok((cached, false));
            }
        }

        let parsed_query = Query::from_json(query_json)?;

        let plan = if let Some(hint_name) = hint {
            let field = self.extract_field_from_index_name(hint_name);
            Some(self.create_plan_for_hint(query_json, hint_name, &field)?)
        } else {
            let indexes = self.indexes.read();
            let available_indexes = indexes.list_indexes();
            drop(indexes);
            QueryPlanner::analyze_query(query_json, &available_indexes).map(|(_, plan)| plan)
        };

        let (doc_ids_vec, used_sort) = if let Some(plan) = plan {
            self.collect_doc_ids_from_plan(&parsed_query, plan, sort_field, sort_desc, skip, limit)?
        } else {
            // Fallback to full scan using catalog
            let docs_by_id = self.scan_documents_via_catalog()?;
            log_debug!(
                "scan_documents_via_catalog returned {} documents",
                docs_by_id.len()
            );
            let mut doc_ids = Vec::new();
            let mut skipped = 0usize;

            for (doc_id, doc) in docs_by_id {
                let doc_json_str = serde_json::to_string(&doc)?;
                let document = Document::from_json(&doc_json_str)?;

                if parsed_query.matches(&document) {
                    if skipped < skip {
                        skipped += 1;
                        continue;
                    }
                    doc_ids.push(doc_id.clone());
                    if let Some(limit_count) = limit {
                        if doc_ids.len() >= limit_count {
                            break;
                        }
                    }
                }
            }

            (doc_ids, false)
        };

        if let Some(hash) = cache_hash {
            self.query_cache.insert(hash, doc_ids_vec.clone());
        }

        Ok((doc_ids_vec, used_sort))
    }

    fn collect_doc_ids_from_plan(
        &self,
        parsed_query: &Query,
        plan: QueryPlan,
        sort_field: Option<&str>,
        sort_desc: bool,
        skip: usize,
        limit: Option<usize>,
    ) -> Result<(Vec<DocumentId>, bool)> {
        let mut doc_ids = {
            let indexes = self.indexes.read();
            match plan {
                QueryPlan::IndexScan {
                    ref index_name,
                    ref key,
                    ..
                } => {
                    if let Some(index) = indexes.get_btree_index(index_name) {
                        index.range_scan(key, key, true, true)
                    } else {
                        vec![]
                    }
                }
                QueryPlan::IndexRangeScan {
                    ref index_name,
                    ref start,
                    ref end,
                    inclusive_start,
                    inclusive_end,
                    ..
                } => {
                    if let Some(index) = indexes.get_btree_index(index_name) {
                        let default_start = IndexKey::Null;
                        let default_end = IndexKey::String("\u{10ffff}".repeat(100));

                        let start_key = start.as_ref().unwrap_or(&default_start);
                        let end_key = end.as_ref().unwrap_or(&default_end);
                        index.range_scan(start_key, end_key, inclusive_start, inclusive_end)
                    } else {
                        vec![]
                    }
                }
                QueryPlan::CollectionScan => vec![],
            }
        };

        let uses_index_sort = match (&plan, sort_field) {
            (QueryPlan::IndexScan { ref field, .. }, Some(sf)) if field == sf => true,
            (QueryPlan::IndexRangeScan { ref field, .. }, Some(sf)) if field == sf => true,
            _ => false,
        };

        if uses_index_sort && sort_desc {
            doc_ids.reverse();
        }

        // Apply skip/limit while verifying query
        let mut results = Vec::new();
        let mut skipped = 0usize;

        for doc_id in doc_ids {
            if let Some(doc) = self.read_document_by_id(&doc_id)? {
                let doc_json_str = serde_json::to_string(&doc)?;
                let document = Document::from_json(&doc_json_str)?;

                if parsed_query.matches(&document) {
                    if skipped < skip {
                        skipped += 1;
                        continue;
                    }

                    results.push(doc_id.clone());
                    if let Some(limit_count) = limit {
                        if results.len() >= limit_count {
                            break;
                        }
                    }
                }
            }
        }

        Ok((results, uses_index_sort))
    }

    fn query_matches_all(query_json: &Value) -> bool {
        match query_json {
            Value::Null => true,
            Value::Object(map) => map.is_empty(),
            _ => false,
        }
    }

    fn extract_id_query(query_json: &Value) -> Option<DocumentId> {
        if let Value::Object(map) = query_json {
            if map.len() == 1 {
                if let Some(id_value) = map.get("_id") {
                    return serde_json::from_value(id_value.clone()).ok();
                }
            }
        }
        None
    }

    /// Scan all documents in this collection and return latest version by _id
    /// This helper reduces code duplication across find(), update(), delete(), etc.
    /// DEPRECATED: Use scan_documents_via_catalog() for better performance
    // Dead code removed - use scan_documents_via_catalog() instead
    // which is faster (O(n) catalog iteration vs O(n) file scan)

    /// Filter documents by query and exclude tombstones
    /// Returns only live documents matching the query
    fn filter_documents(
        &self,
        docs_by_id: HashMap<DocumentId, Value>,
        query: &Query,
    ) -> Result<Vec<Value>> {
        let mut results = Vec::new();

        for (_, doc) in docs_by_id {
            // Skip tombstones
            if doc
                .get("_tombstone")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                continue;
            }

            // Convert to Document and check query
            let doc_json_str = serde_json::to_string(&doc)?;
            let document = Document::from_json(&doc_json_str)?;

            if query.matches(&document) {
                results.push(doc);
            }
        }

        Ok(results)
    }
}

/// Streaming cursor over query results
pub struct FindCursor<'a, S: Storage + RawStorage> {
    collection: &'a CollectionCore<S>,
    doc_ids: Vec<DocumentId>,
    position: usize,
}

impl<'a, S: Storage + RawStorage> FindCursor<'a, S> {
    /// Fetch the next chunk of documents (up to `chunk_size`)
    pub fn next_chunk(&mut self, chunk_size: usize) -> Result<Vec<Value>> {
        if self.position >= self.doc_ids.len() {
            return Ok(Vec::new());
        }

        let end = (self.position + chunk_size).min(self.doc_ids.len());
        let mut results = Vec::with_capacity(end - self.position);
        for doc_id in &self.doc_ids[self.position..end] {
            if let Some(doc) = self.collection.read_document_by_id(doc_id)? {
                results.push(doc);
            }
        }
        self.position = end;
        Ok(results)
    }

    /// Remaining documents in the cursor
    pub fn remaining(&self) -> usize {
        self.doc_ids.len().saturating_sub(self.position)
    }

    pub fn is_finished(&self) -> bool {
        self.position >= self.doc_ids.len()
    }
}
