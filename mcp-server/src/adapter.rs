//! IronBase Adapter - Direct wrapper around IronBase core

use crate::error::{McpError, Result};
use crate::execution;
use ironbase_core::aggregation::{AggregationLimitContext, AggregationLimits};
use ironbase_core::ExecutionContext;
use ironbase_core::{storage::StorageEngine, DatabaseCore};
use parking_lot::RwLock;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

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
        let new_count_raw = current as i64 + delta;
        if new_count_raw < 0 {
            // Log warning when count goes negative - indicates stats are out of sync
            tracing::warn!(
                "Collection '{}' stats count went negative: {} + {} = {} (clamping to 0). \
                 Stats may be out of sync with actual document count.",
                collection,
                current,
                delta,
                new_count_raw
            );
        }
        let new_count = new_count_raw.max(0) as u64;
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
    /// Maximum response size in bytes (OOM protection)
    /// When set, documents are loaded until this limit would be exceeded.
    pub max_response_bytes: Option<usize>,
    /// Cancellation flag for cooperative timeout support
    /// When set to true, the find operation will abort and return an error.
    pub cancel_flag: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
}

/// Find result with optional total count
#[derive(Debug)]
pub struct FindResult {
    pub documents: Vec<Value>,
    /// Total count of matching documents (only populated if include_total was true)
    pub total: Option<usize>,
}

/// Update result (MongoDB-compatible)
#[derive(Debug)]
pub struct UpdateResult {
    pub matched_count: u64,
    pub modified_count: u64,
    /// ID of the upserted document (if upsert occurred)
    pub upserted_id: Option<String>,
}

/// Full-text search options
#[derive(Debug, Default)]
pub struct FulltextSearchOptions {
    pub limit: Option<usize>,
    pub skip: Option<usize>,
    pub min_score: Option<f64>,
    pub projection: Option<HashMap<String, i32>>,
    /// MongoDB-style filter applied AFTER TF-IDF scoring (core-level filtering)
    pub filter: Option<Value>,
    /// AND mode: ALL query tokens must match (default: false = OR mode)
    pub and_mode: bool,
    /// Enable highlight/snippet generation
    pub highlight: bool,
    /// Characters of context around each match (default: 100)
    pub highlight_context: Option<usize>,
    /// Maximum snippets per field (default: 3)
    pub highlight_max_snippets: Option<usize>,
    /// Pre-filter: only return chunks belonging to these parent doc_ids.
    /// Uses chunk_doc_mapping for O(1) lookup per posting entry.
    pub target_doc_ids: Option<std::collections::HashSet<String>>,
}

/// Result of document-level AND qualification for group_by_document.
#[derive(Debug)]
pub enum QualificationResult {
    /// Single token: no intersection needed, skip qualification.
    NotRequired,
    /// Documents qualified via chunk_doc_mapping (fast path).
    Qualified(Vec<String>),
    /// Legacy index without chunk_doc_mapping — caller should use find-based fallback.
    LegacyFallback,
}

/// Full-text search result with optional highlights
#[derive(Debug)]
pub struct FulltextSearchResult {
    pub document: Value,
    pub score: f64,
    pub matched_tokens: Vec<String>,
    pub highlights: Option<Vec<HighlightResultJson>>,
}

/// Highlight result for JSON serialization
#[derive(Debug, serde::Serialize)]
pub struct HighlightResultJson {
    pub field: String,
    pub snippets: Vec<String>,
}

/// Options for extended fuzzy search (adapter-level)
#[derive(Debug, Default)]
pub struct FuzzySearchOptions {
    /// Algorithm to use (default: from index metadata)
    pub algorithm: Option<ironbase_core::FuzzyAlgorithm>,
    /// Minimum similarity threshold (0.0-1.0, default: from index metadata)
    pub threshold: Option<f64>,
    /// Maximum results to return (default: 10)
    pub limit: Option<usize>,
    /// Results to skip for pagination
    pub skip: Option<usize>,
    /// Field projection (include/exclude): {"field": 1} or {"field": 0}
    pub projection: Option<HashMap<String, i32>>,
    /// MongoDB-style post-filter applied to fuzzy results
    pub filter: Option<Value>,
    /// Enable highlight of matched value (default: false)
    pub highlight: bool,
}

/// Result from extended fuzzy search (adapter-level)
#[derive(Debug)]
pub struct FuzzySearchResult {
    /// The matched document (with projection applied if specified)
    pub document: Value,
    /// Similarity score (0.0-1.0)
    pub score: f64,
    /// The original value that matched the query
    pub matched_value: String,
    /// Optional highlight showing the matched value with <mark> tags
    pub highlight: Option<String>,
}

/// IronBase Adapter
pub struct IronBaseAdapter {
    db: Arc<RwLock<DatabaseCore<StorageEngine>>>,
    /// Database file path (stored for stats, wrapped in RwLock for dynamic switching)
    db_path: RwLock<std::path::PathBuf>,
    /// In-memory document counts for fast stats() without lock contention
    collection_stats: CollectionStats,
    /// Singleton guard: true while a background compact is running
    compacting: std::sync::atomic::AtomicBool,
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
            compacting: std::sync::atomic::AtomicBool::new(false),
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

