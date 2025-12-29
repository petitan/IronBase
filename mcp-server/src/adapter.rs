//! IronBase Adapter - Direct wrapper around IronBase core

use crate::error::Result;
use ironbase_core::{storage::StorageEngine, DatabaseCore};
use parking_lot::RwLock;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

/// Format bytes as human-readable string (e.g., "15.50 GB")
fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    const TB: u64 = 1024 * GB;

    if bytes >= TB {
        format!("{:.2} TB", bytes as f64 / TB as f64)
    } else if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

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

/// Full-text search options
#[derive(Debug, Default)]
pub struct FulltextSearchOptions {
    pub limit: Option<usize>,
    pub skip: Option<usize>,
    pub min_score: Option<f64>,
    pub projection: Option<HashMap<String, i32>>,
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

/// API keys collection name
pub const API_KEYS_COLLECTION: &str = "_system.api_keys";

/// ACL rules collection name
pub const ACL_COLLECTION: &str = "_system.acl";

/// Listeners collection name
pub const LISTENERS_COLLECTION: &str = "_system.listeners";

// ============================================================================
// System Collection Schemas
// ============================================================================

/// Get strict JSON schema for _system.scripts
/// Note: _id field serves as the script name (string identifier)
/// Optional fields (description, created_at, etc.) are not type-checked to allow null values
fn scripts_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "required": ["_id", "code", "version", "tags", "dependencies"],
        "properties": {
            "_id": {
                "type": "string",
                "pattern": "^[a-zA-Z_][a-zA-Z0-9_-]{0,63}$"
            },
            "code": {
                "type": "string"
            },
            "version": {
                "type": "integer"
            },
            "tags": {
                "type": "array"
            },
            "dependencies": {
                "type": "array"
            }
        }
    })
}

/// Get strict JSON schema for _system.script_versions
fn script_versions_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "required": ["script_name", "version", "code", "created_at", "tags", "dependencies"],
        "properties": {
            "script_name": {
                "type": "string",
                "pattern": "^[a-zA-Z_][a-zA-Z0-9_-]{0,63}$"
            },
            "version": {
                "type": "integer"
            },
            "code": {
                "type": "string"
            },
            "tags": {
                "type": "array"
            },
            "dependencies": {
                "type": "array"
            },
            "created_at": {
                "type": "string"
            }
        }
    })
}

/// Get strict JSON schema for _system.api_keys
fn api_keys_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "required": ["_id", "key", "name", "created_at", "enabled"],
        "properties": {
            "_id": {
                "type": "integer"
            },
            "key": {
                "type": "string",
                "pattern": "^sk-[a-zA-Z0-9]{32,64}$"
            },
            "name": {
                "type": "string",
                "pattern": "^[a-zA-Z0-9_-]{1,64}$"
            },
            "created_at": {
                "type": "string",
                "pattern": "^\\d{4}-\\d{2}-\\d{2}T\\d{2}:\\d{2}:\\d{2}"
            },
            "enabled": {
                "type": "boolean"
            }
        }
    })
}

/// Get strict JSON schema for _system.acl
fn acl_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "required": ["collection", "rules"],
        "properties": {
            "collection": {
                "type": "string",
                "pattern": "^[a-zA-Z_*][a-zA-Z0-9_.*-]{0,127}$"
            },
            "rules": {
                "type": "array",
                "minItems": 1
            }
        }
    })
}

