// ironbase-core/src/collection_core.rs
// Pure Rust collection logic - NO PyO3 dependencies

use std::sync::Arc;
use parking_lot::RwLock;
use serde_json::Value;
use std::collections::HashMap;

use crate::storage::StorageEngine;
use crate::document::{Document, DocumentId};
use crate::error::{Result, MongoLiteError};
use crate::query::Query;
use crate::index::{IndexManager, IndexKey};
use crate::query_planner::{QueryPlanner, QueryPlan};

/// Pure Rust Collection - language-independent core logic
pub struct CollectionCore {
    pub name: String,
    pub storage: Arc<RwLock<StorageEngine>>,
    /// Index manager for B+ tree indexes
    pub indexes: Arc<RwLock<IndexManager>>,
}

impl CollectionCore {
    /// Create new collection (or get existing)
    pub fn new(name: String, storage: Arc<RwLock<StorageEngine>>) -> Result<Self> {
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
        index_manager.create_btree_index(
            format!("{}_id", name),
            "_id".to_string(),
            true  // unique
        )?;

        Ok(CollectionCore {
            name,
            storage,
            indexes: Arc::new(RwLock::new(index_manager)),
        })
    }

    /// Insert one document - returns inserted DocumentId
    pub fn insert_one(&self, mut fields: HashMap<String, Value>) -> Result<DocumentId> {
        let mut storage = self.storage.write();

        // Get mutable reference to collection metadata
        let meta = storage.get_collection_meta_mut(&self.name)
            .ok_or_else(|| MongoLiteError::CollectionNotFound(self.name.clone()))?;

        // ID generálás
        let doc_id = DocumentId::new_auto(meta.last_id);
        meta.last_id += 1;

        // Add _collection field for multi-collection isolation
        fields.insert("_collection".to_string(), Value::String(self.name.clone()));

        // Dokumentum létrehozása
        let doc = Document::new(doc_id.clone(), fields);

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

        // Szerializálás és írás
        let doc_json = doc.to_json()?;
        storage.write_data(doc_json.as_bytes())?;

        Ok(doc_id)
    }

    /// Find documents matching query
    pub fn find(&self, query_json: &Value) -> Result<Vec<Value>> {
        let parsed_query = Query::from_json(query_json)?;

        // Try to use an index
        let indexes = self.indexes.read();
        let available_indexes = indexes.list_indexes();

        if let Some((_field, plan)) = QueryPlanner::analyze_query(query_json, &available_indexes) {
            // Use index-based execution
            return self.find_with_index(parsed_query, plan);
        }

        // Fall back to full collection scan
        drop(indexes); // Release read lock before write lock

        // Scan all documents and filter by query (helper handles locks internally)
        let docs_by_id = self.scan_documents()?;
        let matching_docs = self.filter_documents(docs_by_id, &parsed_query)?;

        Ok(matching_docs)
    }

    /// Find documents with options (projection, sort, limit, skip)
    pub fn find_with_options(
        &self,
        query_json: &Value,
        options: crate::find_options::FindOptions
    ) -> Result<Vec<Value>> {
        use crate::find_options::{apply_projection, apply_sort, apply_limit_skip};

        // 1. Get matching documents (use existing find() logic)
        let mut docs = self.find(query_json)?;

        // 2. Apply sort
        if let Some(ref sort) = options.sort {
            apply_sort(&mut docs, sort);
        }

        // 3. Apply skip and limit
        docs = apply_limit_skip(docs, options.limit, options.skip);

        // 4. Apply projection
        if let Some(ref projection) = options.projection {
            docs = docs.into_iter()
                .map(|doc| apply_projection(&doc, projection))
                .collect();
        }

        Ok(docs)
    }

