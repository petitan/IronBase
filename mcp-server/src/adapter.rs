//! IronBase Adapter - Direct wrapper around IronBase core
//!
//! CONCURRENCY CONTROL (2026-01):
//! Heavy operations (aggregate without $match) are throttled to prevent OOM.
//! See `HEAVY_OP_SEMAPHORE` for configuration.

use crate::error::Result;
use ironbase_core::{storage::StorageEngine, DatabaseCore};
use parking_lot::RwLock;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Semaphore;

// ============================================================================
// In-Memory Collection Stats (for fast db_stats without lock contention)
// ============================================================================

/// In-memory document counts per collection.
/// This eliminates the need for count_documents() calls in stats(),
/// which previously held the database lock for 89+ seconds on large datasets.
struct CollectionStats {
    counts: RwLock<HashMap<String, u64>>,
}

impl CollectionStats {
    fn new() -> Self {
        Self {
            counts: RwLock::new(HashMap::new()),
        }
    }

    /// Get document count for a collection (returns 0 if not found)
    fn get(&self, collection: &str) -> u64 {
        self.counts.read().get(collection).copied().unwrap_or(0)
    }

    /// Get document count if tracked (returns None if not found)
    fn get_if_present(&self, collection: &str) -> Option<u64> {
        self.counts.read().get(collection).copied()
    }

    /// Set document count for a collection
    fn set(&self, collection: &str, count: u64) {
        self.counts.write().insert(collection.to_string(), count);
    }

    /// Increment or decrement count by delta (handles negative values)
    fn increment(&self, collection: &str, delta: i64) {
        let mut counts = self.counts.write();
        let current = counts.get(collection).copied().unwrap_or(0);
        let new_count = (current as i64 + delta).max(0) as u64;
        counts.insert(collection.to_string(), new_count);
    }

    /// Remove a collection from tracking (on drop_collection)
    fn remove(&self, collection: &str) {
        self.counts.write().remove(collection);
    }

    /// Clear all counts (used when switching databases)
    fn clear(&self) {
        self.counts.write().clear();
    }
}

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
/// Maximum concurrent heavy operations (aggregate without $match, full collection scans)
/// Set to 1 to prevent memory exhaustion on large collections
const MAX_CONCURRENT_HEAVY_OPS: usize = 1;

/// Timeout for waiting to acquire heavy operation permit (in seconds)
const HEAVY_OP_TIMEOUT_SECS: u64 = 30;