/// Get strict JSON schema for _system.listeners
fn listeners_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "required": ["_id", "bind"],
        "properties": {
            "_id": {
                "type": "string",
                "pattern": "^[a-zA-Z_][a-zA-Z0-9_-]{0,63}$"
            },
            "bind": {
                "type": "string",
                "pattern": "^[0-9a-fA-F.:]+:[0-9]{1,5}$"
            },
            "tls": {
                "type": "boolean"
            },
            "cert_path": {
                "type": "string"
            },
            "key_path": {
                "type": "string"
            },
            "enabled": {
                "type": "boolean"
            },
            "description": {
                "type": "string"
            }
        }
    })
}

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

    /// Ensure system collections exist with correct flags and schemas
    fn ensure_system_collections(&self) -> Result<()> {
        use ironbase_core::storage::CollectionFlags;

        let system_flags = CollectionFlags {
            is_system: true,
            protected: true,
            hidden: true,
        };

        // Define all system collections with their schemas
        let system_collections: &[(&str, serde_json::Value)] = &[
            (SCRIPTS_COLLECTION, scripts_schema()),
            (SCRIPT_VERSIONS_COLLECTION, script_versions_schema()),
            (API_KEYS_COLLECTION, api_keys_schema()),
            (ACL_COLLECTION, acl_schema()),
            (LISTENERS_COLLECTION, listeners_schema()),
        ];

        let db = self.db.read();
        let existing_collections = db.list_all_collections();
        drop(db);

        // Process each system collection
        for (collection_name, schema) in system_collections {
            let exists = existing_collections.contains(&collection_name.to_string());

            if !exists {
                // Create collection with flags and schema
                let db = self.db.write();
                db.create_system_collection(collection_name)?;
                db.set_collection_flags(collection_name, system_flags)?;

                // Set schema - SECURITY FIX: Log errors instead of ignoring
                if let Ok(coll) = db.collection(collection_name) {
                    if let Err(e) = coll.set_schema(Some(schema.clone())) {
                        tracing::error!(
                            "SECURITY WARNING: Failed to set schema for {}: {}. \
                             Collection may accept invalid documents!",
                            collection_name,
                            e
                        );
                    }
                }
            } else {
                // Ensure flags are correct
                let db = self.db.read();
                let needs_flags = db
                    .get_collection_flags(collection_name)
                    .map(|f| !f.hidden || !f.protected || !f.is_system)
                    .unwrap_or(true);

                // Check if schema is set
                let needs_schema = db
                    .get_collection(collection_name)
                    .ok()
                    .and_then(|c| c.get_schema().ok().flatten())
                    .is_none();

                drop(db);

                if needs_flags || needs_schema {
                    let db = self.db.write();

                    if needs_flags {
                        db.set_collection_flags(collection_name, system_flags)?;
                    }

                    // SECURITY FIX: Log errors instead of ignoring
                    if needs_schema {
                        if let Ok(coll) = db.collection(collection_name) {
                            if let Err(e) = coll.set_schema(Some(schema.clone())) {
                                tracing::error!(
                                    "SECURITY WARNING: Failed to set schema for {}: {}. \
                                     Collection may accept invalid documents!",
                                    collection_name,
                                    e
                                );
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }

    // ============================================================
    // Warm-up
    // ============================================================

    /// Warm up all collections by initializing their index managers
    ///
    /// This should be called after server startup to avoid slow first queries.
    /// Index managers are initialized lazily, so the first access to a collection
    /// triggers a full index rebuild from disk. Calling this method proactively
    /// moves that cost to startup time.
    ///
    /// Returns the number of collections warmed up and total time taken.
    pub fn warm_up(&self) -> (usize, std::time::Duration) {
        let start = std::time::Instant::now();
        let db = self.db.read();
        let collections: Vec<String> = db
            .list_collections()
            .into_iter()
            .filter(|name| !name.starts_with("_system."))
            .collect();
        drop(db);

        let total = collections.len();
        tracing::info!(
            "Starting warm-up for {} collections...",
            total
        );

        for (i, name) in collections.iter().enumerate() {
            let coll_start = std::time::Instant::now();
            let db = self.db.read();
            match db.collection(name) {
                Ok(_) => {
                    let elapsed = coll_start.elapsed();
                    if elapsed.as_millis() > 100 {
                        // Only log slow collections
                        tracing::info!(
                            "Warmed up [{}/{}] '{}' in {:.2}s",
                            i + 1,
                            total,
                            name,
                            elapsed.as_secs_f64()
                        );
                    } else {
                        tracing::debug!(
                            "Warmed up [{}/{}] '{}' in {}ms",
                            i + 1,
                            total,
                            name,
                            elapsed.as_millis()
                        );
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to warm up collection '{}': {}",
                        name,
                        e
                    );
                }
            }
        }

        let elapsed = start.elapsed();
        tracing::info!(
            "Warm-up complete: {} collections in {:.2}s",
            total,
            elapsed.as_secs_f64()
        );

        (total, elapsed)
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
        let path_str = db_path.display().to_string();

        // Get file size
        let file_size = std::fs::metadata(&*db_path).map(|m| m.len()).unwrap_or(0);

        // Format human-readable file size
        let file_size_human = if file_size >= 1_073_741_824 {
            format!("{:.2} GB", file_size as f64 / 1_073_741_824.0)
        } else if file_size >= 1_048_576 {
            format!("{:.2} MB", file_size as f64 / 1_048_576.0)
        } else if file_size >= 1024 {
            format!("{:.2} KB", file_size as f64 / 1024.0)
        } else {
            format!("{} B", file_size)
        };

        // Get collection details with document counts and indexes
        let collections: Vec<Value> = db
            .list_collections()
            .into_iter()
            .filter(|name| !name.starts_with("_system.")) // Hide system collections
            .map(|name| {
                let doc_count = db.collection(&name)
                    .ok()
                    .map(|c| c.count_documents(&serde_json::json!({})).unwrap_or(0))
                    .unwrap_or(0);

                let indexes = db.collection(&name)
                    .ok()
                    .and_then(|c| c.list_indexes().ok())
                    .unwrap_or_default();

                serde_json::json!({
                    "name": name,
                    "document_count": doc_count,
                    "index_count": indexes.len(),
                    "indexes": indexes,
                })
            })
            .collect();

        // Calculate totals
        let total_documents: u64 = collections
            .iter()
            .filter_map(|c| c.get("document_count").and_then(|v| v.as_u64()))
            .sum();

        let total_indexes: usize = collections
            .iter()
            .filter_map(|c| c.get("index_count").and_then(|v| v.as_u64()))
            .map(|v| v as usize)
            .sum();

        serde_json::json!({
            "database": {
                "path": path_str,
                "file_size_bytes": file_size,
                "file_size": file_size_human,
            },
            "collections": collections,
            "summary": {
                "collection_count": collections.len(),
                "total_documents": total_documents,
                "total_indexes": total_indexes,
            }
        })
    }

    /// Get current database path
    pub fn get_db_path(&self) -> String {
        self.db_path.read().display().to_string()
    }

    /// Switch to a different database file
    /// Returns the new database path on success
    /// BUG #3 fix: Acquire write lock BEFORE existence check to prevent TOCTOU race
    pub fn switch_database(&self, new_path: &str, create_if_missing: bool) -> Result<String> {
        use crate::error::McpError;
        let path = std::path::Path::new(new_path);

        // BUG #3 fix: Acquire write lock FIRST to make existence check + open atomic
        let mut db_guard = self.db.write();

        // Validate path (now atomic with the lock held)
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

        // Open new database (creates if needed) - still under write lock
        let new_db = DatabaseCore::open(path)
            .map_err(|e| McpError::Internal(format!("Failed to open database: {}", e)))?;

        // Swap the database (already holding write lock)
        *db_guard = new_db;
        drop(db_guard); // Explicitly release db lock before acquiring path lock

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
        let start = std::time::Instant::now();
        let db = self.db.write();
        let result = db.compact()?;
        let duration_ms = start.elapsed().as_millis() as u64;

        let space_freed = result.size_before.saturating_sub(result.size_after);

        Ok(serde_json::json!({
            "success": true,
            "size_before": format_bytes(result.size_before),
            "size_after": format_bytes(result.size_after),
            "space_freed": format_bytes(space_freed),
            "size_before_bytes": result.size_before,
            "size_after_bytes": result.size_after,
            "space_freed_bytes": space_freed,
            "documents_scanned": result.documents_scanned,
            "documents_kept": result.documents_kept,
            "tombstones_removed": result.tombstones_removed,
            "compression_ratio": format!("{:.1}%", result.compression_ratio()),
            "duration_ms": duration_ms,
        }))
    }

    /// Force checkpoint (flush to disk)
    ///
    /// Returns checkpoint statistics including WAL size before/after.
    pub fn checkpoint(&self) -> Result<Value> {
        let start = std::time::Instant::now();
        let db = self.db.write();
        let result = db.checkpoint()?;
        let duration_ms = start.elapsed().as_millis() as u64;

        Ok(serde_json::json!({
            "success": true,
            "wal_size_before": format_bytes(result.wal_size_before),
            "wal_size_after": format_bytes(result.wal_size_after),
            "wal_size_before_bytes": result.wal_size_before,
            "wal_size_after_bytes": result.wal_size_after,
            "wal_ops_cleared": result.wal_ops_cleared,
            "duration_ms": duration_ms,
        }))
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

        // Convert to IronBase FindOptions - now uses core's include_total
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
            include_total: options.include_total,
        };

        // Use core's find_with_result which handles count internally
        let result = coll.find_with_result(&query, ironbase_options)?;
        Ok(FindResult {
            documents: result.documents,
            total: result.total.map(|t| t as usize),
        })
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
        let indexes = coll.list_indexes()?;
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
    pub fn find_with_hint(&self, collection: &str, query: Value, hint: &str) -> Result<Vec<Value>> {
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
    // Full-Text Search
    // ============================================================

    /// Create a full-text index with language support
    pub fn create_fulltext_index(
        &self,
        collection: &str,
        field: &str,
        language: &str,
        min_word_length: Option<usize>,
        accent_folding: Option<bool>,
    ) -> Result<String> {
        let db = self.db.read();
        let coll = db.collection(collection)?;
        let name = coll.create_fulltext_index(
            field.to_string(),
            language,
            min_word_length,
            accent_folding,
        )?;
        Ok(name)
    }

    /// Full-text search using the fulltext index (returns documents with scores and matched tokens)
    /// Uses get_collection - no implicit creation
    pub fn fulltext_search(
        &self,
        collection: &str,
        field: &str,
        query: &str,
        options: FulltextSearchOptions,
    ) -> Result<Vec<(Value, f64, Vec<String>)>> {
        let db = self.db.read();
        let coll = db.get_collection(collection)?;
        let results = coll.fulltext_search(
            field,
            query,
            options.limit,
            options.skip,
            options.min_score,
            options.projection,
        )?;
        Ok(results)
    }

    /// List all fulltext indexes for a collection
    /// Uses get_collection - no implicit creation
    pub fn list_fulltext_indexes(&self, collection: &str) -> Result<Vec<Value>> {
        let db = self.db.read();
        let coll = db.get_collection(collection)?;
        let indexes = coll.list_fulltext_indexes()?;
        Ok(indexes
            .into_iter()
            .map(|idx| {
                serde_json::json!({
                    "name": idx.name,
                    "field": idx.field,
                    "language": format!("{:?}", idx.language).to_lowercase(),
                    "min_word_length": idx.min_word_length,
                    "accent_folding": idx.accent_folding,
                    "num_documents": idx.num_documents,
                    "num_tokens": idx.num_tokens
                })
            })
            .collect())
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
        Ok(coll.get_schema()?)
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
    /// BUG #9 fix: Use write lock for flag modification (was read lock - race condition)
    pub fn set_collection_flags(
        &self,
        collection: &str,
        is_system: Option<bool>,
        protected: Option<bool>,
        hidden: Option<bool>,
    ) -> Result<()> {
        let db = self.db.write();
        // Get existing flags first
        let mut flags = db.get_collection_flags(collection).unwrap_or_default();

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
    /// BUG #1 fix: Use write lock for transaction commit (was read lock - race condition)
    pub fn commit_transaction(&self, tx_id: u64) -> Result<()> {
        let db = self.db.write();
        db.commit_transaction(tx_id)?;
        Ok(())
    }

    /// Rollback a transaction
    /// BUG #1 fix: Use write lock for transaction rollback (was read lock - race condition)
    pub fn rollback_transaction(&self, tx_id: u64) -> Result<()> {
        let db = self.db.write();
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