    /// Find one document matching query
    pub fn find_one(&self, query_json: &Value) -> Result<Option<Value>> {
        let parsed_query = Query::from_json(query_json)?;

        let mut storage = self.storage.write();
        let meta = storage.get_collection_meta(&self.name)
            .ok_or_else(|| MongoLiteError::CollectionNotFound(self.name.clone()))?;

        let file_len = storage.file_len()?;

        // Use HashMap to track latest version of each document by _id
        let mut docs_by_id: HashMap<String, Value> = HashMap::new();
        let mut current_offset = meta.data_offset;

        while current_offset < file_len {
            match storage.read_data(current_offset) {
                Ok(doc_bytes) => {
                    let doc: Value = serde_json::from_slice(&doc_bytes)?;

                    // ✅ FILTER: Only include documents from THIS collection
                    let doc_collection = doc.get("_collection")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");

                    if doc_collection == self.name {
                        if let Some(id_value) = doc.get("_id") {
                            let id_key = serde_json::to_string(id_value)
                                .unwrap_or_else(|_| "unknown".to_string());
                            docs_by_id.insert(id_key, doc);
                        }
                    }

                    current_offset += 4 + doc_bytes.len() as u64;
                }
                Err(_) => break,
            }
        }

        // Find first matching document (skip tombstones)
        for (_, doc) in docs_by_id {
            // Skip tombstones (deleted documents)
            if doc.get("_tombstone").and_then(|v| v.as_bool()).unwrap_or(false) {
                continue;
            }

            let doc_json_str = serde_json::to_string(&doc)?;
            let document = Document::from_json(&doc_json_str)?;

            if parsed_query.matches(&document) {
                return Ok(Some(doc));
            }
        }

        Ok(None)
    }

    /// Count documents matching query
    pub fn count_documents(&self, query_json: &Value) -> Result<u64> {
        let parsed_query = Query::from_json(query_json)?;

        let mut storage = self.storage.write();
        let meta = storage.get_collection_meta(&self.name)
            .ok_or_else(|| MongoLiteError::CollectionNotFound(self.name.clone()))?;

        let file_len = storage.file_len()?;

        // Use HashMap to track latest version of each document by _id
        let mut docs_by_id: HashMap<String, Value> = HashMap::new();
        let mut current_offset = meta.data_offset;

        while current_offset < file_len {
            match storage.read_data(current_offset) {
                Ok(doc_bytes) => {
                    let doc: Value = serde_json::from_slice(&doc_bytes)?;

                    // ✅ FILTER: Only include documents from THIS collection
                    let doc_collection = doc.get("_collection")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");

                    if doc_collection == self.name {
                        if let Some(id_value) = doc.get("_id") {
                            let id_key = serde_json::to_string(id_value)
                                .unwrap_or_else(|_| "unknown".to_string());
                            docs_by_id.insert(id_key, doc);
                        }
                    }

                    current_offset += 4 + doc_bytes.len() as u64;
                }
                Err(_) => {
                    break;
                }
            }
        }

        // Count matching documents (skip tombstones)
        let mut count = 0u64;
        for (_, doc) in docs_by_id {
            // Skip tombstones (deleted documents)
            if doc.get("_tombstone").and_then(|v| v.as_bool()).unwrap_or(false) {
                continue;
            }

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

        let mut storage = self.storage.write();
        let meta = storage.get_collection_meta(&self.name)
            .ok_or_else(|| MongoLiteError::CollectionNotFound(self.name.clone()))?;

        let file_len = storage.file_len()?;

        // First pass: collect all documents by _id (latest version only)
        let mut docs_by_id: HashMap<String, Value> = HashMap::new();
        let mut current_offset = meta.data_offset;

        while current_offset < file_len {
            match storage.read_data(current_offset) {
                Ok(doc_bytes) => {
                    let doc: Value = serde_json::from_slice(&doc_bytes)?;

                    // Track latest version (include tombstones so they overwrite originals)
                    if let Some(id_value) = doc.get("_id") {
                        let id_key = serde_json::to_string(id_value)
                            .unwrap_or_else(|_| "unknown".to_string());
                        docs_by_id.insert(id_key, doc);
                    }

                    current_offset += 4 + doc_bytes.len() as u64;
                }
                Err(_) => break,
            }
        }

        // Second pass: find first matching and update (skip tombstones)
        let mut matched = 0u64;
        let mut modified = 0u64;

        for (_, doc) in docs_by_id {
            // Skip tombstones (deleted documents)
            if doc.get("_tombstone").and_then(|v| v.as_bool()).unwrap_or(false) {
                continue;
            }
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

                    // Write tombstone
                    storage.write_data(tombstone_json.as_bytes())?;

                    // ✅ Ensure updated document has _collection
                    document.set("_collection".to_string(), Value::String(self.name.clone()));

                    // Write updated document
                    let updated_json = document.to_json()?;
                    storage.write_data(updated_json.as_bytes())?;

                    modified = 1;
                }
            }
        }

