//! IronBase Adapter - Direct wrapper around IronBase core

use crate::error::Result;
use ironbase_core::{storage::StorageEngine, DatabaseCore};
use parking_lot::RwLock;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

/// Find options for queries
#[derive(Debug, Default)]
pub struct FindOptions {
    pub projection: Option<Value>,
    /// Sort specification - already parsed, None means no sort (O(1) skip/limit)
    pub sort: Option<Vec<(String, i32)>>,
    pub limit: Option<usize>,
    pub skip: Option<usize>,
    /// If true, also return the total count of matching documents (before limit/skip)
    /// Useful for pagination UI ("Showing 1-10 of 100 results")
    pub include_total: bool,
}

/// Find result with optional total count
#[derive(Debug)]
pub struct FindResult {
    pub documents: Vec<Value>,
    /// Total count of matching documents (only populated if include_total was true)
    pub total: Option<usize>,
}

/// Update result
#[derive(Debug)]
pub struct UpdateResult {
    pub matched_count: u64,
    pub modified_count: u64,
}

/// IronBase Adapter
pub struct IronBaseAdapter {
    db: Arc<RwLock<DatabaseCore<StorageEngine>>>,
    /// Database file path (stored for stats, wrapped in RwLock for dynamic switching)
    db_path: RwLock<std::path::PathBuf>,
}

/// Scripts collection name
pub const SCRIPTS_COLLECTION: &str = "_system.scripts";

/// Script versions collection name (for version history)
pub const SCRIPT_VERSIONS_COLLECTION: &str = "_system.script_versions";

impl IronBaseAdapter {
    /// Create a new adapter with the given database path
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self> {
        let db_path = path.as_ref().to_path_buf();
        let db = DatabaseCore::open(&db_path)?;
        let adapter = Self {
            db: Arc::new(RwLock::new(db)),
            db_path: RwLock::new(db_path),
        };
        // Ensure system collections exist
        adapter.ensure_system_collections()?;
        Ok(adapter)
    }

    /// Ensure system collections exist with correct flags (_system.scripts, _system.script_versions)
    fn ensure_system_collections(&self) -> Result<()> {
        let db = self.db.read();
        let collections = db.list_all_collections();

        let scripts_exists = collections.contains(&SCRIPTS_COLLECTION.to_string());
        let versions_exists = collections.contains(&SCRIPT_VERSIONS_COLLECTION.to_string());

        // Check flags on existing collections - fix if hidden != true
        let scripts_needs_flags = scripts_exists
            && db
                .get_collection_flags(SCRIPTS_COLLECTION)
                .map(|f| !f.hidden)
                .unwrap_or(true);
        let versions_needs_flags = versions_exists
            && db
                .get_collection_flags(SCRIPT_VERSIONS_COLLECTION)
                .map(|f| !f.hidden)
                .unwrap_or(true);

        if !scripts_exists || !versions_exists || scripts_needs_flags || versions_needs_flags {
            drop(db); // Release read lock
            let db = self.db.write(); // Need write lock to create/modify collections

            // Create if missing
            if !scripts_exists {
                db.create_system_collection(SCRIPTS_COLLECTION)?;
            }
            if !versions_exists {
                db.create_system_collection(SCRIPT_VERSIONS_COLLECTION)?;
            }

            // Fix flags on existing collections (legacy data migration)
            use ironbase_core::storage::CollectionFlags;
            let system_flags = CollectionFlags {
                is_system: true,
                protected: true,
                hidden: true,
            };
            if scripts_needs_flags {
                db.set_collection_flags(SCRIPTS_COLLECTION, system_flags)?;
            }
            if versions_needs_flags {
                db.set_collection_flags(SCRIPT_VERSIONS_COLLECTION, system_flags)?;
            }
        }
        Ok(())
    }

    // ============================================================
    // Database Management
    // ============================================================