pub struct IronBaseAdapter {
    db: Arc<RwLock<DatabaseCore<StorageEngine>>>,
    /// Database file path (stored for stats, wrapped in RwLock for dynamic switching)
    db_path: RwLock<std::path::PathBuf>,
    /// In-memory document counts for fast stats() without lock contention
    collection_stats: CollectionStats,
    /// Semaphore to limit concurrent heavy operations (aggregate without $match)
    /// This prevents OOM when multiple full-collection scans run simultaneously
    heavy_op_semaphore: Arc<Semaphore>,
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
            collection_stats: CollectionStats::new(),
            heavy_op_semaphore: Arc::new(Semaphore::new(MAX_CONCURRENT_HEAVY_OPS)),
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
    /// and populating in-memory document counts for fast stats().
    ///
    /// This should be called after server startup to avoid slow first queries.
    /// Index managers are initialized lazily, so the first access to a collection
    /// triggers a full index rebuild from disk. Calling this method proactively
    /// moves that cost to startup time.
    ///
    /// Also initializes document counts in memory so stats() can return instantly
    /// without scanning collections (which previously caused 89s lock contention).
    ///
    /// Returns the number of collections warmed up and total time taken.
    pub fn warm_up(&self) -> (usize, std::time::Duration) {
        let start = std::time::Instant::now();
        let db = self.db.read();
        // Get ALL collections including system ones for accurate counts
        let all_collections: Vec<String> = db.list_all_collections();
        let user_collections: Vec<String> = all_collections
            .iter()
            .filter(|name| !name.starts_with("_system."))
            .cloned()
            .collect();
        drop(db);

        let total = user_collections.len();
        tracing::info!("Starting warm-up for {} collections...", total);

        // Warm up user collections and count documents
        for (i, name) in user_collections.iter().enumerate() {
            let coll_start = std::time::Instant::now();
            let db = self.db.read();
            match db.collection(name) {
                Ok(coll) => {
                    // Initialize document count in memory (one-time cost at startup)
                    let count = coll.count_documents(&serde_json::json!({})).unwrap_or(0);
                    self.collection_stats.set(name, count);

                    let elapsed = coll_start.elapsed();
                    if elapsed.as_millis() > 100 {
                        // Only log slow collections
                        tracing::info!(
                            "Warmed up [{}/{}] '{}' ({} docs) in {:.2}s",
                            i + 1,
                            total,
                            name,
                            count,
                            elapsed.as_secs_f64()
                        );
                    } else {
                        tracing::debug!(
                            "Warmed up [{}/{}] '{}' ({} docs) in {}ms",
                            i + 1,
                            total,
                            name,
                            count,
                            elapsed.as_millis()
                        );
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to warm up collection '{}': {}", name, e);
                }
            }
        }

        // Also count system collections (they're small, but need accurate stats)
        let db = self.db.read();
        for name in all_collections.iter().filter(|n| n.starts_with("_system.")) {
            if let Ok(coll) = db.collection(name) {
                let count = coll.count_documents(&serde_json::json!({})).unwrap_or(0);
                self.collection_stats.set(name, count);
            }
        }
        drop(db);

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
        // Remove from in-memory stats
        self.collection_stats.remove(name);
        Ok(())
    }

    /// Get database statistics
    ///
    /// PERFORMANCE: Uses in-memory document counts instead of count_documents().
    /// This reduces lock time from ~89s to <100ms on large datasets (78K docs).
    /// Counts are initialized during warm_up() and updated by insert/delete operations.
    pub fn stats(&self) -> Value {
        let db = self.db.read();
        let db_path = self.db_path.read();
        let path_str = db_path.display().to_string();

        // Get file size (no lock needed - filesystem operation)
        let file_size = std::fs::metadata(&*db_path).map(|m| m.len()).unwrap_or(0);
        drop(db_path); // Release path lock early

        // Format human-readable file size
        let file_size_human = format_bytes(file_size);

        // Get collection details with document counts from memory and indexes from DB
        // NOTE: We only hold the db lock briefly for list_collections and list_indexes
        let collections: Vec<Value> = db
            .list_collections()
            .into_iter()
            .filter(|name| !name.starts_with("_system.")) // Hide system collections
            .map(|name| {
                // PERFORMANCE: Read from in-memory cache instead of count_documents()
                // This was the bottleneck: 89s lock time for 78K docs
                let doc_count = self.collection_stats.get(&name);

                let indexes = db
                    .collection(&name)
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

        drop(db); // Release db lock as soon as possible

        // Calculate totals (no lock needed - just iterating local Vec)
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

    /// Get cached document count if available (no DB scan)
    pub fn collection_count_cached(&self, collection: &str) -> Option<u64> {
        self.collection_stats.get_if_present(collection)
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

        // Clear old stats before updating path
        self.collection_stats.clear();

        // Update path
        {
            let mut path_guard = self.db_path.write();
            *path_guard = path.to_path_buf();
        }

        // Ensure system collections exist in new database
        self.ensure_system_collections()?;

        // Re-initialize collection counts for new database
        self.warm_up();

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

    /// Graceful shutdown - flush indexes and mark clean shutdown
    ///
    /// This enables fast restart by allowing the next startup to trust
    /// persisted .idx files without rebuilding indexes from documents.
    ///
    /// Call this before dropping the adapter for graceful shutdown.
    pub fn close(&self) -> Result<()> {
        let db = self.db.write();
        db.close()
            .map_err(|e| crate::error::McpError::Storage(e.to_string()))?;
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
        // Update in-memory count
        self.collection_stats.increment(collection, 1);
        Ok(Self::doc_id_to_string(&id))
    }

    /// Insert multiple documents (with WAL durability)
    pub fn insert_many(&self, collection: &str, documents: Vec<Value>) -> Result<Vec<String>> {
        let count = documents.len() as i64;
        let db = self.db.read();
        let docs: Vec<HashMap<String, Value>> =
            documents.into_iter().map(Self::value_to_hashmap).collect();
        let ids = db.insert_many(collection, docs)?;
        // Update in-memory count
        self.collection_stats.increment(collection, count);
        Ok(ids.iter().map(Self::doc_id_to_string).collect())
    }

    /// Find documents (uses get_collection - no implicit creation)
    pub fn find(&self, collection: &str, query: Value, options: FindOptions) -> Result<FindResult> {
        let db = self.db.read();
        let coll = db.get_collection(collection)?;

        // Convert to IronBase FindOptions - now uses core's include_total
        let projection = if let Some(proj) = options.projection.as_ref() {
            let obj = proj.as_object().ok_or_else(|| {
                crate::error::McpError::InvalidParams(
                    "projection must be an object like {\"field\": 1} or {\"field\": 0}".into(),
                )
            })?;
            let mut map = HashMap::new();
            for (k, v) in obj {
                let val = if let Some(i) = v.as_i64() {
                    if i != 0 && i != 1 {
                        return Err(crate::error::McpError::InvalidParams(format!(
                            "Invalid projection value for '{}': expected 0 or 1, got {}",
                            k, i
                        )));
                    }
                    i as i32
                } else if let Some(f) = v.as_f64() {
                    if f == 0.0 {
                        0
                    } else if f == 1.0 {
                        1
                    } else {
                        return Err(crate::error::McpError::InvalidParams(format!(
                            "Invalid projection value for '{}': expected 0 or 1, got {}",
                            k, f
                        )));
                    }
                } else {
                    return Err(crate::error::McpError::InvalidParams(format!(
                        "Invalid projection value for '{}': expected 0 or 1, got {:?}",
                        k, v
                    )));
                };
                map.insert(k.clone(), val);
            }
            Some(map)
        } else {
            None
        };

        let ironbase_options = ironbase_core::FindOptions {
            projection,
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
        // Update in-memory count
        if count > 0 {
            self.collection_stats.increment(collection, -(count as i64));
        }
        Ok(count)
    }

    /// Delete multiple documents (with WAL durability)
    pub fn delete_many(&self, collection: &str, filter: Value) -> Result<u64> {
        let db = self.db.read();
        let count = db.delete_many(collection, &filter)?;
        // Update in-memory count
        if count > 0 {
            self.collection_stats.increment(collection, -(count as i64));
        }
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

    /// Check if a pipeline has a leading non-empty $match stage
    /// Used to determine if this is a "heavy" operation that needs throttling.
    /// Note: This does not verify index usage.
    fn has_leading_match(pipeline: &[Value]) -> bool {
        if let Some(first_stage) = pipeline.first() {
            if let Some(obj) = first_stage.as_object() {
                if let Some(match_value) = obj.get("$match") {
                    return match_value
                        .as_object()
                        .map(|m| !m.is_empty())
                        .unwrap_or(false);
                }
            }
        }
        false
    }

    /// Execute aggregation pipeline (sync version - no throttling)
    /// For internal use only. External callers should use aggregate_async().
    ///
    /// Uses `aggregate_auto()` which automatically scales memory limits based on
    /// available system RAM, preventing OOM on resource-constrained servers.
    fn aggregate_internal(&self, collection: &str, pipeline: Vec<Value>) -> Result<Vec<Value>> {
        let db = self.db.read();
        let coll = db.get_collection(collection)?;
        // Convert Vec<Value> to Value::Array
        let pipeline_value = Value::Array(pipeline);
        // Use aggregate_auto() for dynamic memory limits based on system RAM
        let results = coll.aggregate_auto(&pipeline_value)?;
        Ok(results)
    }

    /// Execute aggregation pipeline with concurrency throttling
    ///
    /// Heavy operations (aggregates without leading $match) are throttled to
    /// prevent OOM when multiple full-collection scans run simultaneously.
    ///
    /// # Throttling Rules
    /// - Pipelines WITH non-empty `$match` as first stage: No throttling (may still scan)
    /// - Pipelines WITHOUT `$match`: Limited to MAX_CONCURRENT_HEAVY_OPS concurrent
    ///
    /// # Errors
    /// - Returns error if timeout waiting for heavy operation slot
    /// - Returns error if aggregation itself fails
    pub async fn aggregate_async(
        &self,
        collection: &str,
        pipeline: Vec<Value>,
    ) -> Result<Vec<Value>> {
        let is_heavy = !Self::has_leading_match(&pipeline);

        if is_heavy {
            // Try to acquire semaphore with timeout
            let permit = tokio::time::timeout(
                std::time::Duration::from_secs(HEAVY_OP_TIMEOUT_SECS),
                self.heavy_op_semaphore.acquire(),
            )
            .await;

            match permit {
                Ok(Ok(_permit)) => {
                    // Got permit - execute the heavy operation
                    // The permit is automatically released when dropped
                    tracing::info!(
                        collection = collection,
                        "Heavy aggregate started (no $match, full scan)"
                    );
                    let result = self.aggregate_internal(collection, pipeline);
                    tracing::info!(
                        collection = collection,
                        success = result.is_ok(),
                        "Heavy aggregate completed"
                    );
                    result
                }
                Ok(Err(_)) => {
                    // Semaphore closed (shouldn't happen)
                    Err(crate::error::McpError::Internal(
                        "Heavy operation semaphore closed unexpectedly".to_string(),
                    ))
                }
                Err(_) => {
                    // Timeout waiting for permit
                    Err(crate::error::McpError::OperationTooLarge(format!(
                        "Timeout waiting for heavy aggregate slot. Another full-collection \
                         aggregate is in progress. Either wait {}s or add a $match stage \
                         to filter documents first.",
                        HEAVY_OP_TIMEOUT_SECS
                    )))
                }
            }
        } else {
            // Light operation - no throttling needed
            self.aggregate_internal(collection, pipeline)
        }
    }

    /// Execute aggregation pipeline (sync version)
    /// Uses throttling when called inside a tokio runtime, otherwise runs directly.
    pub fn aggregate(&self, collection: &str, pipeline: Vec<Value>) -> Result<Vec<Value>> {
        // Check if we're in a tokio runtime
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            // We're in a tokio runtime - use block_in_place for sync call
            let is_heavy = !Self::has_leading_match(&pipeline);
            if is_heavy {
                // For heavy ops, we need to handle async properly
                // Since we can't easily do async here, just run without throttle
                // The async version should be preferred for MCP calls
                tracing::warn!(
                    collection = collection,
                    "Heavy aggregate called via sync API - consider using aggregate_async"
                );
            }
            // Use block_in_place to avoid blocking the runtime
            tokio::task::block_in_place(|| {
                handle.block_on(self.aggregate_async(collection, pipeline))
            })
        } else {
            // Not in tokio runtime - just run directly
            self.aggregate_internal(collection, pipeline)
        }
    }

    // ============================================================
    // Index Management
    // ============================================================

    /// Create an index
    ///
    /// # Arguments
    /// * `collection` - Collection name
    /// * `field` - Field to index
    /// * `unique` - Whether values must be unique
    /// * `sparse` - If true, documents missing the field are not indexed
    pub fn create_index(
        &self,
        collection: &str,
        field: &str,
        unique: bool,
        sparse: bool,
    ) -> Result<String> {
        let db = self.db.read();
        let coll = db.collection(collection)?;
        let name = coll.create_index(field.to_string(), unique, sparse)?;
        Ok(name)
    }

    /// Create a compound index
    ///
    /// # Arguments
    /// * `collection` - Collection name
    /// * `fields` - Fields to index (in order)
    /// * `unique` - Whether compound key must be unique
    /// * `sparse` - If true, documents missing any field are not indexed
    pub fn create_compound_index(
        &self,
        collection: &str,
        fields: &[String],
        unique: bool,
        sparse: bool,
    ) -> Result<String> {
        let db = self.db.read();
        let coll = db.collection(collection)?;
        let name = coll.create_compound_index(fields.to_vec(), unique, sparse)?;
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
        // Remove from in-memory stats
        self.collection_stats.remove(name);
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
        // Update in-memory count
        self.collection_stats.increment(collection, 1);
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
        // Update in-memory count
        if count > 0 {
            self.collection_stats.increment(collection, -(count as i64));
        }
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