        Ok((matched, modified))
    }

    /// Update many documents - returns (matched_count, modified_count)
    pub fn update_many(&self, query_json: &Value, update_json: &Value) -> Result<(u64, u64)> {
        let parsed_query = Query::from_json(query_json)?;

        let mut storage = self.storage.write();
        let meta = storage.get_collection_meta(&self.name)
            .ok_or_else(|| MongoLiteError::CollectionNotFound(self.name.clone()))?;

        let file_len = storage.file_len()?;

        // First pass: collect all documents by _id (latest version only)
        let mut docs_by_id: HashMap<String, Value> = HashMap::new();
        let mut current_offset = meta.data_offset;

        while current_offset < file_len {
            match storage.read_data(current_offset) {
                Ok(doc_bytes) => {
                    let doc: Value = serde_json::from_slice(&doc_bytes)?;

                    // Track latest version (include tombstones so they overwrite originals)
                    if let Some(id_value) = doc.get("_id") {
                        let id_key = serde_json::to_string(id_value)
                            .unwrap_or_else(|_| "unknown".to_string());
                        docs_by_id.insert(id_key, doc);
                    }

                    current_offset += 4 + doc_bytes.len() as u64;
                }
                Err(_) => break,
            }
        }

        // Second pass: find all matching and update (skip tombstones)
        let mut matched = 0u64;
        let mut modified = 0u64;

        for (_, doc) in docs_by_id {
            // Skip tombstones (deleted documents)
            if doc.get("_tombstone").and_then(|v| v.as_bool()).unwrap_or(false) {
                continue;
            }

            let doc_json_str = serde_json::to_string(&doc)?;
            let mut document = Document::from_json(&doc_json_str)?;

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

                    // Write tombstone
                    storage.write_data(tombstone_json.as_bytes())?;

                    // ✅ Ensure updated document has _collection
                    document.set("_collection".to_string(), Value::String(self.name.clone()));

                    // Write updated document
                    let updated_json = document.to_json()?;
                    storage.write_data(updated_json.as_bytes())?;

                    modified += 1;
                }
            }
        }

        Ok((matched, modified))
    }

    /// Delete one document - returns deleted_count
    pub fn delete_one(&self, query_json: &Value) -> Result<u64> {
        let parsed_query = Query::from_json(query_json)?;

        let mut storage = self.storage.write();
        let meta = storage.get_collection_meta(&self.name)
            .ok_or_else(|| MongoLiteError::CollectionNotFound(self.name.clone()))?;

        let file_len = storage.file_len()?;

        // First pass: collect all documents by _id (latest version only)
        let mut docs_by_id: HashMap<String, Value> = HashMap::new();
        let mut current_offset = meta.data_offset;

        while current_offset < file_len {
            match storage.read_data(current_offset) {
                Ok(doc_bytes) => {
                    let doc: Value = serde_json::from_slice(&doc_bytes)?;

                    // Track latest version (include tombstones so they overwrite originals)
                    if let Some(id_value) = doc.get("_id") {
                        let id_key = serde_json::to_string(id_value)
                            .unwrap_or_else(|_| "unknown".to_string());
                        docs_by_id.insert(id_key, doc);
                    }

                    current_offset += 4 + doc_bytes.len() as u64;
                }
                Err(_) => break,
            }
        }

        // Second pass: find first matching and delete (skip tombstones)
        let mut deleted = 0u64;

        for (_, doc) in docs_by_id {
            // Skip tombstones (already deleted documents)
            if doc.get("_tombstone").and_then(|v| v.as_bool()).unwrap_or(false) {
                continue;
            }
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

                // Write tombstone
                storage.write_data(tombstone_json.as_bytes())?;

                deleted = 1;
            }
        }

        Ok(deleted)
    }

    /// Delete many documents - returns deleted_count
    pub fn delete_many(&self, query_json: &Value) -> Result<u64> {
        let parsed_query = Query::from_json(query_json)?;

        let mut storage = self.storage.write();
        let meta = storage.get_collection_meta(&self.name)
            .ok_or_else(|| MongoLiteError::CollectionNotFound(self.name.clone()))?;

        let file_len = storage.file_len()?;

        // First pass: collect all documents by _id (latest version only)
        let mut docs_by_id: HashMap<String, Value> = HashMap::new();
        let mut current_offset = meta.data_offset;

        while current_offset < file_len {
            match storage.read_data(current_offset) {
                Ok(doc_bytes) => {
                    let doc: Value = serde_json::from_slice(&doc_bytes)?;

                    // Track latest version (include tombstones so they overwrite originals)
                    if let Some(id_value) = doc.get("_id") {
                        let id_key = serde_json::to_string(id_value)
                            .unwrap_or_else(|_| "unknown".to_string());
                        docs_by_id.insert(id_key, doc);
                    }

                    current_offset += 4 + doc_bytes.len() as u64;
                }
                Err(_) => break,
            }
        }

        // Second pass: find all matching and delete (skip tombstones)
        let mut deleted = 0u64;

        for (_, doc) in docs_by_id {
            // Skip tombstones (already deleted documents)
            if doc.get("_tombstone").and_then(|v| v.as_bool()).unwrap_or(false) {
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

                // Write tombstone
                storage.write_data(tombstone_json.as_bytes())?;

                deleted += 1;
            }
        }

        Ok(deleted)
    }

    /// Distinct values for a field
    pub fn distinct(&self, field: &str, query_json: &Value) -> Result<Vec<Value>> {
        let parsed_query = Query::from_json(query_json)?;

        let mut storage = self.storage.write();
        let meta = storage.get_collection_meta(&self.name)
            .ok_or_else(|| MongoLiteError::CollectionNotFound(self.name.clone()))?;

        let file_len = storage.file_len()?;

        // Use HashMap to track latest version of each document by _id
        let mut docs_by_id: HashMap<String, Value> = HashMap::new();
        let mut current_offset = meta.data_offset;

        while current_offset < file_len {
            match storage.read_data(current_offset) {
                Ok(doc_bytes) => {
                    let doc: Value = serde_json::from_slice(&doc_bytes)?;

                    // ✅ FILTER: Only include documents from THIS collection
                    let doc_collection = doc.get("_collection")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");

                    if doc_collection == self.name {
                        if let Some(id_value) = doc.get("_id") {
                            let id_key = serde_json::to_string(id_value)
                                .unwrap_or_else(|_| "unknown".to_string());
                            docs_by_id.insert(id_key, doc);
                        }
                    }

                    current_offset += 4 + doc_bytes.len() as u64;
                }
                Err(_) => break,
            }
        }

        // Collect distinct values from matching documents (skip tombstones)
        let mut seen_values: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut distinct_values = Vec::new();

        for (_, doc) in docs_by_id {
            // Skip tombstones (deleted documents)
            if doc.get("_tombstone").and_then(|v| v.as_bool()).unwrap_or(false) {
                continue;
            }

            let doc_json_str = serde_json::to_string(&doc)?;
            let document = Document::from_json(&doc_json_str)?;

            // Check if matches query
            if parsed_query.matches(&document) {
                // Extract field value
                if let Some(field_value) = doc.get(field) {
                    // Use JSON string representation for uniqueness check
                    let value_key = serde_json::to_string(field_value)
                        .unwrap_or_else(|_| "null".to_string());

                    // Only add if not seen before
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
        index_name.strip_prefix(&prefix)
            .unwrap_or(index_name)
            .to_string()
    }

    /// Create a query plan for a hinted index
    fn create_plan_for_hint(&self, query_json: &Value, index_name: &str, field: &str) -> Result<QueryPlan> {
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

        Err(MongoLiteError::IndexError(
            format!("Cannot use index '{}' for this query", index_name)
        ))
    }

    /// Execute query using an index
    fn find_with_index(&self, parsed_query: Query, plan: QueryPlan) -> Result<Vec<Value>> {
        // Get candidate document IDs from index
        let doc_ids: Vec<DocumentId> = {
            let indexes = self.indexes.read();

            match plan {
                QueryPlan::IndexScan { index_name, key, .. } => {
                    if let Some(index) = indexes.get_btree_index(&index_name) {
                        // Use range scan with same start and end to get ALL matching documents
                        // (B+ tree may have multiple documents with same key value)
                        index.range_scan(&key, &key, true, true)
                    } else {
                        vec![]
                    }
                }
                QueryPlan::IndexRangeScan {
                    index_name,
                    start,
                    end,
                    inclusive_start,
                    inclusive_end,
                    ..
                } => {
                    if let Some(index) = indexes.get_btree_index(&index_name) {
                        // Range scan
                        let default_start = IndexKey::Null;
                        let default_end = IndexKey::String("\u{10ffff}".repeat(100));

                        let start_key = start.as_ref().unwrap_or(&default_start);
                        let end_key = end.as_ref().unwrap_or(&default_end);

                        index.range_scan(start_key, end_key, inclusive_start, inclusive_end)
                    } else {
                        vec![]
                    }
                }
                QueryPlan::CollectionScan => {
                    // This shouldn't happen, but fall back to empty
                    vec![]
                }
            }
        }; // indexes read lock dropped here

        let mut storage = self.storage.write();

        // Now fetch documents by ID and apply full query filter
        let meta = storage.get_collection_meta(&self.name)
            .ok_or_else(|| MongoLiteError::CollectionNotFound(self.name.clone()))?;

        let file_len = storage.file_len()?;
        let mut docs_by_id: HashMap<String, Value> = HashMap::new();
        let mut current_offset = meta.data_offset;

        // Build docs_by_id map (we still need to get latest version)
        while current_offset < file_len {
            match storage.read_data(current_offset) {
                Ok(doc_bytes) => {
                    let doc: Value = serde_json::from_slice(&doc_bytes)?;

                    // Filter by collection
                    let doc_collection = doc.get("_collection")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");

                    if doc_collection == self.name {
                        if let Some(id_value) = doc.get("_id") {
                            let id_key = serde_json::to_string(id_value)
                                .unwrap_or_else(|_| "unknown".to_string());
                            docs_by_id.insert(id_key, doc);
                        }
                    }

                    current_offset += 4 + doc_bytes.len() as u64;
                }
                Err(_) => break,
            }
        }

        // Filter to only index-matched documents
        let mut matching_docs = Vec::new();
        for doc_id in doc_ids {
            let id_key = serde_json::to_string(&serde_json::json!(doc_id))
                .unwrap_or_else(|_| "unknown".to_string());

            if let Some(doc) = docs_by_id.get(&id_key) {
                // Skip tombstones
                if doc.get("_tombstone").and_then(|v| v.as_bool()).unwrap_or(false) {
                    continue;
                }

                // Apply full query filter (in case index gave us false positives)
                let doc_json_str = serde_json::to_string(doc)?;
                let document = Document::from_json(&doc_json_str)?;

                if parsed_query.matches(&document) {
                    matching_docs.push(doc.clone());
                }
            }
        }

        Ok(matching_docs)
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
                                    if let (Some(curr_int), Some(inc_int)) = (current.as_i64(), inc_value.as_i64()) {
                                        document.set(field.clone(), Value::from(curr_int + inc_int));
                                        was_modified = true;
                                    } else if let (Some(curr_num), Some(inc_num)) = (current.as_f64(), inc_value.as_f64()) {
                                        document.set(field.clone(), Value::from(curr_num + inc_num));
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
                    _ => {
                        return Err(MongoLiteError::InvalidQuery(format!("Unsupported update operator: {}", op)));
                    }
                }
            }
        }

        Ok(was_modified)
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
                return Err(MongoLiteError::IndexError(
                    format!("Index '{}' not found (hint)", hint)
                ));
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

        // Get all documents (TODO: optimize with index if $match is first stage)
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

        // TODO: Rebuild index from existing documents
        // For now, the index will be populated as new documents are inserted

        Ok(index_name)
    }

    /// Drop an index
    pub fn drop_index(&self, index_name: &str) -> Result<()> {
        let mut indexes = self.indexes.write();
        indexes.drop_index(index_name)
    }

    /// List all indexes
    pub fn list_indexes(&self) -> Vec<String> {
        let indexes = self.indexes.read();
        indexes.list_indexes()
    }

    // ========== TRANSACTION METHODS ==========

    /// Insert one document within a transaction
    ///
    /// Note: Index changes are tracked but not yet applied atomically.
    /// See INDEX_CONSISTENCY.md for future two-phase commit implementation.
    pub fn insert_one_tx(&self, doc: HashMap<String, Value>, tx: &mut crate::transaction::Transaction) -> Result<DocumentId> {
        use crate::transaction::Operation;

        // Generate document ID
        let mut storage = self.storage.write();
        let meta = storage.get_collection_meta_mut(&self.name)
            .ok_or_else(|| MongoLiteError::CollectionNotFound(self.name.clone()))?;

        let doc_id = DocumentId::new_auto(meta.last_id);
        meta.last_id += 1;
        drop(storage); // Release lock early

        // Create document with _id and _collection
        let mut doc_with_id = doc.clone();
        doc_with_id.insert("_id".to_string(), serde_json::json!(doc_id.clone()));
        doc_with_id.insert("_collection".to_string(), Value::String(self.name.clone()));

        // Add operation to transaction
        tx.add_operation(Operation::Insert {
            collection: self.name.clone(),
            doc_id: doc_id.clone(),
            doc: serde_json::json!(doc_with_id),
        })?;

        // TODO: Track index changes (future: two-phase commit)

        Ok(doc_id)
    }

    /// Update one document within a transaction
    ///
    /// Note: Pass the new_doc directly (not update operators).
    /// Index changes are tracked but not yet applied atomically.
    /// See INDEX_CONSISTENCY.md for future two-phase commit implementation.
    pub fn update_one_tx(&self, query: &Value, new_doc: Value, tx: &mut crate::transaction::Transaction) -> Result<(u64, u64)> {
        use crate::transaction::Operation;

        // Find the document first
        let doc = self.find_one(query)?;

        if let Some(old_doc) = doc {
            // Extract document ID from _id field
            let id_value = old_doc.get("_id")
                .ok_or_else(|| MongoLiteError::DocumentNotFound)?;

            let doc_id = match id_value {
                Value::Number(n) if n.is_i64() => DocumentId::Int(n.as_i64().unwrap()),
                Value::Number(n) if n.is_u64() => DocumentId::Int(n.as_u64().unwrap() as i64),
                Value::String(s) => DocumentId::String(s.clone()),
                _ => return Err(MongoLiteError::Serialization("Invalid _id type".to_string())),
            };

            // Ensure new_doc has _id and _collection fields
            let new_doc_with_meta = if let Value::Object(mut map) = new_doc {
                map.insert("_id".to_string(), id_value.clone());
                map.insert("_collection".to_string(), Value::String(self.name.clone()));
                Value::Object(map)
            } else {
                return Err(MongoLiteError::Serialization("new_doc must be an object".to_string()));
            };

            // Add operation to transaction
            tx.add_operation(Operation::Update {
                collection: self.name.clone(),
                doc_id: doc_id.clone(),
                old_doc: old_doc.clone(),
                new_doc: new_doc_with_meta,
            })?;

            // TODO: Track index changes (future: two-phase commit)

            Ok((1, 1)) // matched_count, modified_count
        } else {
            Ok((0, 0))
        }
    }

    /// Delete one document within a transaction
    ///
    /// Note: Index changes are tracked but not yet applied atomically.
    /// See INDEX_CONSISTENCY.md for future two-phase commit implementation.
    pub fn delete_one_tx(&self, query: &Value, tx: &mut crate::transaction::Transaction) -> Result<u64> {
        use crate::transaction::Operation;

        // Find the document first
        let doc = self.find_one(query)?;

        if let Some(old_doc) = doc {
            // Extract document ID from _id field
            let id_value = old_doc.get("_id")
                .ok_or_else(|| MongoLiteError::DocumentNotFound)?;

            let doc_id = match id_value {
                Value::Number(n) if n.is_i64() => DocumentId::Int(n.as_i64().unwrap()),
                Value::Number(n) if n.is_u64() => DocumentId::Int(n.as_u64().unwrap() as i64),
                Value::String(s) => DocumentId::String(s.clone()),
                _ => return Err(MongoLiteError::Serialization("Invalid _id type".to_string())),
            };

            // Add operation to transaction
            tx.add_operation(Operation::Delete {
                collection: self.name.clone(),
                doc_id: doc_id.clone(),
                old_doc: old_doc.clone(),
            })?;

            // TODO: Track index changes (future: two-phase commit)

            Ok(1) // deleted_count
        } else {
            Ok(0)
        }
    }

    // ========== PRIVATE HELPER METHODS ==========

    /// Scan all documents in this collection and return latest version by _id
    /// This helper reduces code duplication across find(), update(), delete(), etc.
    fn scan_documents(&self) -> Result<HashMap<String, Value>> {
        let mut storage = self.storage.write();
        let meta = storage.get_collection_meta(&self.name)
            .ok_or_else(|| MongoLiteError::CollectionNotFound(self.name.clone()))?;

        let file_len = storage.file_len()?;
        let mut docs_by_id: HashMap<String, Value> = HashMap::new();
        let mut current_offset = meta.data_offset;

        while current_offset < file_len {
            match storage.read_data(current_offset) {
                Ok(doc_bytes) => {
                    let doc: Value = serde_json::from_slice(&doc_bytes)?;

                    // Filter by collection
                    let doc_collection = doc.get("_collection")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");

                    if doc_collection == self.name {
                        if let Some(id_value) = doc.get("_id") {
                            let id_key = serde_json::to_string(id_value)
                                .unwrap_or_else(|_| "unknown".to_string());
                            docs_by_id.insert(id_key, doc);
                        }
                    }
                    current_offset += 4 + doc_bytes.len() as u64;
                }
                Err(_) => break,
            }
        }

        Ok(docs_by_id)
    }

    /// Filter documents by query and exclude tombstones
    /// Returns only live documents matching the query
    fn filter_documents(&self, docs_by_id: HashMap<String, Value>, query: &Query) -> Result<Vec<Value>> {
        let mut results = Vec::new();

        for (_, doc) in docs_by_id {
            // Skip tombstones
            if doc.get("_tombstone").and_then(|v| v.as_bool()).unwrap_or(false) {
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