    /// List all collections
    pub fn list_collections(&self) -> Vec<String> {
        let db = self.db.read();
        db.list_collections()
    }

    /// Create a new collection
    pub fn create_collection(&self, name: &str) -> Result<()> {
        let db = self.db.read();
        // Use collection() which creates the collection if it doesn't exist
        let _ = db.collection(name)?;
        Ok(())
    }

    /// Drop a collection
    pub fn drop_collection(&self, name: &str) -> Result<()> {
        let db = self.db.write();
        db.drop_collection(name)?;
        Ok(())
    }

    /// Get database statistics
    pub fn stats(&self) -> Value {
        let db = self.db.read();
        let db_path = self.db_path.read();
        serde_json::json!({
            "database_path": db_path.display().to_string(),
            "collections": db.list_collections(),
            "collection_count": db.list_collections().len(),
        })
    }

    /// Get current database path
    pub fn get_db_path(&self) -> String {
        self.db_path.read().display().to_string()
    }

    /// Switch to a different database file
    /// Returns the new database path on success
    pub fn switch_database(&self, new_path: &str, create_if_missing: bool) -> Result<String> {
        use crate::error::McpError;
        let path = std::path::Path::new(new_path);

        // Validate path
        if !create_if_missing && !path.exists() {
            return Err(McpError::InvalidParams(format!(
                "Database file does not exist: {}",
                new_path
            )));
        }

        if create_if_missing && path.exists() {
            return Err(McpError::InvalidParams(format!(
                "Database file already exists: {}",
                new_path
            )));
        }

        // Create parent directory if needed
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    McpError::Internal(format!("Failed to create directory: {}", e))
                })?;
            }
        }

        // Open new database (creates if needed)
        let new_db = DatabaseCore::open(path)
            .map_err(|e| McpError::Internal(format!("Failed to open database: {}", e)))?;

        // Swap the database (write lock)
        {
            let mut db_guard = self.db.write();
            *db_guard = new_db;
        }

        // Update path
        {
            let mut path_guard = self.db_path.write();
            *path_guard = path.to_path_buf();
        }

        // Ensure system collections exist in new database
        self.ensure_system_collections()?;

        Ok(new_path.to_string())
    }

    /// Compact the database
    pub fn compact(&self) -> Result<Value> {
        let db = self.db.write();
        let result = db.compact()?;
        Ok(serde_json::json!({
            "size_before": result.size_before,
            "size_after": result.size_after,
            "documents_scanned": result.documents_scanned,
            "documents_kept": result.documents_kept,
            "tombstones_removed": result.tombstones_removed,
        }))
    }

    /// Force checkpoint (flush to disk)
    pub fn checkpoint(&self) -> Result<()> {
        let db = self.db.write();
        db.checkpoint()?;
        Ok(())
    }

    // ============================================================
    // Document CRUD
    // ============================================================

    /// Convert Value to HashMap for insertion
    fn value_to_hashmap(value: Value) -> HashMap<String, Value> {
        match value {
            Value::Object(map) => map.into_iter().collect(),
            _ => HashMap::new(),
        }
    }

    /// Convert DocumentId to string
    fn doc_id_to_string(id: &ironbase_core::DocumentId) -> String {
        match id {
            ironbase_core::DocumentId::Int(i) => i.to_string(),
            ironbase_core::DocumentId::String(s) => s.clone(),
            ironbase_core::DocumentId::ObjectId(oid) => oid.clone(),
        }
    }

    /// Insert a single document (with WAL durability)
    pub fn insert_one(&self, collection: &str, document: Value) -> Result<String> {
        let db = self.db.read();
        let fields = Self::value_to_hashmap(document);
        let id = db.insert_one(collection, fields)?;
        Ok(Self::doc_id_to_string(&id))
    }

    /// Insert multiple documents (with WAL durability)
    pub fn insert_many(&self, collection: &str, documents: Vec<Value>) -> Result<Vec<String>> {
        let db = self.db.read();
        let docs: Vec<HashMap<String, Value>> =
            documents.into_iter().map(Self::value_to_hashmap).collect();
        let ids = db.insert_many(collection, docs)?;
        Ok(ids.iter().map(Self::doc_id_to_string).collect())
    }

    /// Find documents (uses get_collection - no implicit creation)
    pub fn find(&self, collection: &str, query: Value, options: FindOptions) -> Result<FindResult> {
        let db = self.db.read();
        let coll = db.get_collection(collection)?;

        // Get total count before limit/skip if requested
        let total = if options.include_total {
            Some(coll.count_documents(&query)? as usize)
        } else {
            None
        };

        // Convert to IronBase FindOptions - clean pass-through, no conversion needed
        let ironbase_options = ironbase_core::FindOptions {
            projection: options.projection.as_ref().and_then(|p| {
                p.as_object().map(|obj| {
                    obj.iter()
                        .map(|(k, v)| (k.clone(), v.as_i64().unwrap_or(1) as i32))
                        .collect()
                })
            }),
            // Sort already parsed by tools.rs - None means O(1) skip/limit
            sort: options.sort,
            limit: options.limit,
            skip: options.skip,
        };

        let documents = coll.find_with_options(&query, ironbase_options)?;
        Ok(FindResult { documents, total })
    }

    /// Find a single document (uses get_collection - no implicit creation)
    pub fn find_one(&self, collection: &str, query: Value) -> Result<Option<Value>> {
        let db = self.db.read();
        let coll = db.get_collection(collection)?;
        let result = coll.find_one(&query)?;
        Ok(result)
    }

    /// Update a single document (with WAL durability)
    pub fn update_one(
        &self,
        collection: &str,
        filter: Value,
        update: Value,
    ) -> Result<UpdateResult> {
        let db = self.db.read();
        let (matched, modified) = db.update_one(collection, &filter, &update)?;
        Ok(UpdateResult {
            matched_count: matched,
            modified_count: modified,
        })
    }

    /// Update multiple documents (with WAL durability)
    pub fn update_many(
        &self,
        collection: &str,
        filter: Value,
        update: Value,
    ) -> Result<UpdateResult> {
        let db = self.db.read();
        let (matched, modified) = db.update_many(collection, &filter, &update)?;
        Ok(UpdateResult {
            matched_count: matched,
            modified_count: modified,
        })
    }

    /// Delete a single document (with WAL durability)
    pub fn delete_one(&self, collection: &str, filter: Value) -> Result<u64> {
        let db = self.db.read();
        let count = db.delete_one(collection, &filter)?;
        Ok(count)
    }

    /// Delete multiple documents (with WAL durability)
    pub fn delete_many(&self, collection: &str, filter: Value) -> Result<u64> {
        let db = self.db.read();
        let count = db.delete_many(collection, &filter)?;
        Ok(count)
    }

    /// Count documents matching query (uses get_collection - no implicit creation)
    pub fn count_documents(&self, collection: &str, query: Value) -> Result<u64> {
        let db = self.db.read();
        let coll = db.get_collection(collection)?;
        let count = coll.count_documents(&query)?;
        Ok(count)
    }

    /// Get distinct values for a field (uses get_collection - no implicit creation)
    pub fn distinct(&self, collection: &str, field: &str, query: Value) -> Result<Vec<Value>> {
        let db = self.db.read();
        let coll = db.get_collection(collection)?;
        let values = coll.distinct(field, &query)?;
        Ok(values)
    }

    // ============================================================
    // Aggregation
    // ============================================================

    /// Execute aggregation pipeline (uses get_collection - no implicit creation)
    pub fn aggregate(&self, collection: &str, pipeline: Vec<Value>) -> Result<Vec<Value>> {
        let db = self.db.read();
        let coll = db.get_collection(collection)?;
        // Convert Vec<Value> to Value::Array
        let pipeline_value = Value::Array(pipeline);
        let results = coll.aggregate(&pipeline_value)?;
        Ok(results)
    }

    // ============================================================
    // Index Management
    // ============================================================

    /// Create an index
    pub fn create_index(&self, collection: &str, field: &str, unique: bool) -> Result<String> {
        let db = self.db.read();
        let coll = db.collection(collection)?;
        let name = coll.create_index(field.to_string(), unique)?;
        Ok(name)
    }

    /// Create a compound index
    pub fn create_compound_index(
        &self,
        collection: &str,
        fields: &[String],
        unique: bool,
    ) -> Result<String> {
        let db = self.db.read();
        let coll = db.collection(collection)?;
        let name = coll.create_compound_index(fields.to_vec(), unique)?;
        Ok(name)
    }

    /// List indexes on a collection (uses get_collection - no implicit creation)
    pub fn list_indexes(&self, collection: &str) -> Result<Vec<String>> {
        let db = self.db.read();
        let coll = db.get_collection(collection)?;
        let indexes = coll.list_indexes();
        Ok(indexes)
    }

    /// Drop an index (uses get_collection - no implicit creation)
    pub fn drop_index(&self, collection: &str, index_name: &str) -> Result<()> {
        let db = self.db.read();
        let coll = db.get_collection(collection)?;
        coll.drop_index(index_name)?;
        Ok(())
    }

    /// Explain query execution plan (uses get_collection - no implicit creation)
    pub fn explain(&self, collection: &str, query: Value) -> Result<Value> {
        let db = self.db.read();
        let coll = db.get_collection(collection)?;
        let plan = coll.explain(&query)?;
        Ok(plan)
    }

    /// Find documents with index hint (uses get_collection - no implicit creation)
    pub fn find_with_hint(
        &self,
        collection: &str,
        query: Value,
        hint: &str,
    ) -> Result<Vec<Value>> {
        let db = self.db.read();
        let coll = db.get_collection(collection)?;
        let documents = coll.find_with_hint(&query, hint)?;
        Ok(documents)
    }

    /// Create a fuzzy text index
    pub fn create_fuzzy_index(
        &self,
        collection: &str,
        field: &str,
        algorithm: &str,
        threshold: f64,
    ) -> Result<String> {
        use ironbase_core::FuzzyAlgorithm;

        let algo = match algorithm {
            "levenshtein" => FuzzyAlgorithm::Levenshtein,
            "damerau_levenshtein" => FuzzyAlgorithm::DamerauLevenshtein,
            _ => FuzzyAlgorithm::JaroWinkler, // default
        };

        let db = self.db.read();
        let coll = db.collection(collection)?;
        let name = coll.create_fuzzy_index(field.to_string(), algo, threshold)?;
        Ok(name)
    }

    /// Fuzzy search using the fuzzy index (returns documents with similarity scores)
    /// Uses get_collection - no implicit creation
    pub fn fuzzy_search(
        &self,
        collection: &str,
        field: &str,
        query: &str,
        threshold: Option<f64>,
        algorithm: Option<&str>,
    ) -> Result<Vec<(Value, f64)>> {
        use ironbase_core::FuzzyAlgorithm;

        let algo = algorithm.map(|a| match a {
            "levenshtein" => FuzzyAlgorithm::Levenshtein,
            "damerau_levenshtein" => FuzzyAlgorithm::DamerauLevenshtein,
            _ => FuzzyAlgorithm::JaroWinkler,
        });

        let db = self.db.read();
        let coll = db.get_collection(collection)?;
        let results = coll.fuzzy_search(field, query, threshold, algo)?;
        Ok(results)
    }

    // ============================================================
    // Schema Management
    // ============================================================

    /// Set schema for a collection
    pub fn set_schema(&self, collection: &str, schema: Option<Value>) -> Result<()> {
        let db = self.db.read();
        let coll = db.collection(collection)?;
        coll.set_schema(schema)?;
        Ok(())
    }

    /// Get schema for a collection (uses get_collection - no implicit creation)
    pub fn get_schema(&self, collection: &str) -> Result<Option<Value>> {
        let db = self.db.read();
        let coll = db.get_collection(collection)?;
        Ok(coll.get_schema())
    }

    // ============================================================
    // Admin Operations
    // ============================================================

    /// List ALL collections including hidden/system collections
    pub fn list_all_collections(&self) -> Vec<String> {
        let db = self.db.read();
        db.list_all_collections()
    }

    /// Create a system collection with is_system, protected, hidden flags
    pub fn create_system_collection(&self, name: &str) -> Result<()> {
        let db = self.db.read();
        db.create_system_collection(name)?;
        Ok(())
    }

    /// Set collection flags (only sets flags that are Some)
    pub fn set_collection_flags(
        &self,
        collection: &str,
        is_system: Option<bool>,
        protected: Option<bool>,
        hidden: Option<bool>,
    ) -> Result<()> {
        let db = self.db.read();
        // Get existing flags first
        let mut flags = db
            .get_collection_flags(collection)
            .unwrap_or_default();

        // Only update flags that are explicitly set
        if let Some(v) = is_system {
            flags.is_system = v;
        }
        if let Some(v) = protected {
            flags.protected = v;
        }
        if let Some(v) = hidden {
            flags.hidden = v;
        }

        db.set_collection_flags(collection, flags)?;
        Ok(())
    }

    /// Force drop a collection, ignoring protected flag
    pub fn force_drop_collection(&self, name: &str) -> Result<()> {
        let db = self.db.write();
        db.force_drop_collection(name)?;
        Ok(())
    }

    // ============================================================
    // Transaction Management (Read Committed Isolation)
    // ============================================================

    /// Begin a new transaction
    /// Returns the transaction ID as a string
    pub fn begin_transaction(&self) -> u64 {
        let db = self.db.read();
        db.begin_transaction()
    }

    /// Commit a transaction
    pub fn commit_transaction(&self, tx_id: u64) -> Result<()> {
        let db = self.db.read();
        db.commit_transaction(tx_id)?;
        Ok(())
    }

    /// Rollback a transaction
    pub fn rollback_transaction(&self, tx_id: u64) -> Result<()> {
        let db = self.db.read();
        db.rollback_transaction(tx_id)?;
        Ok(())
    }

    /// Insert a document within a transaction
    pub fn insert_one_tx(&self, collection: &str, document: Value, tx_id: u64) -> Result<String> {
        let db = self.db.read();
        let fields = Self::value_to_hashmap(document);
        let id = db.insert_one_tx(collection, fields, tx_id)?;
        Ok(Self::doc_id_to_string(&id))
    }

    /// Update a document within a transaction
    pub fn update_one_tx(
        &self,
        collection: &str,
        filter: Value,
        update: Value,
        tx_id: u64,
    ) -> Result<UpdateResult> {
        let db = self.db.read();
        let (matched, modified) = db.update_one_tx(collection, &filter, update, tx_id)?;
        Ok(UpdateResult {
            matched_count: matched,
            modified_count: modified,
        })
    }

    /// Delete a document within a transaction
    pub fn delete_one_tx(&self, collection: &str, filter: Value, tx_id: u64) -> Result<u64> {
        let db = self.db.read();
        let count = db.delete_one_tx(collection, &filter, tx_id)?;
        Ok(count)
    }

    /// Check if there's an active write transaction
    pub fn has_active_write_transaction(&self) -> bool {
        let db = self.db.read();
        db.has_active_write_transaction()
    }

    /// Get the current write lock holder (if any)
    pub fn get_write_lock_holder(&self) -> Option<u64> {
        let db = self.db.read();
        db.get_write_lock_holder()
    }
}
