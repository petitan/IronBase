// Index operations for CollectionCore
// B+ tree index creation, management, and query optimization

use serde_json::Value;

use crate::error::{IronBaseError, Result};
use crate::index::{IndexKey, IndexManager, IndexMetadata, IndexStats};
use crate::query::Query;
use crate::query_planner::QueryPlanner;
use crate::storage::{RawStorage, Storage};
use crate::value_utils::{get_all_nested_values, path_crosses_array};

use super::index_persistence::persist_index_to_disk;
use super::CollectionCore;

/// Index operations for CollectionCore
impl<S: Storage + RawStorage> CollectionCore<S> {
    /// Explain query execution plan without executing
    pub fn explain(&self, query_json: &Value) -> Result<Value> {
        let indexes = self.indexes.read();
        let index_fields = indexes.list_indexes_with_compound_info();

        let plan = QueryPlanner::explain_query_with_fields(query_json, &index_fields);
        Ok(plan)
    }

    /// Find with manual index hint
    pub fn find_with_hint(&self, query_json: &Value, hint: &str) -> Result<Vec<Value>> {
        let parsed_query = Query::from_json(query_json)?;

        // Verify hint index exists
        {
            let indexes = self.indexes.read();
            if indexes.get_btree_index(hint).is_none() {
                return Err(IronBaseError::IndexError(format!(
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
        // Note: find_with_hint doesn't support ExecutionContext yet
        // Use find_with_options with hint parameter for deadline/cancellation support
        self.find_with_index(parsed_query, plan, None, None)
    }

    /// Create a compound B+ tree index on multiple fields
    ///
    /// Compound indexes allow efficient queries on multiple fields in order.
    /// The field order matters - queries can use the index if they query
    /// the first N fields in order (prefix matching).
    ///
    /// # Arguments
    /// * `fields` - Ordered list of fields (e.g., ["country", "city"])
    /// * `unique` - Whether the compound key must be unique
    /// * `sparse` - If true, documents missing any field are not indexed
    ///
    /// # Example
    /// ```rust,ignore
    /// // Create compound index on (country, city)
    /// collection.create_compound_index(
    ///     vec!["country".to_string(), "city".to_string()],
    ///     false,
    ///     false
    /// )?;
    ///
    /// // These queries can use the index:
    /// // - {"country": "US"}                    (prefix match)
    /// // - {"country": "US", "city": "NYC"}    (full match)
    ///
    /// // This query CANNOT use the index efficiently:
    /// // - {"city": "NYC"}                      (not a prefix)
    /// ```
    pub fn create_compound_index(
        &self,
        fields: Vec<String>,
        unique: bool,
        sparse: bool,
    ) -> Result<String> {
        if fields.is_empty() {
            return Err(IronBaseError::IndexError(
                "Compound index must have at least one field".to_string(),
            ));
        }

        // Create index name from all fields: users_country_city
        let index_name = format!("{}_{}", self.name, fields.join("_"));

        tracing::info!(
            collection = %self.name,
            fields = ?fields,
            index_name = %index_name,
            unique = unique,
            "Starting compound index creation"
        );

        let mut indexes = self.indexes.write();
        indexes.create_compound_index(index_name.clone(), fields.clone(), unique, sparse)?;
        drop(indexes); // Release index lock before batch scanning

        // Insert entries in batches to avoid full materialization (memory-safe).
        const INDEX_BUILD_BATCH_SIZE: usize = 1000; // Increased for better throughput
        const PROGRESS_LOG_INTERVAL: usize = 10000; // Log every 10K docs
        let mut indexed_entries: usize = 0;
        let mut total_scanned: usize = 0;
        let mut multikey_seen = false;
        let fields_clone = fields.clone();
        let collection_name = self.name.clone();
        self.scan_documents_in_batches(INDEX_BUILD_BATCH_SIZE, |_batch_num, batch_docs| {
            let mut indexes = self.indexes.write();
            let index = indexes.get_btree_index_mut(&index_name).ok_or_else(|| {
                IronBaseError::IndexError(format!("Index '{}' not found", index_name))
            })?;
            for (doc_id, doc) in batch_docs {
                if !multikey_seen
                    && fields_clone
                        .iter()
                        .any(|field| path_crosses_array(&doc, field))
                {
                    multikey_seen = true;
                }
                // Replicate extract_keys logic inline to avoid holding index lock
                let mut field_values: Vec<Vec<IndexKey>> = Vec::with_capacity(fields_clone.len());
                let mut missing_field = false;
                for field in &fields_clone {
                    let values = get_all_nested_values(&doc, field);
                    if values.is_empty() {
                        missing_field = true;
                        field_values.push(vec![IndexKey::Null]);
                    } else {
                        field_values.push(values.into_iter().map(IndexKey::from).collect());
                    }
                }
                if sparse && missing_field {
                    total_scanned += 1;
                    continue;
                }

                let mut combinations: Vec<Vec<IndexKey>> = vec![Vec::new()];
                for values in field_values {
                    let mut next = Vec::new();
                    for prefix in &combinations {
                        for value in &values {
                            let mut key = prefix.clone();
                            key.push(value.clone());
                            next.push(key);
                        }
                    }
                    combinations = next;
                }

                let mut seen = std::collections::HashSet::new();
                for keys in combinations {
                    let index_key = IndexKey::Compound(keys);
                    if !seen.insert(index_key.clone()) {
                        continue;
                    }
                    let is_null = IndexManager::is_key_all_null(&index_key);
                    let should_index = !is_null || (unique && !sparse);
                    if should_index {
                        index.insert(index_key, doc_id.clone())?;
                        indexed_entries += 1;
                    }
                }
                total_scanned += 1;
            }
            // Log progress every PROGRESS_LOG_INTERVAL documents
            if total_scanned % PROGRESS_LOG_INTERVAL == 0 {
                tracing::info!(
                    collection = %collection_name,
                    scanned = total_scanned,
                    indexed = indexed_entries,
                    "Compound index build progress: scanning documents"
                );
            }
            Ok(())
        })?;

        tracing::info!(
            collection = %self.name,
            total_scanned = total_scanned,
            indexed = indexed_entries,
            "Document scan complete, setting index metadata"
        );

        // Update metadata after streaming index build.
        let mut indexes = self.indexes.write();
        if let Some(index) = indexes.get_btree_index_mut(&index_name) {
            if multikey_seen {
                index.metadata.multikey = true;
            }
        }
        drop(indexes); // Release index lock

        tracing::info!(
            collection = %self.name,
            "B+ tree built, persisting to disk"
        );

        // PERSIST index data to .idx file FIRST (to get correct root_offset)
        let root_offset = {
            let storage = self.storage.read();
            let db_file_path = storage.get_file_path().to_string();
            drop(storage);

            if !db_file_path.is_empty() {
                let mut indexes = self.indexes.write();
                if let Some(index) = indexes.get_btree_index_mut(&index_name) {
                    persist_index_to_disk(&db_file_path, &index_name, |file| {
                        index.save_to_file(file)
                    })?;
                    index.metadata.root_offset
                } else {
                    0
                }
            } else {
                0
            }
        };

        // Mark index as ready BEFORE persisting metadata
        {
            let mut indexes = self.indexes.write();
            if let Err(e) = indexes.set_index_ready(&index_name) {
                tracing::warn!(error = %e, "Failed to mark compound index as ready");
            }
        }

        // THEN persist metadata with correct root_offset
        {
            let mut storage = self.storage.write();
            if let Some(meta) = storage.get_collection_meta_mut(&self.name) {
                let index_meta = self
                    .indexes
                    .read()
                    .get_btree_index(&index_name)
                    .map(|index| {
                        let mut meta = index.metadata.clone();
                        meta.root_offset = root_offset;
                        meta
                    })
                    .unwrap_or_else(|| IndexMetadata {
                        name: index_name.clone(),
                        field: fields[0].clone(), // Primary field for backward compat
                        fields: fields.clone(),
                        unique,
                        sparse,
                        multikey: multikey_seen,
                        case_insensitive: false,
                        num_keys: 0,
                        tree_height: 1,
                        root_offset,
                        stats: IndexStats::default(),
                        building: false,
                    });

                meta.indexes.push(index_meta);
                storage.flush()?;
            }
        }

        tracing::info!(
            collection = %self.name,
            index_name = %index_name,
            "Compound index creation complete"
        );

        Ok(index_name)
    }

    /// Create a B+ tree index on a field
    ///
    /// # Arguments
    /// * `field` - Field to index
    /// * `unique` - Whether values must be unique
    /// * `sparse` - If true, documents missing the field are not indexed
    pub fn create_index(&self, field: String, unique: bool, sparse: bool) -> Result<String> {
        self.check_not_closed()?;
        let index_name = format!("{}_{}", self.name, field);

        tracing::info!(
            collection = %self.name,
            field = %field,
            index_name = %index_name,
            unique = unique,
            sparse = sparse,
            "Starting index creation"
        );

        let mut indexes = self.indexes.write();
        indexes.create_btree_index(index_name.clone(), field.clone(), unique, sparse)?;

        // Release index lock before batch scanning
        drop(indexes);

        // Insert index entries in batches to avoid full materialization.
        const INDEX_BUILD_BATCH_SIZE: usize = 1000; // Increased for better throughput
        const PROGRESS_LOG_INTERVAL: usize = 10000; // Log every 10K docs
        let mut indexed_entries: usize = 0;
        let mut total_scanned: usize = 0;
        let mut multikey_seen = false;
        let field_clone = field.clone();
        let collection_name = self.name.clone();
        self.scan_documents_in_batches(INDEX_BUILD_BATCH_SIZE, |_batch_num, batch_docs| {
            let mut indexes = self.indexes.write();
            let index = indexes.get_btree_index_mut(&index_name).ok_or_else(|| {
                IronBaseError::IndexError(format!("Index '{}' not found", index_name))
            })?;
            for (doc_id, doc) in batch_docs {
                if !multikey_seen && path_crosses_array(&doc, &field_clone) {
                    multikey_seen = true;
                }
                let values = get_all_nested_values(&doc, &field_clone);
                if values.is_empty() {
                    if sparse {
                        total_scanned += 1;
                        continue;
                    }
                    let index_key = IndexKey::Null;
                    let should_index =
                        !IndexManager::is_key_all_null(&index_key) || (unique && !sparse);
                    if should_index {
                        index.insert(index_key, doc_id.clone())?;
                        indexed_entries += 1;
                    }
                } else {
                    let mut seen = std::collections::HashSet::new();
                    for value in values {
                        let index_key = IndexKey::from(value);
                        if !seen.insert(index_key.clone()) {
                            continue;
                        }
                        let should_index =
                            !IndexManager::is_key_all_null(&index_key) || (unique && !sparse);
                        if should_index {
                            index.insert(index_key, doc_id.clone())?;
                            indexed_entries += 1;
                        }
                    }
                }
                total_scanned += 1;
            }
            // Log progress every PROGRESS_LOG_INTERVAL documents
            if total_scanned % PROGRESS_LOG_INTERVAL == 0 {
                tracing::info!(
                    collection = %collection_name,
                    scanned = total_scanned,
                    indexed = indexed_entries,
                    "Index build progress: scanning documents"
                );
            }
            Ok(())
        })?;

        tracing::info!(
            collection = %self.name,
            total_scanned = total_scanned,
            indexed = indexed_entries,
            "Document scan complete, setting index metadata"
        );

        // Update metadata after streaming index build.
        let mut indexes = self.indexes.write();
        if let Some(index) = indexes.get_btree_index_mut(&index_name) {
            if multikey_seen {
                index.metadata.multikey = true;
            }
        }
        drop(indexes); // Release index lock

        tracing::info!(
            collection = %self.name,
            "B+ tree built, persisting to disk"
        );

        // PERSIST index data to .idx file FIRST (to get correct root_offset)
        let root_offset = {
            let storage = self.storage.read();
            let db_file_path = storage.get_file_path().to_string();
            drop(storage);

            if !db_file_path.is_empty() {
                let mut indexes = self.indexes.write();
                if let Some(index) = indexes.get_btree_index_mut(&index_name) {
                    persist_index_to_disk(&db_file_path, &index_name, |file| {
                        index.save_to_file(file)
                    })?;
                    index.metadata.root_offset
                } else {
                    0
                }
            } else {
                0
            }
        };

        // Mark index as ready BEFORE persisting metadata
        // This ensures the saved metadata has building=false
        {
            let mut indexes = self.indexes.write();
            if let Err(e) = indexes.set_index_ready(&index_name) {
                tracing::warn!(error = %e, "Failed to mark index as ready");
            }
        }

        // THEN persist metadata with correct root_offset
        {
            let mut storage = self.storage.write();
            if let Some(meta) = storage.get_collection_meta_mut(&self.name) {
                let index_meta = self
                    .indexes
                    .read()
                    .get_btree_index(&index_name)
                    .map(|index| {
                        let mut meta = index.metadata.clone();
                        meta.root_offset = root_offset;
                        meta
                    })
                    .unwrap_or_else(|| IndexMetadata {
                        name: index_name.clone(),
                        field: field.clone(),
                        fields: vec![field.clone()], // Single-field index
                        unique,
                        sparse,
                        multikey: multikey_seen,
                        case_insensitive: false,
                        num_keys: 0,
                        tree_height: 1,
                        root_offset,
                        stats: IndexStats::default(),
                        building: false, // Explicitly set to false
                    });

                meta.indexes.push(index_meta);
                storage.flush()?;
            }
        }

        tracing::info!(
            collection = %self.name,
            index_name = %index_name,
            "Index creation complete"
        );

        Ok(index_name)
    }

    /// Create a case-insensitive B+ tree index on a field
    ///
    /// String values are lowercased before indexing, allowing case-insensitive
    /// queries using `$regex` with `(?i)` prefix.
    ///
    /// # Arguments
    /// * `field` - Field to index
    /// * `unique` - Whether lowercased values must be unique
    ///
    /// # Example
    /// ```rust,ignore
    /// collection.create_ci_index("email".to_string(), true)?;
    /// // Now queries like {"email": {"$regex": "(?i)^john"}} will use this index
    /// ```
    pub fn create_ci_index(&self, field: String, unique: bool) -> Result<String> {
        self.check_not_closed()?;
        let index_name = format!("{}_{}_ci", self.name, field);

        tracing::info!(
            collection = %self.name,
            field = %field,
            index_name = %index_name,
            unique = unique,
            "Starting case-insensitive index creation"
        );

        let mut indexes = self.indexes.write();
        indexes.create_btree_index_ci(index_name.clone(), field.clone(), unique)?;

        // Release index lock before batch scanning
        drop(indexes);

        // Insert index entries in batches - lowercase strings for CI.
        const INDEX_BUILD_BATCH_SIZE: usize = 1000;
        const PROGRESS_LOG_INTERVAL: usize = 10000;
        let mut indexed_entries: usize = 0;
        let mut total_scanned: usize = 0;
        let mut multikey_seen = false;
        let field_clone = field.clone();
        let collection_name = self.name.clone();

        self.scan_documents_in_batches(INDEX_BUILD_BATCH_SIZE, |_batch_num, batch_docs| {
            let mut indexes = self.indexes.write();
            let index = indexes.get_btree_index_mut(&index_name).ok_or_else(|| {
                IronBaseError::IndexError(format!("Index '{}' not found", index_name))
            })?;
            for (doc_id, doc) in batch_docs {
                if !multikey_seen && path_crosses_array(&doc, &field_clone) {
                    multikey_seen = true;
                }
                let values = get_all_nested_values(&doc, &field_clone);
                if values.is_empty() {
                    // CI index is always sparse - skip missing values
                    total_scanned += 1;
                    continue;
                } else {
                    let mut seen = std::collections::HashSet::new();
                    for value in values {
                        // Lowercase string values for case-insensitive matching
                        let index_key = match value {
                            Value::String(s) => IndexKey::String(s.to_lowercase()),
                            other => IndexKey::from(other),
                        };
                        if !seen.insert(index_key.clone()) {
                            continue;
                        }
                        // Skip nulls - CI indexes are implicitly sparse
                        if !IndexManager::is_key_all_null(&index_key) {
                            index.insert(index_key, doc_id.clone())?;
                            indexed_entries += 1;
                        }
                    }
                }
                total_scanned += 1;
            }
            if total_scanned % PROGRESS_LOG_INTERVAL == 0 {
                tracing::info!(
                    collection = %collection_name,
                    scanned = total_scanned,
                    indexed = indexed_entries,
                    "CI index build progress: scanning documents"
                );
            }
            Ok(())
        })?;

        tracing::info!(
            collection = %self.name,
            total_scanned = total_scanned,
            indexed = indexed_entries,
            "Document scan complete, setting index metadata"
        );

        // Update metadata after streaming index build.
        let mut indexes = self.indexes.write();
        if let Some(index) = indexes.get_btree_index_mut(&index_name) {
            if multikey_seen {
                index.metadata.multikey = true;
            }
        }
        drop(indexes);

        tracing::info!(
            collection = %self.name,
            "B+ tree built, persisting to disk"
        );

        // PERSIST index data to .idx file
        let root_offset = {
            let storage = self.storage.read();
            let db_file_path = storage.get_file_path().to_string();
            drop(storage);

            if !db_file_path.is_empty() {
                let mut indexes = self.indexes.write();
                if let Some(index) = indexes.get_btree_index_mut(&index_name) {
                    persist_index_to_disk(&db_file_path, &index_name, |file| {
                        index.save_to_file(file)
                    })?;
                    index.metadata.root_offset
                } else {
                    0
                }
            } else {
                0
            }
        };

        // Persist metadata
        {
            let mut storage = self.storage.write();
            if let Some(meta) = storage.get_collection_meta_mut(&self.name) {
                let index_meta = self
                    .indexes
                    .read()
                    .get_btree_index(&index_name)
                    .map(|index| {
                        let mut meta = index.metadata.clone();
                        meta.root_offset = root_offset;
                        meta
                    })
                    .unwrap_or_else(|| IndexMetadata {
                        name: index_name.clone(),
                        field: field.clone(),
                        fields: vec![field.clone()],
                        unique,
                        sparse: true, // CI indexes are implicitly sparse
                        multikey: multikey_seen,
                        case_insensitive: true,
                        num_keys: 0,
                        tree_height: 1,
                        root_offset,
                        stats: IndexStats::default(),
                        building: false, // CI index is ready after this point
                    });

                meta.indexes.push(index_meta);
                storage.flush()?;
            }
        }

        tracing::info!(
            collection = %self.name,
            index_name = %index_name,
            "Case-insensitive index creation complete"
        );

        Ok(index_name)
    }

    /// Drop an index
    pub fn drop_index(&self, index_name: &str) -> Result<()> {
        self.check_not_closed()?;
        let mut indexes = self.indexes.write();
        indexes.drop_index(index_name)?;

        // FIX: Hold index lock while updating metadata to prevent race condition
        // where another thread sees inconsistent state (index gone but metadata present)
        let mut storage = self.storage.write();
        if let Some(meta) = storage.get_collection_meta_mut(&self.name) {
            // Remove from B+ tree indexes
            meta.indexes.retain(|idx| idx.name != index_name);
            // Remove from fuzzy indexes
            meta.fuzzy_indexes.retain(|idx| idx.name != index_name);
            // Remove from fulltext indexes
            meta.fulltext_indexes.retain(|idx| idx.name != index_name);
            storage.flush()?;
        }
        // Both locks released here atomically

        Ok(())
    }

    /// List all indexes
    pub fn list_indexes(&self) -> Result<Vec<String>> {
        self.check_not_closed()?;
        let indexes = self.indexes.read();
        Ok(indexes.list_indexes())
    }

    /// List all indexes with their prefix field (for QueryPlanner)
    ///
    /// Returns tuples of (index_name, first_field) - compound indexes only
    /// return their FIRST field, as they can only be used for prefix queries.
    pub fn list_indexes_with_prefix_field(&self) -> Result<Vec<(String, String)>> {
        self.check_not_closed()?;
        let indexes = self.indexes.read();
        Ok(indexes.list_indexes_with_prefix_field())
    }
}