                // Set schema - CRITICAL: Schema enforcement is required for security
                let coll = db.collection(collection_name).map_err(|e| {
                    McpError::internal(format!(
                        "Failed to access system collection '{}': {}",
                        collection_name, e
                    ))
                })?;
                coll.set_schema(Some(schema.clone())).map_err(|e| {
                    McpError::internal(format!(
                        "CRITICAL: Failed to set schema for system collection '{}': {}. \
                         System integrity requires valid schemas.",
                        collection_name, e
                    ))
                })?;
            } else {
                // Use write lock directly to avoid TOCTOU race condition
                // (checking with read lock, then acting with write lock could allow
                // state changes between the check and the action)
                let db = self.db.write();

                // Check and set flags if needed
                let needs_flags = db
                    .get_collection_flags(collection_name)
                    .map(|f| !f.hidden || !f.protected || !f.is_system)
                    .unwrap_or(true);

                if needs_flags {
                    db.set_collection_flags(collection_name, system_flags)?;
                }

                // Check and set schema if needed
                let needs_schema = db
                    .get_collection(collection_name)
                    .ok()
                    .and_then(|c| c.get_schema().ok().flatten())
                    .is_none();

                if needs_schema {
                    let coll = db.collection(collection_name).map_err(|e| {
                        McpError::internal(format!(
                            "Failed to access system collection '{}': {}",
                            collection_name, e
                        ))
                    })?;
                    coll.set_schema(Some(schema.clone())).map_err(|e| {
                        McpError::internal(format!(
                            "CRITICAL: Failed to set schema for system collection '{}': {}. \
                             System integrity requires valid schemas.",
                            collection_name, e
                        ))
                    })?;
                }
            }
        }

        Ok(())
    }

    // ============================================================
    // Execution Context (for cancellation/timeout)
    // ============================================================

    /// Create an ExecutionContext from current thread-local execution context.
    ///
    /// This consolidates cancellation/timeout support across all operations.
    /// The returned context should be passed to core `_with_ctx` methods.
    fn create_execution_context(&self) -> ExecutionContext {
        let mut ctx = ExecutionContext::new();

        // Add deadline and cancel flag from unified thread-local context
        if let Some(exec_ctx) = execution::current_execution_context() {
            if let Some(deadline) = exec_ctx.deadline {
                ctx = ctx.with_deadline(deadline);
            }
            if let Some(flag) = exec_ctx.cancel_flag {
                ctx = ctx.with_cancel_flag(flag);
            }
        }

        ctx
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

        // Clear stale stats from previous warm_up or deleted collections
        // This ensures we don't report counts for collections that no longer exist
        self.collection_stats.clear();

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
            // Log BEFORE starting - critical for diagnosing hangs/crashes
            tracing::debug!("Warming up [{}/{}] '{}'...", i + 1, total, name);

            let coll_start = std::time::Instant::now();
            let db = self.db.read();
            match db.collection(name) {
                Ok(coll) => {
                    // Phase 1: Count documents (can be slow on large collections)
                    tracing::debug!("  [{}/{}] '{}': counting documents...", i + 1, total, name);
                    let count_start = std::time::Instant::now();
                    let count = coll.count_documents(&serde_json::json!({})).unwrap_or(0);
                    let count_elapsed = count_start.elapsed();
                    if count_elapsed.as_millis() > 500 {
                        tracing::info!(
                            "  [{}/{}] '{}': counted {} docs in {:.2}s",
                            i + 1,
                            total,
                            name,
                            count,
                            count_elapsed.as_secs_f64()
                        );
                    }
                    self.collection_stats.set(name, count);

                    // Phase 2: Refresh index statistics (can be slow with many indexes)
                    tracing::debug!(
                        "  [{}/{}] '{}': refreshing index stats...",
                        i + 1,
                        total,
                        name
                    );
                    let index_start = std::time::Instant::now();
                    if let Err(e) = coll.refresh_index_stats() {
                        tracing::warn!("Failed to refresh index stats for '{}': {}", name, e);
                    }
                    let index_elapsed = index_start.elapsed();
                    if index_elapsed.as_millis() > 500 {
                        tracing::info!(
                            "  [{}/{}] '{}': index stats refreshed in {:.2}s",
                            i + 1,
                            total,
                            name,
                            index_elapsed.as_secs_f64()
                        );
                    }

                    // Phase 3: Load vector indexes (can be slow for large HNSW indexes)
                    tracing::debug!(
                        "  [{}/{}] '{}': loading vector indexes...",
                        i + 1,
                        total,
                        name
                    );
                    let vector_start = std::time::Instant::now();
                    match coll.ensure_vector_indexes_loaded() {
                        Ok(loaded) if loaded > 0 => {
                            let vector_elapsed = vector_start.elapsed();
                            tracing::info!(
                                "  [{}/{}] '{}': loaded {} vector index(es) in {:.2}s",
                                i + 1,
                                total,
                                name,
                                loaded,
                                vector_elapsed.as_secs_f64()
                            );
                        }
                        Err(e) => {
                            tracing::warn!("Failed to load vector indexes for '{}': {}", name, e);
                        }
                        _ => {}
                    }

                    // Summary for this collection
                    let elapsed = coll_start.elapsed();
                    if elapsed.as_millis() > 100 {
                        // Only log slow collections at info level
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

    /// Check if a collection exists
    ///
    /// Used by permission checks to prevent implicit collection creation
    /// by non-admin clients through insert operations.
    pub fn collection_exists(&self, name: &str) -> bool {
        let db = self.db.read();
        db.list_collections().contains(&name.to_string())
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

                let btree_indexes = db
                    .collection(&name)
                    .ok()
                    .and_then(|c| match c.list_indexes() {
                        Ok(indexes) => Some(indexes),
                        Err(e) => {
                            tracing::warn!(
                                "Failed to list B-tree indexes for collection '{}': {}",
                                name, e
                            );
                            None
                        }
                    })
                    .unwrap_or_default();

                let vector_index_list = db
                    .collection(&name)
                    .ok()
                    .and_then(|c| match c.list_vector_indexes() {
                        Ok(indexes) => Some(indexes),
                        Err(e) => {
                            tracing::warn!(
                                "Failed to list vector indexes for collection '{}': {}",
                                name, e
                            );
                            None
                        }
                    })
                    .unwrap_or_default();

                let vector_count: usize = vector_index_list.iter().map(|idx| idx.vector_count).sum();

                // Convert VectorIndexMetadata to JSON for the response
                let vector_indexes: Vec<Value> = vector_index_list
                    .iter()
                    .map(|idx| {
                        serde_json::json!({
                            "name": idx.name,
                            "field": idx.field,
                            "dim": idx.config.dim,
                            "metric": format!("{:?}", idx.config.metric).to_lowercase(),
                            "vector_count": idx.vector_count,
                        })
                    })
                    .collect();

                serde_json::json!({
                    "name": name,
                    "document_count": doc_count,
                    "btree_index_count": btree_indexes.len(),
                    "btree_indexes": btree_indexes,
                    "vector_index_count": vector_indexes.len(),
                    "vector_indexes": vector_indexes,
                    "total_vectors": vector_count,
                })
            })
            .collect();

        drop(db); // Release db lock as soon as possible

        // Calculate totals (no lock needed - just iterating local Vec)
        let total_documents: u64 = collections
            .iter()
            .filter_map(|c| c.get("document_count").and_then(|v| v.as_u64()))
            .sum();

        let total_btree_indexes: usize = collections
            .iter()
            .filter_map(|c| c.get("btree_index_count").and_then(|v| v.as_u64()))
            .map(|v| v as usize)
            .sum();

        let total_vector_indexes: usize = collections
            .iter()
            .filter_map(|c| c.get("vector_index_count").and_then(|v| v.as_u64()))
            .map(|v| v as usize)
            .sum();

        let total_vectors: usize = collections
            .iter()
            .filter_map(|c| c.get("total_vectors").and_then(|v| v.as_u64()))
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
                "total_btree_indexes": total_btree_indexes,
                "total_vector_indexes": total_vector_indexes,
                "total_vectors": total_vectors,
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
            return Err(McpError::invalid_params(format!(
                "Database file does not exist: {}",
                new_path
            )));
        }

        if create_if_missing && path.exists() {
            return Err(McpError::invalid_params(format!(
                "Database file already exists: {}",
                new_path
            )));
        }

        // Create parent directory if needed
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    McpError::internal(format!("Failed to create directory: {}", e))
                })?;
            }
        }

        // Open new database (creates if needed) - still under write lock
        let new_db = DatabaseCore::open(path)
            .map_err(|e| McpError::internal(format!("Failed to open database: {}", e)))?;

        // Clear old stats BEFORE swapping database (prevents race condition where
        // concurrent requests see new DB but old stats)
        self.collection_stats.clear();

        // Swap the database (already holding write lock)
        *db_guard = new_db;
        drop(db_guard); // Explicitly release db lock

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

    /// Try to set the compacting flag (singleton guard).
    /// Returns true if we successfully set it (no other compact running).
    pub fn try_start_compact(&self) -> bool {
        self.compacting
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::SeqCst,
                std::sync::atomic::Ordering::SeqCst,
            )
            .is_ok()
    }

    /// Clear the compacting flag (MUST be called after compact finishes/fails/cancels)
    pub fn clear_compact_flag(&self) {
        self.compacting
            .store(false, std::sync::atomic::Ordering::SeqCst);
    }

    /// Check if a background compact is currently running
    pub fn is_compacting(&self) -> bool {
        self.compacting.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Non-blocking compact — delegates to DatabaseCore::compact_nonblocking()
    ///
    /// Uses db.read() (NOT write!) because compact_nonblocking internally
    /// manages storage lock acquisition per-phase.
    pub fn compact_nonblocking(
        &self,
        config: &ironbase_core::storage::CompactionConfig,
        progress_callback: &dyn Fn(u64, u64),
    ) -> std::result::Result<ironbase_core::storage::CompactionStats, ironbase_core::IronBaseError>
    {
        let db = self.db.read();
        db.compact_nonblocking(config, progress_callback)
    }

    /// Force checkpoint (flush indexes + metadata + clear WAL)
    ///
    /// This is the MongoDB-style checkpoint that persists indexes to disk.
    /// Returns checkpoint statistics including indexes flushed and WAL size.
    pub fn checkpoint(&self) -> Result<Value> {
        let start = std::time::Instant::now();

        // TWO-PHASE CHECKPOINT:
        //
        // Phase 1: Flush indexes to disk (slow: writes .idx/.ftidx/.fzidx/.hnsw files)
        // Uses db.read() so concurrent inserts/finds are NOT blocked.
        let indexes_flushed = {
            let db = self.db.read();
            db.flush_all_indexes_counted()
                .map_err(|e| crate::error::McpError::storage(e.to_string()))?
        }; // db.read() released here

        // Phase 2: Flush metadata + clear WAL
        // Uses db.read() — the internal storage.write() provides sufficient
        // ordering (waits for in-flight insert_one_persist to release storage lock).
        // Previously used db.write() which caused priority inversion: parking_lot's
        // write-preferring behavior blocked ALL new db.read() callers (insert, find)
        // while flush_metadata serialized 130K doc catalog (~6MB) under memory pressure.
        let result = {
            let db = self.db.read();
            db.checkpoint_wal_only()
                .map_err(|e| crate::error::McpError::storage(e.to_string()))?
        }; // db.read() released here

        let duration_ms = start.elapsed().as_millis() as u64;

        Ok(serde_json::json!({
            "success": true,
            "indexes_flushed": indexes_flushed,
            "wal_size_before": format_bytes(result.wal_size_before),
            "wal_size_after": format_bytes(result.wal_size_after),
            "wal_size_before_bytes": result.wal_size_before,
            "wal_size_after_bytes": result.wal_size_after,
            "wal_ops_cleared": result.wal_ops_cleared,
            "duration_ms": duration_ms,
        }))
    }

    /// Periodic checkpoint (flush btree indexes + metadata + clear WAL)
    ///
    /// Skips fulltext/fuzzy/vector index flush to avoid blocking insert_one.
    /// Fulltext flush holds index_manager.write() for 13+ seconds (130K docs),
    /// which blocks insert_one's check_index_constraints (needs indexes.read()).
    /// Btree indexes flush in < 300ms each — acceptable.
    ///
    /// Fulltext/fuzzy indexes are flushed during close() and compact() only.
    /// On dirty shutdown they are rebuilt from documents (safe, slower startup).
    pub fn checkpoint_periodic(&self) -> Result<Value> {
        let start = std::time::Instant::now();

        // Phase 1: Flush only btree indexes (fast, < 300ms per index)
        // Fulltext/fuzzy skipped — they block insert_one for minutes under memory pressure
        let indexes_flushed = {
            let db = self.db.read();
            db.flush_btree_indexes_counted()
                .map_err(|e| crate::error::McpError::storage(e.to_string()))?
        };

        // Phase 2: Flush metadata + clear WAL (pre-serialize outside lock)
        let result = {
            let db = self.db.read();
            db.checkpoint_wal_only()
                .map_err(|e| crate::error::McpError::storage(e.to_string()))?
        };

        let duration_ms = start.elapsed().as_millis() as u64;

        Ok(serde_json::json!({
            "success": true,
            "indexes_flushed": indexes_flushed,
            "wal_size_before": format_bytes(result.wal_size_before),
            "wal_size_after": format_bytes(result.wal_size_after),
            "wal_size_before_bytes": result.wal_size_before,
            "wal_size_after_bytes": result.wal_size_after,
            "wal_ops_cleared": result.wal_ops_cleared,
            "duration_ms": duration_ms,
        }))
    }

    /// Compute storage wastage statistics for auto-compaction decisions.
    ///
    /// Delegates to DatabaseCore::storage_wastage(). O(C) cost.
    pub fn compute_wastage(&self) -> ironbase_core::storage::StorageWastage {
        let db = self.db.read();
        db.storage_wastage()
    }

    /// Store the file size after the last successful compaction.
    ///
    /// Called after compact finishes to calibrate bloat_ratio.
    pub fn set_last_compact_size(&self, size: u64) {
        let db = self.db.read();
        db.set_last_compact_size(size);
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
            .map_err(|e| crate::error::McpError::storage(e.to_string()))?;
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

    /// Find documents
    pub fn find(&self, collection: &str, query: Value, options: FindOptions) -> Result<FindResult> {
        let db = self.db.read();
        let coll = db.get_collection(collection)?;

        // Convert to IronBase FindOptions
        let projection = if let Some(proj) = options.projection.as_ref() {
            let obj = proj.as_object().ok_or_else(|| {
                crate::error::McpError::invalid_params(
                    "projection must be an object like {\"field\": 1} or {\"field\": 0}",
                )
            })?;
            let mut map = HashMap::new();
            for (k, v) in obj {
                let val = if let Some(i) = v.as_i64() {
                    if i != 0 && i != 1 {
                        return Err(crate::error::McpError::invalid_params(format!(
                            "Invalid projection value for '{}': expected 0 or 1, got {}",
                            k, i
                        )));
                    }
                    i as i32
                } else if let Some(f) = v.as_f64() {
                    // Use epsilon-based comparison for floating point values
                    // to handle potential precision issues in JSON parsing
                    if f.abs() < f64::EPSILON {
                        0
                    } else if (f - 1.0).abs() < f64::EPSILON {
                        1
                    } else {
                        return Err(crate::error::McpError::invalid_params(format!(
                            "Invalid projection value for '{}': expected 0 or 1, got {}",
                            k, f
                        )));
                    }
                } else {
                    return Err(crate::error::McpError::invalid_params(format!(
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

        // Create ExecutionContext for timeout/cancellation support
        let ctx = self.create_execution_context();

        let ironbase_options = ironbase_core::FindOptions {
            projection,
            sort: options.sort,
            limit: options.limit,
            skip: options.skip,
            include_total: options.include_total,
            max_response_bytes: options.max_response_bytes,
            cancel_flag: ctx.cancel_flag().cloned(),
            deadline: ctx.deadline(),
        };

        let result = coll.find_with_result(&query, ironbase_options)?;
        Ok(FindResult {
            documents: result.documents,
            total: result.total.map(|t| t as usize),
        })
    }

    /// Find a single document (with cancellation/timeout support)
    pub fn find_one(&self, collection: &str, query: Value) -> Result<Option<Value>> {
        let db = self.db.read();
        let coll = db.get_collection(collection)?;
        let ctx = self.create_execution_context();
        let result = coll.find_one_with_ctx(&query, Some(&ctx))?;
        Ok(result)
    }

    /// Find a single document with projection support (projection applied in core)
    pub fn find_one_with_options(
        &self,
        collection: &str,
        query: Value,
        options: ironbase_core::find_options::FindOptions,
    ) -> Result<Option<Value>> {
        let db = self.db.read();
        let coll = db.get_collection(collection)?;
        let result = coll.find_one_with_options(&query, options)?;
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
            upserted_id: None,
        })
    }

    /// Update a single document with upsert support (MongoDB-compatible)
    ///
    /// If `upsert` is true and no document matches the filter,
    /// a new document is created from the filter criteria and update.
    ///
    /// # Arguments
    /// * `collection` - Target collection
    /// * `filter` - Filter to find document
    /// * `update` - Update operators to apply
    /// * `upsert` - If true, insert when no match found
    ///
    /// # Returns
    /// `UpdateResult` with matched/modified counts and optional upserted_id
    pub fn update_one_with_options(
        &self,
        collection: &str,
        filter: Value,
        update: Value,
        upsert: bool,
    ) -> Result<UpdateResult> {
        let db = self.db.read();
        let options = ironbase_core::UpdateOptions::new().with_upsert(upsert);
        let result = db.update_one_with_options(collection, &filter, &update, options)?;

        // If upsert occurred, update in-memory count
        if result.upserted_id.is_some() {
            self.collection_stats.increment(collection, 1);
        }

        Ok(UpdateResult {
            matched_count: result.matched_count,
            modified_count: result.modified_count,
            upserted_id: result.upserted_id.map(|id| Self::doc_id_to_string(&id)),
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
            upserted_id: None,
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

    /// Count documents matching query (with cancellation/timeout support)
    pub fn count_documents(&self, collection: &str, query: Value) -> Result<u64> {
        let db = self.db.read();
        let coll = db.get_collection(collection)?;
        let ctx = self.create_execution_context();
        let count = coll.count_documents_with_ctx(&query, Some(&ctx))?;
        Ok(count)
    }

    /// Get distinct values for a field (with cancellation/timeout support)
    pub fn distinct(&self, collection: &str, field: &str, query: Value) -> Result<Vec<Value>> {
        let db = self.db.read();
        let coll = db.get_collection(collection)?;
        let ctx = self.create_execution_context();
        let values = coll.distinct_with_ctx(field, &query, Some(&ctx))?;
        Ok(values)
    }

    // ============================================================
    // Aggregation
    // ============================================================

    /// Execute aggregation pipeline
    ///
    /// Uses `aggregate_auto()` which automatically scales memory limits based on
    /// available system RAM, preventing OOM on resource-constrained servers.
    pub fn aggregate(&self, collection: &str, pipeline: Vec<Value>) -> Result<Vec<Value>> {
        let db = self.db.read();
        let coll = db.get_collection(collection)?;
        // Convert Vec<Value> to Value::Array
        let pipeline_value = Value::Array(pipeline.clone());
        // Use dynamic limits based on available system RAM (OOM protection)
        let limits = AggregationLimits::from_system_memory();
        let mut ctx = AggregationLimitContext::new(limits);
        if let Some(deadline) = execution::current_deadline() {
            ctx = ctx.with_deadline(deadline);
        }
        let results = coll.aggregate_with_context(&pipeline_value, &ctx)?;
        Ok(results)
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

    /// Refresh index statistics for a collection
    ///
    /// Scans all B+ tree indexes and computes distinct_count and null_count.
    /// The query planner uses these statistics to choose the best index.
    pub fn refresh_index_stats(&self, collection: &str) -> Result<()> {
        let db = self.db.read();
        let coll = db.get_collection(collection)?;
        coll.refresh_index_stats()?;
        Ok(())
    }

    /// Get detailed index statistics for a collection
    ///
    /// Returns information about each index including:
    /// - num_keys: Total keys in the index
    /// - distinct_count: Number of unique values
    /// - has_histogram: Whether histogram data is available
    /// - has_mcv: Whether MCV (Most Common Values) data is available
    pub fn get_index_statistics(&self, collection: &str) -> Result<Vec<Value>> {
        use serde_json::json;

        let db = self.db.read();
        let coll = db.get_collection(collection)?;
        let stats = coll.get_index_statistics();

        let result: Vec<Value> = stats
            .into_iter()
            .map(|s| {
                json!({
                    "name": s.name,
                    "field": s.field,
                    "num_keys": s.num_keys,
                    "distinct_count": s.distinct_count,
                    "has_histogram": s.has_histogram,
                    "has_mcv": s.has_mcv
                })
            })
            .collect();

        Ok(result)
    }

    /// Explain query execution plan (uses get_collection - no implicit creation)
    pub fn explain(&self, collection: &str, query: Value) -> Result<Value> {
        let db = self.db.read();
        let coll = db.get_collection(collection)?;
        let plan = coll.explain(&query)?;
        Ok(plan)
    }

    /// Find documents with index hint and full options support
    ///
    /// Extended version that supports sort, skip, limit, projection, and OOM protection.
    /// All post-processing is done in core (not in the tools layer).
    pub fn find_with_hint_ext(
        &self,
        collection: &str,
        query: Value,
        hint: &str,
        options: ironbase_core::find_options::FindOptions,
    ) -> Result<Vec<Value>> {
        let db = self.db.read();
        let coll = db.get_collection(collection)?;
        let documents = coll.find_with_hint_ext(&query, hint, options)?;
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

    /// Fuzzy search using the fuzzy index (with cancellation/timeout support)
    pub fn fuzzy_search(
        &self,
        collection: &str,
        field: &str,
        query_str: &str,
        threshold: Option<f64>,
        algorithm: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Vec<(Value, f64)>> {
        use ironbase_core::FuzzyAlgorithm;

        // Default to JaroWinkler if no algorithm specified
        let algo = Some(match algorithm.unwrap_or("jaro_winkler") {
            "levenshtein" => FuzzyAlgorithm::Levenshtein,
            "damerau_levenshtein" => FuzzyAlgorithm::DamerauLevenshtein,
            _ => FuzzyAlgorithm::JaroWinkler,
        });

        let db = self.db.read();
        let coll = db.get_collection(collection)?;
        let ctx = self.create_execution_context();
        let results =
            coll.fuzzy_search_with_ctx(field, query_str, threshold, algo, limit, Some(&ctx))?;
        Ok(results)
    }

    /// Extended fuzzy search with filter, projection, highlight support (CORE LEVEL)
    ///
    /// All logic (similarity matching, filter, projection) is handled in core via `fuzzy_search_ext()`.
    /// This adapter method simply converts options and returns results.
    pub fn fuzzy_search_ext(
        &self,
        collection: &str,
        field: &str,
        query_str: &str,
        options: FuzzySearchOptions,
    ) -> Result<Vec<FuzzySearchResult>> {
        use ironbase_core::FuzzySearchOptions as CoreOptions;

        let db = self.db.read();
        let coll = db.get_collection(collection)?;
        let ctx = self.create_execution_context();

        // Convert adapter options to core options
        let core_options = CoreOptions {
            algorithm: options.algorithm,
            threshold: options.threshold,
            limit: options.limit,
            skip: options.skip,
            projection: options.projection,
            filter: options.filter,
            highlight: options.highlight,
            cancel_flag: ctx.cancel_flag().cloned(),
            deadline: ctx.deadline(),
        };

        // Execute search in core
        let core_results = coll.fuzzy_search_ext(field, query_str, core_options)?;

        // Convert core results to adapter results
        let results = core_results
            .into_iter()
            .map(|r| FuzzySearchResult {
                document: r.document,
                score: r.score,
                matched_value: r.matched_value,
                highlight: r.highlight,
            })
            .collect();

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

    /// Full-text search using the fulltext index (with cancellation/timeout support)
    /// Returns results with optional highlights/snippets
    ///
    /// All logic (TF-IDF, filter, highlight) is now handled in core via `fulltext_search_ext()`.
    /// This adapter method simply converts options and returns results.
    pub fn fulltext_search(
        &self,
        collection: &str,
        field: &str,
        query_str: &str,
        options: FulltextSearchOptions,
    ) -> Result<Vec<FulltextSearchResult>> {
        use ironbase_core::fulltext::{FulltextSearchOptions as CoreOptions, HighlightOptions};

        let db = self.db.read();
        let coll = db.get_collection(collection)?;
        let ctx = self.create_execution_context();

        // Convert adapter options to core options
        let core_options = CoreOptions {
            limit: options.limit,
            skip: options.skip,
            min_score: options.min_score,
            projection: options.projection,
            filter: options.filter,
            and_mode: options.and_mode,
            highlight: options.highlight,
            highlight_options: if options.highlight {
                Some(HighlightOptions {
                    context_chars: options.highlight_context.unwrap_or(100),
                    max_snippets: options.highlight_max_snippets.unwrap_or(3),
                    ..Default::default()
                })
            } else {
                None
            },
            cancel_flag: ctx.cancel_flag().cloned(),
            deadline: ctx.deadline(),
            target_doc_ids: options.target_doc_ids,
        };

        // Call the unified core API - all logic (filter, highlight) handled there
        let results = coll.fulltext_search_ext(field, query_str, core_options)?;

        // Convert core results to adapter results
        let results = results
            .into_iter()
            .map(|r| FulltextSearchResult {
                document: r.document,
                score: r.score,
                matched_tokens: r.matched_tokens,
                highlights: r.highlights.map(|hs| {
                    hs.into_iter()
                        .map(|h| HighlightResultJson {
                            field: h.field,
                            snippets: h.snippets,
                        })
                        .collect()
                }),
            })
            .collect();

        Ok(results)
    }

    /// Multi-field fulltext search
    ///
    /// Searches across multiple fulltext-indexed fields and merges results by document.
    /// Each document's final score is the maximum score across all matching fields.
    pub fn fulltext_search_multi(
        &self,
        collection: &str,
        fields: &[&str],
        query_str: &str,
        options: FulltextSearchOptions,
    ) -> Result<Vec<FulltextSearchResult>> {
        use ironbase_core::fulltext::{FulltextSearchOptions as CoreOptions, HighlightOptions};

        let db = self.db.read();
        let coll = db.get_collection(collection)?;
        let ctx = self.create_execution_context();

        let core_options = CoreOptions {
            limit: options.limit,
            skip: options.skip,
            min_score: options.min_score,
            projection: options.projection,
            filter: options.filter,
            and_mode: options.and_mode,
            highlight: options.highlight,
            highlight_options: if options.highlight {
                Some(HighlightOptions {
                    context_chars: options.highlight_context.unwrap_or(100),
                    max_snippets: options.highlight_max_snippets.unwrap_or(3),
                    ..Default::default()
                })
            } else {
                None
            },
            cancel_flag: ctx.cancel_flag().cloned(),
            deadline: ctx.deadline(),
            target_doc_ids: options.target_doc_ids,
        };

        let results = coll.fulltext_search_multi_ext(fields, query_str, core_options)?;

        let results = results
            .into_iter()
            .map(|r| FulltextSearchResult {
                document: r.document,
                score: r.score,
                matched_tokens: r.matched_tokens,
                highlights: r.highlights.map(|hs| {
                    hs.into_iter()
                        .map(|h| HighlightResultJson {
                            field: h.field,
                            snippets: h.snippets,
                        })
                        .collect()
                }),
            })
            .collect();

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

    /// Get field names that have fulltext indexes (lightweight, no metadata computation)
    ///
    /// Unlike `list_fulltext_indexes()`, avoids `unique_token_count()` in lazy mode.
    /// Uses get_collection - no implicit creation
    pub fn get_fulltext_field_names(&self, collection: &str) -> Result<Vec<String>> {
        let db = self.db.read();
        let coll = db.get_collection(collection)?;
        Ok(coll.fulltext_indexed_fields()?)
    }

    /// Tokenize query using the fulltext index's config for a given field.
    /// Returns unique stemmed tokens — same processing as fulltext search uses.
    pub fn fulltext_tokenize_query(
        &self,
        collection: &str,
        field: &str,
        query: &str,
    ) -> Result<Vec<String>> {
        let db = self.db.read();
        let coll = db.get_collection(collection)?;
        Ok(coll.fulltext_tokenize_query(field, query)?)
    }

    /// Get the stored tokenization options (language, accent_folding, min_word_length)
    /// of an existing fulltext index for a field.
    pub fn get_fulltext_index_options(
        &self,
        collection: &str,
        field: &str,
    ) -> Result<ironbase_core::fulltext::FtsOptions> {
        let db = self.db.read();
        let coll = db.get_collection(collection)?;
        Ok(coll.get_fulltext_index_options(field)?)
    }

    /// Get posting list sizes for each token (for rarity-based ordering).
    pub fn fulltext_token_posting_counts(
        &self,
        collection: &str,
        field: &str,
        tokens: &[String],
    ) -> Result<Vec<(String, usize)>> {
        let db = self.db.read();
        let coll = db.get_collection(collection)?;
        Ok(coll.fulltext_token_posting_counts(field, tokens)?)
    }

    /// Get all chunk _ids from posting list for a stemmed token.
    /// Returns DocumentIds as serde_json::Value for use in $in filters.
    pub fn fulltext_token_chunk_ids(
        &self,
        collection: &str,
        field: &str,
        token: &str,
    ) -> Result<Vec<Value>> {
        let db = self.db.read();
        let coll = db.get_collection(collection)?;
        let ids = coll.fulltext_token_chunk_ids(field, token)?;
        Ok(ids
            .into_iter()
            .filter_map(|id| serde_json::to_value(id).ok())
            .collect())
    }

    /// Resolve chunk _ids to parent doc_id values using the fulltext index's in-memory mapping.
    /// Returns chunk_id (as DocumentId) → doc_id (String) for all mapped chunks.
    pub fn fulltext_resolve_chunk_doc_ids(
        &self,
        collection: &str,
        field: &str,
        chunk_ids: &[ironbase_core::document::DocumentId],
    ) -> Result<std::collections::HashMap<ironbase_core::document::DocumentId, String>> {
        let db = self.db.read();
        let coll = db.get_collection(collection)?;
        Ok(coll.fulltext_resolve_chunk_doc_ids(field, chunk_ids)?)
    }

    /// Check if the fulltext index has a chunk→doc_id mapping populated.
    pub fn fulltext_has_chunk_doc_mapping(&self, collection: &str, field: &str) -> Result<bool> {
        let db = self.db.read();
        let coll = db.get_collection(collection)?;
        Ok(coll.fulltext_has_chunk_doc_mapping(field)?)
    }

    /// Check if the fulltext index tracks a parent doc_id field (RAG-style).
    /// `false` → non-RAG collection: the chunk `_id` is the document identity.
    pub fn fulltext_has_parent_doc_id_field(&self, collection: &str, field: &str) -> Result<bool> {
        let db = self.db.read();
        let coll = db.get_collection(collection)?;
        Ok(coll.fulltext_has_parent_doc_id_field(field)?)
    }

    /// Qualify documents for document-level AND mode using in-memory chunk→doc_id mapping.
    ///
    /// For each token, gets the posting list chunk_ids from the fulltext index,
    /// resolves them to doc_ids via the chunk_doc_mapping, then intersects.
    /// Returns the set of doc_ids that contain ALL query tokens (across any chunks).
    pub fn fulltext_qualify_documents_fast(
        &self,
        collection: &str,
        field: &str,
        query: &str,
    ) -> Result<QualificationResult> {
        let db = self.db.read();
        let coll = db.get_collection(collection)?;

        // Tokenize
        let tokens = coll.fulltext_tokenize_query(field, query)?;
        if tokens.len() <= 1 {
            return Ok(QualificationResult::NotRequired);
        }

        // Check if mapping is available
        if !coll.fulltext_has_chunk_doc_mapping(field)? {
            return Ok(QualificationResult::LegacyFallback);
        }

        // Get posting counts for rarity ordering
        let mut token_counts = coll.fulltext_token_posting_counts(field, &tokens)?;
        token_counts.sort_by_key(|(_, count)| *count);

        if token_counts[0].1 == 0 {
            return Ok(QualificationResult::Qualified(Vec::new()));
        }

        // Intersect at doc_id level using chunk_doc_mapping
        let mut qualified: Option<std::collections::HashSet<String>> = None;

        for (token, _) in &token_counts {
            let chunk_ids = coll.fulltext_token_chunk_ids(field, token)?;
            let chunk_doc_ids = coll.fulltext_resolve_chunk_doc_ids(field, &chunk_ids)?;

            let doc_ids: std::collections::HashSet<String> = chunk_doc_ids.into_values().collect();

            qualified = Some(match qualified {
                None => doc_ids,
                Some(prev) => prev.intersection(&doc_ids).cloned().collect(),
            });

            if qualified.as_ref().is_some_and(|q| q.is_empty()) {
                return Ok(QualificationResult::Qualified(Vec::new()));
            }
        }

        Ok(QualificationResult::Qualified(
            qualified
                .map(|q| q.into_iter().collect())
                .unwrap_or_default(),
        ))
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
    // Auto-Embedding Configuration
    // ============================================================

    /// Set auto-embedding configuration for a collection
    ///
    /// When enabled, documents inserted or updated will automatically have
    /// embeddings generated from the source field and stored in the target field.
    pub fn set_auto_embedding_config(
        &self,
        collection: &str,
        config: Option<ironbase_core::storage::AutoEmbeddingConfig>,
    ) -> Result<()> {
        let db = self.db.read();
        let coll = db.collection(collection)?;
        coll.set_auto_embedding_config(config)?;
        Ok(())
    }

    /// Get auto-embedding configuration for a collection
    pub fn get_auto_embedding_config(
        &self,
        collection: &str,
    ) -> Result<Option<ironbase_core::storage::AutoEmbeddingConfig>> {
        let db = self.db.read();
        let coll = db.get_collection(collection)?;
        Ok(coll.get_auto_embedding_config()?)
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
            upserted_id: None,
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

    // =========================================================================
    // Vector Index Operations
    // =========================================================================

    /// Create a vector index for similarity search
    ///
    /// # Arguments
    /// * `collection` - Collection name
    /// * `field` - Field containing embedding vectors (array of numbers)
    /// * `dim` - Vector dimension (must match embedding size)
    /// * `metric` - Distance metric: "cosine", "euclidean", or "dot_product"
    /// * `max_vectors` - Maximum number of vectors to index
    /// * `m` - HNSW M parameter (connections per node)
    /// * `ef_construction` - HNSW build-time quality parameter
    /// * `ef_search` - HNSW search-time quality parameter
    #[allow(clippy::too_many_arguments)]
    pub fn create_vector_index(
        &self,
        collection: &str,
        field: &str,
        dim: usize,
        metric: &str,
        max_vectors: usize,
        m: usize,
        ef_construction: usize,
        ef_search: usize,
    ) -> Result<String> {
        use ironbase_core::vector::{DistanceMetric, VectorIndexConfig};

        // Parse metric
        let distance_metric = match metric.to_lowercase().as_str() {
            "cosine" => DistanceMetric::Cosine,
            "euclidean" => DistanceMetric::Euclidean,
            "dot_product" | "dotproduct" => DistanceMetric::DotProduct,
            _ => {
                return Err(crate::error::McpError::invalid_params(format!(
                    "Unknown distance metric '{}'. Use: cosine, euclidean, or dot_product",
                    metric
                )))
            }
        };

        let config = VectorIndexConfig::new(dim)
            .with_metric(distance_metric)
            .with_max_vectors(max_vectors)
            .with_m(m)
            .with_ef_construction(ef_construction)
            .with_ef_search(ef_search);

        let db = self.db.read();
        let coll = db.collection(collection)?;
        let name = coll.create_vector_index(field, config)?;
        Ok(name)
    }

    /// List all vector indexes on a collection
    pub fn list_vector_indexes(&self, collection: &str) -> Result<Vec<Value>> {
        let db = self.db.read();
        let coll = db.collection(collection)?;
        let indexes = coll.list_vector_indexes()?;

        Ok(indexes
            .into_iter()
            .map(|idx| {
                serde_json::json!({
                    "name": idx.name,
                    "field": idx.field,
                    "dim": idx.config.dim,
                    "metric": format!("{:?}", idx.config.metric).to_lowercase(),
                    "max_vectors": idx.config.max_vectors,
                    "vector_count": idx.vector_count,
                    "cache_file": idx.cache_file,
                    "created_at": idx.created_at,
                    "hnsw_params": {
                        "m": idx.config.m,
                        "ef_construction": idx.config.ef_construction,
                        "ef_search": idx.config.ef_search
                    }
                })
            })
            .collect())
    }

    /// Drop a vector index
    pub fn drop_vector_index(&self, collection: &str, index_name: &str) -> Result<()> {
        let db = self.db.read();
        let coll = db.collection(collection)?;
        coll.drop_vector_index(index_name)?;
        Ok(())
    }

    /// Perform vector similarity search
    ///
    /// # Arguments
    /// * `collection` - Collection name
    /// * `field` - Field with vector index
    /// * `query_vector` - Query embedding vector
    /// * `limit` - Maximum results to return
    ///
    /// # Returns
    /// Vector of (document, score) tuples sorted by similarity
    pub fn vector_search(
        &self,
        collection: &str,
        field: &str,
        query_vector: &[f32],
        limit: usize,
    ) -> Result<Vec<(Value, f32)>> {
        let db = self.db.read();
        let coll = db.collection(collection)?;
        let results = coll.vector_search(field, query_vector, limit)?;
        Ok(results)
    }

    /// Perform vector similarity search with attribute filter
    ///
    /// # Arguments
    /// * `collection` - Collection name
    /// * `field` - Field with vector index
    /// * `query_vector` - Query embedding vector
    /// * `filter` - MongoDB-style query filter
    /// * `limit` - Maximum results to return
    ///
    /// # Returns
    /// Vector of (document, score) tuples sorted by similarity
    pub fn vector_search_with_filter(
        &self,
        collection: &str,
        field: &str,
        query_vector: &[f32],
        filter: &Value,
        limit: usize,
    ) -> Result<Vec<(Value, f32)>> {
        let db = self.db.read();
        let coll = db.collection(collection)?;
        let results = coll.vector_search_with_filter(field, query_vector, filter, limit)?;
        Ok(results)
    }
}
