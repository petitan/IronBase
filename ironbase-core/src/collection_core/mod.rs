//! # CollectionCore - Dokumentum Gyűjtemény Modul
//!
//! ## Cél
//!
//! A `CollectionCore` a dokumentum műveletek központi implementációja.
//! Kezeli a CRUD műveleteket, indexeket, query cache-t, és sémát.
//! Storage-agnosztikus: működik `StorageEngine` és `MemoryStorage` backend-del is.
//!
//! ## Architektúra
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                    CollectionCore<S: Storage>                   │
//! ├─────────────────────────────────────────────────────────────────┤
//! │                                                                 │
//! │  ┌──────────────────────────────────────────────────────────┐  │
//! │  │                   PUBLIC API                              │  │
//! │  │  find() find_one() insert_one() update_one() delete_one()│  │
//! │  │  aggregate() count() create_index() explain()            │  │
//! │  └────────────────────────┬─────────────────────────────────┘  │
//! │                           │                                     │
//! │                           ▼                                     │
//! │  ┌──────────────────────────────────────────────────────────┐  │
//! │  │              INTERNAL COMPONENTS                          │  │
//! │  ├──────────────────────────────────────────────────────────┤  │
//! │  │                                                          │  │
//! │  │  ┌─────────────┐  ┌─────────────┐  ┌─────────────────┐  │  │
//! │  │  │ IndexManager│  │ QueryCache  │  │ CompiledSchema  │  │  │
//! │  │  │  (B+ tree)  │  │   (LRU)     │  │   (JSON Schema) │  │  │
//! │  │  └──────┬──────┘  └──────┬──────┘  └────────┬────────┘  │  │
//! │  │         │                │                  │            │  │
//! │  │         ▼                ▼                  ▼            │  │
//! │  │  Arc<RwLock<...>>  Arc<QueryCache>  Arc<RwLock<...>>    │  │
//! │  │                                                          │  │
//! │  └──────────────────────────────────────────────────────────┘  │
//! │                           │                                     │
//! │                           ▼                                     │
//! │  ┌──────────────────────────────────────────────────────────┐  │
//! │  │                  Arc<RwLock<Storage>>                     │  │
//! │  │         StorageEngine (file) │ MemoryStorage (RAM)       │  │
//! │  └──────────────────────────────────────────────────────────┘  │
//! │                                                                 │
//! └─────────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Query Végrehajtás Pipeline
//!
//! ```text
//! find(query, options)
//!     │
//!     ▼
//! ┌─────────────────────────────────────────┐
//! │ 1. QUERY CACHE CHECK                    │
//! │    - Hash(query + options)              │
//! │    - Cache hit → return cached result   │
//! └────────────────┬────────────────────────┘
//!                  │ cache miss
//!                  ▼
//! ┌─────────────────────────────────────────┐
//! │ 2. QUERY PLANNING                       │
//! │    - QueryPlanner::plan()               │
//! │    - Index selection                    │
//! │    - QueryPlan: IndexScan | FullScan    │
//! └────────────────┬────────────────────────┘
//!                  │
//!                  ▼
//! ┌─────────────────────────────────────────┐
//! │ 3. DOC ID COLLECTION                    │
//! │    ┌───────────────┬───────────────┐   │
//! │    │  IndexScan    │   FullScan    │   │
//! │    │ B+ tree lookup│ catalog scan  │   │
//! │    └───────┬───────┴───────┬───────┘   │
//! │            │               │           │
//! │            ▼               ▼           │
//! │         doc_ids (filtered)             │
//! └────────────────┬────────────────────────┘
//!                  │
//!                  ▼
//! ┌─────────────────────────────────────────┐
//! │ 4. DOCUMENT FETCH                       │
//! │    - Read docs from storage by offset   │
//! │    - Apply post-filter (complex queries)│
//! └────────────────┬────────────────────────┘
//!                  │
//!                  ▼
//! ┌─────────────────────────────────────────┐
//! │ 5. POST-PROCESSING                      │
//! │    - Sort (if not index-sorted)         │
//! │    - Skip/Limit pagination              │
//! │    - Projection (field selection)       │
//! └────────────────┬────────────────────────┘
//!                  │
//!                  ▼
//! ┌─────────────────────────────────────────┐
//! │ 6. CACHE UPDATE                         │
//! │    - Store result in LRU cache          │
//! └─────────────────────────────────────────┘
//! ```
//!
//! ## Index Management
//!
//! ```text
//! IndexManager tartalma:
//! ┌─────────────────────────────────────────────────────────────┐
//! │  btree_indexes: HashMap<String, BPlusTree>                  │
//! │    - _id index (automatikus, unique)                        │
//! │    - user-defined indexes                                   │
//! │                                                             │
//! │  fuzzy_indexes: HashMap<String, FuzzyIndex>                 │
//! │    - Jaro-Winkler, Levenshtein, Damerau-Levenshtein        │
//! │                                                             │
//! │  fulltext_indexes: HashMap<String, FulltextIndex>           │
//! │    - TF-IDF scoring, stemming, stop words                   │
//! └─────────────────────────────────────────────────────────────┘
//!
//! Index Persistence:
//! ┌─────────────────────────────────────────────────────────────┐
//! │  .mlite file     → B+ tree index metadata (JSON)            │
//! │  .mlite.idx      → B+ tree binary data                      │
//! │  .mlite.fzidx    → Fuzzy index data                         │
//! │  .mlite.ftidx    → Fulltext index data                      │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Locking Stratégia
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │              FINE-GRAINED LOCKING                           │
//! ├─────────────────────────────────────────────────────────────┤
//! │                                                             │
//! │  Storage Lock (RwLock):                                     │
//! │  ┌─────────────────────────────────────────────────────┐   │
//! │  │  READ:  find, count, aggregate (concurrent)         │   │
//! │  │  WRITE: insert, update, delete (exclusive)          │   │
//! │  └─────────────────────────────────────────────────────┘   │
//! │                                                             │
//! │  Index Lock (RwLock):                                       │
//! │  ┌─────────────────────────────────────────────────────┐   │
//! │  │  READ:  index lookup (concurrent)                   │   │
//! │  │  WRITE: index update, create/drop (exclusive)       │   │
//! │  └─────────────────────────────────────────────────────┘   │
//! │                                                             │
//! │  Query Cache (internal RwLock):                             │
//! │  ┌─────────────────────────────────────────────────────┐   │
//! │  │  Lock-free read path (DashMap internally)           │   │
//! │  │  Automatic eviction (LRU, capacity: 1000)           │   │
//! │  └─────────────────────────────────────────────────────┘   │
//! │                                                             │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Invariánsok
//!
//! 1. **_id Index**: Minden collection-nek van automatikus `_id` index
//! 2. **Unique Constraint**: Insert előtt unique index ellenőrzés
//! 3. **Schema Validation**: Ha van schema, insert/update előtt validálás
//! 4. **Cache Invalidation**: Bármely write művelet törli a query cache-t
//! 5. **Index Consistency**: Write után MINDIG frissül az index
//!
//! ## Submodulok
//!
//! | Modul               | Felelősség                                    |
//! |---------------------|-----------------------------------------------|
//! | `raw_operations`    | Prepare/Persist pattern, WAL integráció       |
//! | `cursor`            | FindCursor streaming iterator                 |
//! | `index_ops`         | create_index, drop_index, explain, hint       |
//! | `index_persistence` | .idx/.fzidx/.ftidx fájl kezelés               |
//! | `constraints`       | Batch unique constraint validáció             |
//! | `schema`            | JSON Schema validáció                         |
//! | `search`            | fulltext_search, fuzzy_search                 |
//! | `update_operators`  | $set, $inc, $push, stb. implementáció         |
//!
//! ## Kapcsolódó Modulok
//!
//! - [`crate::database`] - DatabaseCore, collection létrehozás
//! - [`crate::index`] - IndexManager, BPlusTree, FuzzyIndex, FulltextIndex
//! - [`crate::query`] - Query parsing és matching
//! - [`crate::query_planner`] - Index selection
//! - [`crate::query_cache`] - LRU query result cache
//! - [`crate::storage`] - StorageEngine, MemoryStorage

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use parking_lot::RwLock;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

use crate::document::{Document, DocumentId};
use crate::error::{IronBaseError, Result};
use crate::index::{IndexKey, IndexManager};
use crate::query::Query;
use crate::query_cache::{QueryCache, QueryHash};
use crate::query_planner::{QueryPlan, QueryPlanner};
use crate::storage::{RawStorage, Storage};
use crate::{log_debug, log_error, log_trace, log_warn};

mod constraints;
mod cursor;
mod index_ops;
mod index_persistence;
mod raw_operations;
pub(crate) mod schema;
mod search;
mod update_operators;

pub(crate) use self::constraints::BatchConstraintValidator;
pub(crate) use self::index_persistence::try_load_index_from_file;
pub(crate) use self::index_persistence::{
    build_fulltext_index_file_path, build_fuzzy_index_file_path, persist_index_to_disk,
    try_load_fulltext_index_from_file, try_load_fuzzy_index_from_file,
};
use self::schema::CompiledSchema;

// Re-export the sealed RawOperations trait for crate-internal use
pub(crate) use self::raw_operations::RawOperations;

// Re-export prepared operation structs for WAL-first batch mode
pub(crate) use self::raw_operations::InsertOnePrepared;

// Re-export batch-mode prepared structs (WAL ORDERING FIX)
pub(crate) use self::raw_operations::{DeleteOnePreparedBatch, UpdateOnePreparedBatch};

// Re-export FindCursor for streaming queries
pub use self::cursor::FindCursor;

// ============================================================================
// CONSTANTS
// ============================================================================

/// Default capacity for the LRU query cache
const QUERY_CACHE_CAPACITY: usize = 1000;

// OOM protection: try_reserve() dynamically checks available memory (jemalloc-aware)

/// Threshold for logging a warning about large document loads
const LARGE_QUERY_WARNING_THRESHOLD: usize = 10_000;

/// Result of insert_many operation
#[derive(Debug, Clone)]
pub struct InsertManyResult {
    pub inserted_ids: Vec<DocumentId>,
    pub inserted_count: usize,
}

/// Query execution context extracted from FindOptions
/// Single Responsibility: Transform user options into execution strategy
#[derive(Debug)]
struct QueryExecutionContext {
    /// Single-field sort optimization info
    sort_field: Option<String>,
    sort_descending: bool,

    /// Whether to defer limit/skip until after sorting
    apply_limit_after_sort: bool,

    /// Pre-fetch parameters (0/None if deferred)
    fetch_skip: usize,
    fetch_limit: Option<usize>,

    /// Original user options for post-processing
    original_skip: usize,
    original_limit: Option<usize>,

    /// Original sort specification
    sort_spec: Option<Vec<(String, i32)>>,

    /// Projection specification
    projection: Option<HashMap<String, i32>>,
}

impl QueryExecutionContext {
    /// Build execution context from FindOptions
    fn from_options(options: &crate::find_options::FindOptions) -> Self {
        let original_skip = options.skip.unwrap_or(0);
        let original_limit = options.limit;

        // Extract single-field sort for index optimization
        let (sort_field, sort_descending) = options
            .sort
            .as_ref()
            .filter(|s| s.len() == 1)
            .map(|s| (Some(s[0].0.clone()), s[0].1 < 0))
            .unwrap_or((None, false));

        // Determine fetch strategy: sort present → load all, then paginate
        // NOTE: We defer limit when sorting because we don't know at this point
        // if the index will be used for sorting. The actual optimization happens
        // in collect_doc_ids_with_options -> try_index_sorted_scan.
        let apply_limit_after_sort = options.sort.is_some();
        let (fetch_skip, fetch_limit) = if apply_limit_after_sort {
            (0, None)
        } else {
            (original_skip, original_limit)
        };

        Self {
            sort_field,
            sort_descending,
            apply_limit_after_sort,
            fetch_skip,
            fetch_limit,
            original_skip,
            original_limit,
            sort_spec: options.sort.clone(),
            projection: options.projection.clone(),
        }
    }

    /// Get sort field reference (for collect_doc_ids_with_options)
    fn sort_field_ref(&self) -> Option<&str> {
        self.sort_field.as_deref()
    }

    /// Determine if in-memory sort is needed (when index didn't sort for us)
    fn needs_memory_sort(&self, index_sorted: bool) -> bool {
        match &self.sort_spec {
            Some(sort) => !(index_sorted && sort.len() == 1),
            None => false,
        }
    }

    /// Apply pagination after sorting (returns owned docs)
    fn apply_post_sort_pagination(&self, docs: Vec<Value>) -> Vec<Value> {
        if self.apply_limit_after_sort {
            crate::find_options::apply_limit_skip(
                docs,
                self.original_limit,
                Some(self.original_skip),
            )
        } else {
            docs
        }
    }

    /// Apply projection to documents (returns owned docs)
    fn apply_projection_to_docs(&self, docs: Vec<Value>) -> crate::error::Result<Vec<Value>> {
        match &self.projection {
            Some(proj) => docs
                .into_iter()
                .map(|doc| crate::find_options::apply_projection(&doc, proj))
                .collect(),
            None => Ok(docs),
        }
    }
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
    /// Reference to database closed flag - prevents writes after db.close()
    is_closed: Arc<AtomicBool>,
}

impl<S: Storage + RawStorage> CollectionCore<S> {
    // ========== CONSTRUCTOR ==========

    /// Create new collection (or get existing)
    pub fn new(name: String, storage: Arc<RwLock<S>>, is_closed: Arc<AtomicBool>) -> Result<Self> {
        // Create collection if it doesn't exist
        {
            let mut storage_guard = storage.write();
            if storage_guard.get_collection_meta(&name).is_none() {
                storage_guard.create_collection(&name)?;
            }
        }

        // Initialize index manager with automatic _id index
        let mut index_manager = IndexManager::new();

        // Create automatic _id index (unique, non-sparse - every doc has _id)
        let id_index_name = format!("{}_id", name);
        index_manager.create_btree_index(
            id_index_name.clone(),
            "_id".to_string(),
            true,  // unique
            false, // sparse - _id is always present
        )?;

        // PERSISTENCE FIX: Load persisted indexes and rebuild from document catalog
        let schema_definition = {
            let storage_guard = storage.write();
            let meta = storage_guard
                .get_collection_meta(&name)
                .ok_or_else(|| IronBaseError::CollectionNotFound(name.clone()))?;
            meta.schema.clone()
        };

        {
            let storage_guard = storage.write();
            let meta = storage_guard
                .get_collection_meta(&name)
                .ok_or_else(|| IronBaseError::CollectionNotFound(name.clone()))?;

            // Clone metadata to avoid borrow issues
            let catalog = meta.document_catalog.clone();
            let persisted_indexes = meta.indexes.clone();
            let persisted_fuzzy_indexes = meta.fuzzy_indexes.clone();
            let persisted_fulltext_indexes = meta.fulltext_indexes.clone();

            log_debug!(
                "Collection '{}' - catalog size: {}, persisted indexes: {}, fuzzy: {}, fulltext: {}",
                name,
                catalog.len(),
                persisted_indexes.len(),
                persisted_fuzzy_indexes.len(),
                persisted_fulltext_indexes.len()
            );

            // Get db_path for .idx file loading (before releasing lock)
            let db_path = storage_guard.get_file_path().to_string();

            drop(storage_guard); // Release write lock before rebuilding

            // Load persisted custom indexes (if any)
            for index_meta in &persisted_indexes {
                // Skip _id index (already created)
                if index_meta.name == id_index_name {
                    continue;
                }

                // Try to load from .idx file first (for index structure/metadata)
                // NOTE: We still rebuild from documents below to ensure consistency
                if let Some(loaded_tree) = try_load_index_from_file(&db_path, index_meta) {
                    log_debug!(
                        "Loaded index '{}' from .idx file (will rebuild from documents)",
                        index_meta.name
                    );
                    index_manager.add_loaded_index(loaded_tree);
                    // Index loaded, but we still rebuild from documents below
                } else {
                    // Fallback: create empty index (will be rebuilt from documents)
                    log_debug!(
                        "Creating index '{}' on field '{}' (will rebuild from documents)",
                        index_meta.name,
                        index_meta.field
                    );

                    // Create index with persisted sparse flag
                    index_manager.create_btree_index(
                        index_meta.name.clone(),
                        index_meta.field.clone(),
                        index_meta.unique,
                        index_meta.sparse,
                    )?;
                }
            }

            // PERSISTENCE FIX: Load persisted fuzzy indexes
            for fuzzy_meta in &persisted_fuzzy_indexes {
                log_debug!(
                    "Loading fuzzy index '{}' on field '{}' (will rebuild from documents)",
                    fuzzy_meta.name,
                    fuzzy_meta.field
                );
                index_manager.create_fuzzy_index(
                    fuzzy_meta.name.clone(),
                    fuzzy_meta.field.clone(),
                    fuzzy_meta.algorithm,
                    fuzzy_meta.threshold,
                )?;
            }

            // PERSISTENCE FIX: Load persisted fulltext indexes from .ftidx files
            for ft_meta in &persisted_fulltext_indexes {
                // Try to load from .ftidx file first
                if let Some(loaded_index) = try_load_fulltext_index_from_file(&db_path, ft_meta) {
                    log_debug!(
                        "Loaded fulltext index '{}' from .ftidx file ({} docs, {} tokens)",
                        ft_meta.name,
                        loaded_index.doc_count(),
                        loaded_index.token_count()
                    );
                    index_manager.add_loaded_fulltext_index(loaded_index);
                } else {
                    // Create new index with disk storage (will be rebuilt from documents)
                    log_debug!(
                        "Creating fulltext index '{}' on field '{}' (will rebuild from documents)",
                        ft_meta.name,
                        ft_meta.field
                    );
                    let storage_path = build_fulltext_index_file_path(&db_path, &ft_meta.name);
                    index_manager.create_fulltext_index_with_storage(
                        ft_meta.name.clone(),
                        ft_meta.field.clone(),
                        ft_meta.language,
                        Some(ft_meta.min_word_length),
                        Some(ft_meta.accent_folding),
                        storage_path,
                    )?;
                }
            }

            // Rebuild all indexes from document catalog (always rebuild to ensure consistency)
            log_debug!(
                "Starting index rebuild from {} catalog entries",
                catalog.len()
            );

            // OPTIMIZATION: Sort by offset for sequential disk reads
            // This eliminates random disk seeks which are 5-10ms each on spinning disks.
            // For N documents, this reduces I/O from O(N * 5-10ms) to O(N * 0.01ms).
            let mut sorted_entries: Vec<_> = catalog.iter().collect();
            sorted_entries.sort_by_key(|(_, offset)| *offset);

            let mut storage_guard = storage.write();
            let mut rebuilt_count = 0;
            for (_id_key, offset) in sorted_entries {
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

                                        // Rebuild ALL custom indexes (always rebuild to ensure correctness)
                                        // FIX #19: Use index.extract_key() for proper compound index support
                                        for index_meta in &persisted_indexes {
                                            if index_meta.name == id_index_name {
                                                continue;
                                            }
                                            // NOTE: We always rebuild from documents to ensure index consistency
                                            // The .idx file is only used as a fast path for initial loading,
                                            // but we still rebuild to catch any entries added after initial creation

                                            // FIX #19: Use extract_key() which handles compound indexes correctly
                                            if let Some(index) =
                                                index_manager.get_btree_index_mut(&index_meta.name)
                                            {
                                                let key = index.extract_key(&doc);
                                                // Check if all key components are Null
                                                let is_all_null = match &key {
                                                    IndexKey::Null => true,
                                                    IndexKey::Compound(keys) => keys
                                                        .iter()
                                                        .all(|k| matches!(k, IndexKey::Null)),
                                                    _ => false,
                                                };
                                                // For unique indexes: include null keys (null is a value)
                                                // For non-unique indexes: skip null keys (no query benefit)
                                                if !is_all_null || index.metadata.unique {
                                                    let _ = index.insert(key, doc_id.clone());
                                                    rebuilt_count += 1;
                                                }
                                            }
                                        }

                                        // PERSISTENCE FIX: Rebuild fuzzy indexes from documents
                                        for fuzzy_meta in &persisted_fuzzy_indexes {
                                            if let Some(fuzzy_index) =
                                                index_manager.get_fuzzy_index_mut(&fuzzy_meta.name)
                                            {
                                                // Extract field value using dot notation
                                                if let Some(value) =
                                                    crate::value_utils::get_nested_value(
                                                        &doc,
                                                        &fuzzy_meta.field,
                                                    )
                                                {
                                                    if let Some(text) = value.as_str() {
                                                        fuzzy_index.insert(text, doc_id.clone());
                                                    }
                                                }
                                            }
                                        }

                                        // PERSISTENCE FIX: Rebuild fulltext indexes from documents
                                        // SKIP indexes that already have data (loaded from .ftidx file)
                                        for ft_meta in &persisted_fulltext_indexes {
                                            if let Some(ft_index) =
                                                index_manager.get_fulltext_index_mut(&ft_meta.name)
                                            {
                                                // Skip if index was loaded from file (has data)
                                                if ft_index.doc_count() > 0 {
                                                    continue;
                                                }
                                                // Extract field value using dot notation
                                                if let Some(value) =
                                                    crate::value_utils::get_nested_value(
                                                        &doc,
                                                        &ft_meta.field,
                                                    )
                                                {
                                                    if let Some(text) = value.as_str() {
                                                        let _ = ft_index.insert(&doc_id, text);
                                                    }
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
            query_cache: Arc::new(QueryCache::new(QUERY_CACHE_CAPACITY)),
            schema: Arc::new(RwLock::new(compiled_schema)),
            is_closed,
        })
    }

    /// Create a collection with shared IndexManager and Schema
    ///
    /// This constructor is used by DatabaseCore to share IndexManagers and Schemas
    /// across multiple CollectionCore instances, fixing both "stale index" and
    /// "stale schema" problems.
    pub(crate) fn with_shared_indexes(
        name: String,
        storage: Arc<RwLock<S>>,
        indexes: Arc<RwLock<IndexManager>>,
        schema: Arc<RwLock<Option<CompiledSchema>>>,
        is_closed: Arc<AtomicBool>,
    ) -> Result<Self> {
        // Ensure collection exists
        {
            let mut storage_guard = storage.write();
            if storage_guard.get_collection_meta(&name).is_none() {
                storage_guard.create_collection(&name)?;
            }
        }

        Ok(CollectionCore {
            name,
            storage,
            indexes, // Shared!
            query_cache: Arc::new(QueryCache::new(QUERY_CACHE_CAPACITY)),
            schema, // Shared!
            is_closed,
        })
    }

    /// Create CollectionCore for EXISTING collection (readonly path - no creation)
    ///
    /// This method uses READ locks only, avoiding write lock contention.
    /// Use this for read operations where the collection must already exist.
    ///
    /// Returns `CollectionNotFound` if collection doesn't exist in storage.
    pub(crate) fn with_shared_indexes_readonly(
        name: String,
        storage: Arc<RwLock<S>>,
        indexes: Arc<RwLock<IndexManager>>,
        schema: Arc<RwLock<Option<CompiledSchema>>>,
        is_closed: Arc<AtomicBool>,
    ) -> Result<Self> {
        // Verify collection exists (READ lock only)
        {
            let storage_guard = storage.read();
            if storage_guard.get_collection_meta(&name).is_none() {
                return Err(IronBaseError::CollectionNotFound(name.clone()));
            }
        }

        Ok(CollectionCore {
            name,
            storage,
            indexes, // Shared!
            query_cache: Arc::new(QueryCache::new(QUERY_CACHE_CAPACITY)),
            schema, // Shared!
            is_closed,
        })
    }

    /// Check if database is closed - prevents writes after db.close()
    pub(crate) fn check_not_closed(&self) -> Result<()> {
        if self.is_closed.load(Ordering::SeqCst) {
            return Err(IronBaseError::DatabaseClosed);
        }
        Ok(())
    }

    fn compile_schema(schema: &Value) -> Result<CompiledSchema> {
        CompiledSchema::from_value(schema)
    }

    pub(crate) fn validate_value_against_schema(&self, value: &Value) -> Result<()> {
        let guard = self.schema.read();
        if let Some(schema) = guard.as_ref() {
            schema.validate(value)?;
        }
        Ok(())
    }

    fn validate_document(&self, document: &Document) -> Result<()> {
        let value = serde_json::to_value(document)
            .map_err(|e| IronBaseError::Serialization(e.to_string()))?;
        self.validate_value_against_schema(&value)
    }

    /// Set or clear the JSON schema for this collection.
    pub fn set_schema(&self, schema: Option<Value>) -> Result<()> {
        self.check_not_closed()?;
        let compiled = if let Some(ref raw) = schema {
            Some(Self::compile_schema(raw)?)
        } else {
            None
        };

        {
            let mut storage = self.storage.write();
            let meta = storage
                .get_collection_meta_mut(&self.name)
                .ok_or_else(|| IronBaseError::CollectionNotFound(self.name.clone()))?;
            meta.schema = schema;
            storage.flush()?;
        }

        let mut guard = self.schema.write();
        *guard = compiled;
        Ok(())
    }

    /// Get the JSON schema for this collection (if any)
    pub fn get_schema(&self) -> Result<Option<Value>> {
        self.check_not_closed()?;
        let storage = self.storage.read();
        Ok(storage
            .get_collection_meta(&self.name)
            .and_then(|meta| meta.schema.clone()))
    }

    // ========== BATCH VALIDATION ==========

    /// Validate batch of documents for unique constraint violations within the batch
    ///
    /// This is a pre-insert validation step that catches duplicates BEFORE any
    /// documents are written. Used by DatabaseCore::insert_many for atomic failure.
    ///
    /// FIX #18: Now checks BOTH:
    /// 1. Duplicates within the batch (via BatchConstraintValidator)
    /// 2. Conflicts with EXISTING documents in the index
    #[allow(dead_code)] // Will be removed in Phase 6 of WAL refactor
    pub(crate) fn validate_batch_constraints(
        &self,
        documents: &[HashMap<String, Value>],
    ) -> Result<()> {
        let indexes = self.indexes.read();
        let mut batch_validator = BatchConstraintValidator::new(&indexes, &self.name);
        let id_index_name = format!("{}_id", self.name);

        for document in documents {
            let doc_value = serde_json::to_value(document)
                .map_err(|e| IronBaseError::Serialization(e.to_string()))?;

            // Check for duplicates WITHIN the batch
            batch_validator.check_and_track(&doc_value)?;

            // FIX #18: Check against EXISTING documents in the index
            // This ensures atomic failure - all checks happen before any writes.
            for index_name in indexes.list_indexes() {
                if index_name == id_index_name {
                    continue; // _id handled separately
                }

                if let Some(index) = indexes.get_btree_index(&index_name) {
                    if !index.metadata.unique {
                        continue; // Only check unique indexes
                    }

                    let field = &index.metadata.field;
                    // Use get_nested_value to support dot notation (e.g., "profile.code")
                    if let Some(field_value) =
                        crate::value_utils::get_nested_value(&doc_value, field)
                    {
                        let index_key = IndexKey::from(field_value);
                        if index.search(&index_key).is_some() {
                            return Err(IronBaseError::IndexError(format!(
                                "Duplicate key: {:?} in field '{}' (unique index)",
                                index_key, field
                            )));
                        }
                    }
                }
            }
        }
        Ok(())
    }

    // ========== QUERY OPERATIONS ==========

    /// Find documents matching query
    ///
    /// MEMORY FIX: Removed scan_documents_via_catalog() optimization for find({}).
    /// That optimization loaded ALL documents into a HashMap at once, causing OOM
    /// on large collections (e.g., 21GB emails). Now uses streaming: collect IDs first,
    /// then load documents one by one. Slightly slower but memory-safe.
    ///
    /// For large collections, consider using find_streaming() instead.
    pub fn find(&self, query_json: &Value) -> Result<Vec<Value>> {
        self.check_not_closed()?;
        log_debug!("find() called with query: {:?}", query_json);

        // STREAMING: Collect IDs first (small), then load docs one by one
        // This avoids bulk-loading all documents into memory at once
        let doc_ids = self.collect_doc_ids(query_json)?;
        let mut results = Vec::new();
        results.try_reserve(doc_ids.len()).map_err(|e| {
            IronBaseError::InvalidQuery(format!(
                "Out of memory: cannot allocate space for {} documents ({}). \
                Solutions: 1) Add 'limit' to your query, 2) Use an index, 3) Use find_streaming() for large results.",
                doc_ids.len(),
                e
            ))
        })?;
        for doc_id in doc_ids {
            if let Some(doc) = self.read_document_by_id(&doc_id)? {
                results.push(doc);
            }
        }
        Ok(results)
    }

    /// Find documents with options (projection, sort, limit, skip)
    ///
    /// Clean Architecture: Uses QueryExecutionContext for configuration,
    /// separating setup logic from execution.
    pub fn find_with_options(
        &self,
        query_json: &Value,
        options: crate::find_options::FindOptions,
    ) -> Result<Vec<Value>> {
        self.check_not_closed()?;
        // Phase 1: Build execution context (all setup logic centralized)
        let ctx = QueryExecutionContext::from_options(&options);

        // Phase 2: Collect document IDs (may use index for sorting)
        // Pass original skip/limit for index-based sort optimization (early termination)
        let (doc_ids, index_sorted) = self.collect_doc_ids_with_options(
            query_json,
            None,
            ctx.sort_field_ref(),
            ctx.sort_descending,
            ctx.fetch_skip,
            ctx.fetch_limit,
            ctx.sort_field.is_none(),
            ctx.original_skip,  // For index-based sort: skip
            ctx.original_limit, // For index-based sort: enables early termination
        )?;

        // Phase 3: Document loading with OOM protection
        let doc_count = doc_ids.len();

        // Warning for large queries
        if doc_count > LARGE_QUERY_WARNING_THRESHOLD {
            log_warn!(
                "find on '{}': loading {} documents - this may take a while",
                self.name,
                doc_count
            );
        }

        // Load documents with progress logging for very large sets
        // Use try_reserve to detect OOM before it happens (especially important with jemalloc)
        let mut docs = Vec::new();
        docs.try_reserve(doc_count).map_err(|e| {
            IronBaseError::InvalidQuery(format!(
                "Out of memory: cannot allocate space for {} documents ({}). \
                Solutions: 1) Add 'limit' to your query, 2) Use an index, 3) Use find_streaming() for large results.",
                doc_count, e
            ))
        })?;
        let mut loaded = 0;
        for doc_id in doc_ids {
            if let Some(doc) = self.read_document_by_id(&doc_id)? {
                docs.push(doc);
                loaded += 1;
                // Progress logging every 10,000 documents
                if loaded % 10_000 == 0 && doc_count > LARGE_QUERY_WARNING_THRESHOLD {
                    log_debug!(
                        "find on '{}': loaded {}/{} documents",
                        self.name,
                        loaded,
                        doc_count
                    );
                }
            }
        }

        // Phase 4: Post-processing pipeline
        // 4a. Apply sort if needed (index didn't sort for us)
        if ctx.needs_memory_sort(index_sorted) {
            if let Some(ref sort_spec) = ctx.sort_spec {
                crate::find_options::apply_sort(&mut docs, sort_spec)?;
            }
        }

        // 4b. Apply pagination after sorting
        // SKIP if index already applied skip/limit (index_sorted + empty query)
        let docs = if index_sorted && Self::query_matches_all(query_json) {
            // Index-based sort already applied skip/limit - don't apply again
            docs
        } else {
            ctx.apply_post_sort_pagination(docs)
        };

        // 4c. Apply projection
        let docs = ctx.apply_projection_to_docs(docs)?;

        Ok(docs)
    }

    /// Find documents with options and optionally include total count
    ///
    /// Returns a `FindResult` containing documents and optional total count.
    /// When `options.include_total` is true, also runs a count query to get
    /// the total number of matching documents (ignoring limit/skip).
    ///
    /// # Example
    /// ```rust,ignore
    /// use ironbase_core::FindOptions;
    ///
    /// let options = FindOptions::new()
    ///     .with_limit(10)
    ///     .with_skip(20)
    ///     .with_include_total(true);
    ///
    /// let result = collection.find_with_result(&json!({}), options)?;
    /// println!("Page: {} of {} total", result.documents.len(), result.total.unwrap());
    /// ```
    pub fn find_with_result(
        &self,
        query_json: &Value,
        options: crate::find_options::FindOptions,
    ) -> Result<crate::find_options::FindResult> {
        let include_total = options.include_total;

        // Get documents with options
        let documents = self.find_with_options(query_json, options)?;

        // Get total count if requested
        let total = if include_total {
            Some(self.count_documents(query_json)?)
        } else {
            None
        };

        Ok(crate::find_options::FindResult { documents, total })
    }

    /// Streaming cursor for large result sets
    ///
    /// Returns a cursor that lazily loads documents, allowing memory-efficient
    /// iteration over large result sets.
    ///
    /// # Example
    /// ```rust,ignore
    /// // Process 1 million documents without loading all into memory
    /// let mut cursor = collection.find_streaming(&json!({}))?;
    ///
    /// // Set batch size (optional, default is 100)
    /// let mut cursor = cursor.with_batch_size(500);
    ///
    /// // Process in batches
    /// while !cursor.is_finished() {
    ///     let batch = cursor.next_batch()?;
    ///     for doc in batch {
    ///         process_document(&doc);
    ///     }
    /// }
    /// ```
    pub fn find_streaming(&self, query_json: &Value) -> Result<FindCursor<'_, S>> {
        self.check_not_closed()?;
        let (doc_ids, _) = self
            .collect_doc_ids_with_options(query_json, None, None, false, 0, None, true, 0, None)?;
        Ok(FindCursor::new(self, doc_ids))
    }

    /// Find one document matching query
    ///
    /// Uses QueryPlanner for index optimization when available.
    /// For `_id` queries, uses direct O(1) catalog lookup.
    pub fn find_one(&self, query_json: &Value) -> Result<Option<Value>> {
        self.check_not_closed()?;
        // OPTIMIZATION: Check if this is an _id equality query (O(1) lookup)
        // This is faster than going through QueryPlanner for the most common case
        if let Some(query_obj) = query_json.as_object() {
            if query_obj.len() == 1 && query_obj.contains_key("_id") {
                if let Some(id_val) = query_obj.get("_id") {
                    // Direct O(1) lookup using document_catalog
                    if let Ok(doc_id) = serde_json::from_value::<DocumentId>(id_val.clone()) {
                        if let Some(doc) = self.read_document_by_id(&doc_id)? {
                            // Verify query still matches (for consistency)
                            let parsed_query = Query::from_json(query_json)?;
                            // PERF: from_value borrows+clones, still faster than to_string+from_json
                            let document = Document::from_value(&doc)?;

                            if parsed_query.matches(&document)? {
                                return Ok(Some(doc));
                            }
                        }
                    }
                    return Ok(None);
                }
            }
        }

        // Use QueryPlanner with limit=1 - enables index usage for indexed fields
        // This was previously a full collection scan (issue #19)
        let (doc_ids, _) = self.collect_doc_ids_with_options(
            query_json,
            None,
            None,
            false,
            0,
            Some(1),
            true,
            0,
            None,
        )?;

        if let Some(doc_id) = doc_ids.first() {
            self.read_document_by_id(doc_id)
        } else {
            Ok(None)
        }
    }

    /// Count documents matching query
    ///
    /// Uses QueryPlanner for index optimization when available.
    /// Performance optimized: Uses streaming count without Vec allocation for scans.
    pub fn count_documents(&self, query_json: &Value) -> Result<u64> {
        self.check_not_closed()?;

        // Fast path: empty query = count all (O(1))
        if Self::query_matches_all(query_json) {
            let storage = self.storage.read();
            return Ok(storage.get_live_count(&self.name).unwrap_or(0));
        }

        // Fast path: _id query = O(1) lookup
        if let Some(doc_id) = Self::extract_id_query(query_json) {
            return Ok(if self.read_document_by_id(&doc_id)?.is_some() {
                1
            } else {
                0
            });
        }

        // Index-aware count with Vec-less fallback
        self.count_with_plan(query_json)
    }

    /// Count using QueryPlanner for index optimization
    ///
    /// Tries to use available indexes for faster counting.
    /// For simple equality queries, trusts the index count directly.
    /// Falls back to streaming scan if no suitable index exists.
    fn count_with_plan(&self, query_json: &Value) -> Result<u64> {
        // Try index-based counting
        let indexes = self.indexes.read();
        let index_fields = indexes.list_indexes_with_compound_info();

        if let Some((_, plan)) = QueryPlanner::analyze_query_with_fields(query_json, &index_fields)
        {
            // For simple equality IndexScan, trust the index count directly
            // This avoids reading each document just to verify
            match &plan {
                QueryPlan::IndexScan {
                    ref index_name,
                    ref key,
                    is_compound,
                    ..
                } => {
                    if let Some(index) = indexes.get_btree_index(index_name) {
                        let doc_ids = if *is_compound {
                            let (start, end) = index.build_prefix_range(key.clone());
                            index.range_scan(&start, &end, true, true)
                        } else {
                            index.range_scan(key, key, true, true)
                        };
                        drop(indexes);
                        // Count live documents (exclude tombstones)
                        return self.count_live_docs_from_ids(&doc_ids);
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
                        let doc_ids =
                            index.range_scan(start_key, end_key, *inclusive_start, *inclusive_end);
                        drop(indexes);
                        return self.count_live_docs_from_ids(&doc_ids);
                    }
                }
                _ => {}
            }

            drop(indexes);
        } else {
            drop(indexes);
        }

        // Fallback: streaming count without Vec allocation
        self.count_with_scan(query_json)
    }

    /// Count live documents from a list of doc IDs (skip tombstones)
    ///
    /// Optimized: checks tombstone by pattern matching raw bytes instead of parsing JSON.
    fn count_live_docs_from_ids(&self, doc_ids: &[DocumentId]) -> Result<u64> {
        let storage = self.storage.read();
        let meta = storage
            .get_collection_meta(&self.name)
            .ok_or_else(|| IronBaseError::CollectionNotFound(self.name.clone()))?;

        // Fast path: if no tombstones exist, return index count directly
        let live_count = storage.get_live_count(&self.name).unwrap_or(0) as usize;
        let catalog_len = meta.document_catalog.len();
        if live_count == catalog_len {
            // No tombstones - trust index count
            // But still filter to docs that exist in catalog (handles stale index entries)
            let count = doc_ids
                .iter()
                .filter(|id| meta.document_catalog.contains_key(*id))
                .count();
            return Ok(count as u64);
        }

        // Tombstones exist - check each document
        // Optimized: check for tombstone pattern without full JSON parse
        const TOMBSTONE_PATTERN: &[u8] = b"\"_tombstone\":true";

        let mut count = 0u64;
        for doc_id in doc_ids {
            if let Some(&offset) = meta.document_catalog.get(doc_id) {
                if let Ok(doc_bytes) = storage.read_data_at(offset) {
                    // Fast check: tombstones are small and contain the pattern
                    // Skip if it looks like a tombstone
                    if doc_bytes.len() < 100
                        && doc_bytes
                            .windows(TOMBSTONE_PATTERN.len())
                            .any(|w| w == TOMBSTONE_PATTERN)
                    {
                        continue; // Skip tombstone
                    }
                    count += 1;
                }
            }
        }

        Ok(count)
    }

    /// Count by scanning without building Vec (memory efficient)
    ///
    /// This is the fallback when no index is available.
    /// Unlike collect_doc_ids, this doesn't allocate a Vec of all matching IDs.
    /// Uses read lock only for better concurrency.
    fn count_with_scan(&self, query_json: &Value) -> Result<u64> {
        let parsed_query = Query::from_json(query_json)?;
        let storage = self.storage.read(); // Read lock - better concurrency

        // Get catalog entries
        let catalog_entries: Vec<(DocumentId, u64)> = {
            let meta = storage
                .get_collection_meta(&self.name)
                .ok_or_else(|| IronBaseError::CollectionNotFound(self.name.clone()))?;
            meta.document_catalog
                .iter()
                .map(|(id, &offset)| (id.clone(), offset))
                .collect()
        };

        let mut count = 0u64;

        for (_doc_id, offset) in catalog_entries {
            // Read document from storage (immutable read)
            let doc_bytes = match storage.read_data_at(offset) {
                Ok(bytes) => bytes,
                Err(_) => continue, // Skip corrupted entries
            };

            // Parse document
            let doc: Value = match serde_json::from_slice(&doc_bytes) {
                Ok(d) => d,
                Err(_) => continue, // Skip corrupted JSON
            };

            // Skip tombstones
            if doc
                .get("_tombstone")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                continue;
            }

            // Apply query filter
            let document = Document::from_value_owned(doc)?;
            if parsed_query.matches(&document)? {
                count += 1;
            }
        }

        Ok(count)
    }

    // =========================================================================
    // HELPER FUNCTIONS (Extracted for reduced CC and cognitive complexity)
    // =========================================================================

    /// Try O(1) _id lookup if query is simple _id equality
    ///
    /// Returns:
    /// - `Ok(Some(docs))` if _id optimization was successful (may be empty if doc not found)
    /// - `Ok(None)` if query doesn't match _id pattern, caller should fallback to scan
    ///
    /// NOTE: Currently unused after streaming refactor, kept for potential future use.
    #[allow(dead_code)]
    fn try_id_query_optimization(
        &self,
        query_json: &Value,
    ) -> Result<Option<HashMap<DocumentId, Value>>> {
        // 1. Check: {_id: value} format?
        let query_obj = match query_json.as_object() {
            Some(obj) if obj.len() == 1 && obj.contains_key("_id") => obj,
            _ => return Ok(None), // Fallback needed
        };

        // 2. DocumentId conversion
        let id_val = query_obj.get("_id").unwrap(); // Safe: we checked contains_key above
        let doc_id = match serde_json::from_value::<DocumentId>(id_val.clone()) {
            Ok(id) => id,
            Err(_) => return Ok(None), // Invalid _id format, fallback to scan
        };

        // 3. O(1) lookup
        if let Some(doc) = self.read_document_by_id(&doc_id)? {
            let mut result = HashMap::new();
            result.insert(doc_id, doc);
            Ok(Some(result))
        } else {
            Ok(Some(HashMap::new())) // Empty result (doc doesn't exist)
        }
    }

    /// Batch write tombstones and updated documents to storage
    ///
    /// Acquires storage lock once and writes all updates atomically.
    /// Each update consists of: (doc_id, tombstone, updated_json)
    fn batch_write_updates(&self, writes: Vec<(DocumentId, Value, String)>) -> Result<()> {
        if writes.is_empty() {
            return Ok(());
        }

        let mut storage = self.storage.write();
        for (doc_id, tombstone, updated_json) in writes {
            let tombstone_json = serde_json::to_string(&tombstone)?;
            storage.write_data(tombstone_json.as_bytes())?;
            storage.write_document_raw(&self.name, &doc_id, updated_json.as_bytes())?;
        }
        Ok(())
    }

    /// Distinct values for a field
    /// FIX #19: Now supports nested fields via get_nested_value (e.g., "address.city")
    ///
    /// MEMORY FIX: Uses streaming document loading instead of bulk load.
    /// Previously used scan_documents_via_catalog() which loaded ALL documents
    /// into memory at once - causing OOM on large collections (e.g., 21GB emails).
    /// Now uses collect_doc_ids + streaming read - only IDs in memory, docs loaded one by one.
    pub fn distinct(&self, field: &str, query_json: &Value) -> Result<Vec<Value>> {
        self.check_not_closed()?;
        use crate::value_utils::canonical_json_string;

        // Handle _id query optimization
        if let Some(doc_id) = Self::extract_id_query(query_json) {
            if let Some(doc) = self.read_document_by_id(&doc_id)? {
                let doc_json_str = serde_json::to_string(&doc)?;
                let document = Document::from_json(&doc_json_str)?;
                // Use Document::get_all for MongoDB-style array traversal
                let values: Vec<Value> = document.get_all(field).into_iter().cloned().collect();
                return Ok(values);
            }
            return Ok(Vec::new());
        }

        // INDEX-BASED OPTIMIZATION: If query is empty {} and there's an index on the field,
        // use the index to get distinct values without loading any documents.
        // This is O(index_size) instead of O(all_documents) - huge speedup for large collections.
        if query_json == &serde_json::json!({}) {
            if let Some(distinct_values) = self.try_index_based_distinct(field)? {
                log_debug!(
                    "distinct: used index for field '{}', found {} unique values",
                    field,
                    distinct_values.len()
                );
                return Ok(distinct_values);
            }
        }

        // STREAMING FALLBACK: When no index is available or query has filters
        // Collect only document IDs first (small: ~8-32 bytes each)
        // Then stream-load documents one by one - never bulk load all docs into memory
        let (doc_ids, _) = self
            .collect_doc_ids_with_options(query_json, None, None, false, 0, None, true, 0, None)?;

        // Collect distinct values - stream documents one by one
        // OOM PROTECTION: Use try_reserve for dynamic memory checking (jemalloc-aware)
        let mut seen_values: HashSet<String> = HashSet::new();
        let mut distinct_values = Vec::new();

        // Pre-check: try to reserve for estimated unique values (capped at 100K)
        let estimated_unique = doc_ids.len().min(100_000);
        distinct_values.try_reserve(estimated_unique).map_err(|e| {
            IronBaseError::OutOfMemory(format!(
                "Cannot allocate for ~{} distinct values ({}). \
                Solutions: 1) Use indexed distinct, 2) Add a query filter, 3) Increase system memory.",
                estimated_unique, e
            ))
        })?;

        for doc_id in doc_ids {
            // Load document one at a time - O(1) memory per iteration
            if let Some(doc) = self.read_document_by_id(&doc_id)? {
                // PERF: Direct Value→Document conversion (no serialization)
                // Old: serde_json::to_string(&doc) + Document::from_json() = 2x serialization
                // New: from_value_owned() = 0 serialization, just moves ownership
                let document = Document::from_value_owned(doc)?;

                // Use Document::get_all() for MongoDB-style implicit array traversal
                // This properly handles dot notation with arrays like "items.name"
                for field_value in document.get_all(field) {
                    // Use canonical_json_string for deterministic object key ordering
                    let value_key = canonical_json_string(field_value);

                    if seen_values.insert(value_key) {
                        // Dynamic memory check every 1000 new distinct values
                        if distinct_values.len() % 1000 == 0 && !distinct_values.is_empty() {
                            distinct_values.try_reserve(1000).map_err(|e| {
                                IronBaseError::OutOfMemory(format!(
                                    "Out of memory at {} distinct values ({}). \
                                    Consider using a more restrictive query filter.",
                                    distinct_values.len(),
                                    e
                                ))
                            })?;
                        }
                        distinct_values.push(field_value.clone());
                    }
                }
            }
        }

        Ok(distinct_values)
    }

    /// Try to get distinct values using an index on the field
    ///
    /// Returns Some(values) if an index was found and used, None otherwise.
    /// This is O(index_entries) which is much faster than loading all documents.
    fn try_index_based_distinct(&self, field: &str) -> Result<Option<Vec<Value>>> {
        let indexes = self.indexes.read();

        // Find an index that covers this field (single-field index, not compound)
        // For compound indexes, we'd need more complex logic
        let index_info = indexes.list_indexes_with_compound_info();

        for info in &index_info {
            // Only use single-field indexes for now
            // The field in index name format is: "{collection}_{field}" or just the field directly
            if !info.is_compound && info.prefix_field == field {
                // Found a matching index!
                if let Some(btree) = indexes.get_btree_index(&info.index_name) {
                    // Get all entries from the index
                    let entries = btree.get_all_entries();

                    // Extract unique keys (the B+ tree already has them in order)
                    // But we need to deduplicate because non-unique indexes have duplicates
                    let mut seen: HashSet<IndexKey> = HashSet::new();
                    let mut distinct_values = Vec::new();

                    for (key, _doc_id) in entries {
                        // Skip Null keys (documents without this field)
                        if matches!(key, IndexKey::Null) {
                            continue;
                        }
                        if seen.insert(key.clone()) {
                            distinct_values.push(key.to_value());
                        }
                    }

                    return Ok(Some(distinct_values));
                }
            }
        }

        // No suitable index found
        Ok(None)
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
        // Check if the hinted index is compound
        let is_compound = {
            let indexes = self.indexes.read();
            indexes
                .get_btree_index(index_name)
                .map(|idx| idx.metadata.is_compound())
                .unwrap_or(false)
        };

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
                    is_compound,
                });
            }
        }

        Err(IronBaseError::IndexError(format!(
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

    // ========== INDEX HELPER FUNCTIONS FOR UPDATE/DELETE ==========

    /// Remove a document from all indexes
    /// Used during update and delete operations
    ///
    /// FIX #19: Refactored to use IndexManager.remove_document_from_indexes()
    /// which properly handles compound indexes.
    fn remove_from_indexes(&self, doc: &Document) -> Result<()> {
        let mut indexes = self.indexes.write();
        let id_index_name = format!("{}_id", self.name);

        // Remove from _id index (handled separately due to DocumentId type)
        if let Some(id_index) = indexes.get_btree_index_mut(&id_index_name) {
            let id_key = match &doc.id {
                DocumentId::Int(i) => IndexKey::Int(*i),
                DocumentId::String(s) => IndexKey::String(s.clone()),
                DocumentId::ObjectId(oid) => IndexKey::String(oid.clone()),
            };
            id_index.delete(&id_key, &doc.id)?;
        }

        // Remove from all other indexes - delegate to IndexManager
        let doc_value =
            serde_json::to_value(doc).map_err(|e| IronBaseError::Serialization(e.to_string()))?;
        indexes.remove_document_from_indexes(&doc_value, &doc.id, Some(&id_index_name))?;

        Ok(())
    }

    /// Add a document to all indexes (with unique constraint checking)
    /// Used during update operations after removing old values
    ///
    /// FIX #19: Refactored to use IndexManager.add_document_to_indexes()
    /// which properly handles compound indexes.
    fn add_to_indexes(&self, doc: &Document) -> Result<()> {
        let mut indexes = self.indexes.write();
        let id_index_name = format!("{}_id", self.name);

        // Add to _id index (handled separately due to DocumentId type)
        if let Some(id_index) = indexes.get_btree_index_mut(&id_index_name) {
            let id_key = match &doc.id {
                DocumentId::Int(i) => IndexKey::Int(*i),
                DocumentId::String(s) => IndexKey::String(s.clone()),
                DocumentId::ObjectId(oid) => IndexKey::String(oid.clone()),
            };
            id_index.insert(id_key, doc.id.clone())?;
        }

        // Add to all other indexes - delegate to IndexManager
        let doc_value =
            serde_json::to_value(doc).map_err(|e| IronBaseError::Serialization(e.to_string()))?;
        indexes.add_document_to_indexes(&doc_value, &doc.id, Some(&id_index_name))?;

        Ok(())
    }

    /// 🚀 OPTIMIZED: Batch update indexes using HashMap + rebuild
    ///
    /// **OLD APPROACH (O(n * k)):**
    /// - For each of k updates: delete() + insert() each O(n) due to Vec::insert
    /// - 20K updates on 100K index = ~8 billion element moves!
    ///
    /// **NEW APPROACH (O(n log n + k)):**
    /// - Collect all updates as (old_key, new_key) tuples: O(k)
    /// - Apply batch updates via HashMap + sorted rebuild: O(n log n)
    /// - Total: O(n log n + k) instead of O(n * k)
    fn batch_update_indexes(&self, updates: &[(Document, Document)]) -> Result<()> {
        if updates.is_empty() {
            return Ok(());
        }

        let mut indexes = self.indexes.write();
        let id_index_name = format!("{}_id", self.name);
        let index_names: Vec<String> = indexes.list_indexes();

        // --- _id INDEX: Use apply_batch_updates ---
        {
            let id_updates: Vec<(IndexKey, DocumentId, IndexKey, DocumentId)> = updates
                .iter()
                .map(|(original_doc, updated_doc)| {
                    let old_key = match &original_doc.id {
                        DocumentId::Int(i) => IndexKey::Int(*i),
                        DocumentId::String(s) => IndexKey::String(s.clone()),
                        DocumentId::ObjectId(oid) => IndexKey::String(oid.clone()),
                    };
                    let new_key = match &updated_doc.id {
                        DocumentId::Int(i) => IndexKey::Int(*i),
                        DocumentId::String(s) => IndexKey::String(s.clone()),
                        DocumentId::ObjectId(oid) => IndexKey::String(oid.clone()),
                    };
                    (
                        old_key,
                        original_doc.id.clone(),
                        new_key,
                        updated_doc.id.clone(),
                    )
                })
                .collect();

            if let Some(id_index) = indexes.get_btree_index_mut(&id_index_name) {
                id_index.apply_batch_updates(id_updates)?;
            }
        }

        // --- OTHER INDEXES: Use extract_key for proper compound index support ---
        for index_name in &index_names {
            if index_name == &id_index_name {
                continue;
            }

            // Extract keys using index's extract_key method (handles compound indexes)
            // Also handle cases where field is added/removed (not just changed)
            let mut field_updates: Vec<(IndexKey, DocumentId, IndexKey, DocumentId)> = Vec::new();
            let mut removals: Vec<(IndexKey, DocumentId)> = Vec::new();
            let mut additions: Vec<(IndexKey, DocumentId)> = Vec::new();

            if let Some(idx) = indexes.get_btree_index(index_name) {
                let is_unique = idx.metadata.unique;

                for (original_doc, updated_doc) in updates {
                    // Convert Document to Value for extract_key
                    let old_doc_value =
                        serde_json::to_value(original_doc).unwrap_or(serde_json::Value::Null);
                    let new_doc_value =
                        serde_json::to_value(updated_doc).unwrap_or(serde_json::Value::Null);
                    let old_key = idx.extract_key(&old_doc_value);
                    let new_key = idx.extract_key(&new_doc_value);

                    // Skip if keys are identical
                    if old_key == new_key {
                        continue;
                    }

                    let old_is_null = crate::index::IndexManager::is_key_all_null(&old_key);
                    let new_is_null = crate::index::IndexManager::is_key_all_null(&new_key);

                    // Handle all cases:
                    // 1. Both have values (update): use batch update
                    // 2. Old has value, new is null (removed): remove from index
                    // 3. Old is null, new has value (added): add to index
                    // 4. Both null: skip (no change)

                    match (old_is_null, new_is_null) {
                        (false, false) => {
                            // Both have values - update
                            field_updates.push((
                                old_key,
                                original_doc.id.clone(),
                                new_key,
                                updated_doc.id.clone(),
                            ));
                        }
                        (false, true) => {
                            // Value removed - delete from index (if was indexed)
                            if is_unique {
                                // Unique indexes include null, so we need to update
                                field_updates.push((
                                    old_key,
                                    original_doc.id.clone(),
                                    new_key,
                                    updated_doc.id.clone(),
                                ));
                            } else {
                                // Non-unique: just remove old entry
                                removals.push((old_key, original_doc.id.clone()));
                            }
                        }
                        (true, false) => {
                            // Value added - add to index
                            if is_unique {
                                // Unique indexes include null, so we need to update
                                field_updates.push((
                                    old_key,
                                    original_doc.id.clone(),
                                    new_key,
                                    updated_doc.id.clone(),
                                ));
                            } else {
                                // Non-unique: just add new entry
                                additions.push((new_key, updated_doc.id.clone()));
                            }
                        }
                        (true, true) => {
                            // Both null - no action needed for non-unique
                            // For unique indexes, both null means same value
                        }
                    }
                }
            }

            // Apply updates
            if let Some(index) = indexes.get_btree_index_mut(index_name) {
                // Apply batch updates for changed values
                if !field_updates.is_empty() {
                    index.apply_batch_updates(field_updates)?;
                }
                // Apply removals (log warning if delete fails unexpectedly)
                for (key, doc_id) in removals {
                    if let Err(e) = index.delete(&key, &doc_id) {
                        log_warn!(
                            "B+tree index '{}' delete failed for doc {:?}: {:?}",
                            index_name,
                            doc_id,
                            e
                        );
                    }
                }
                // Apply additions
                for (key, doc_id) in additions {
                    index.insert(key, doc_id)?;
                }
            }
        }

        // --- FULLTEXT INDEXES: Update for each document ---
        // FIX: Fulltext indexes were not being updated during batch updates!
        let fulltext_index_names: Vec<String> = indexes
            .list_fulltext_indexes()
            .iter()
            .map(|idx| idx.name.clone())
            .collect();

        for fts_name in fulltext_index_names {
            if let Some(fts_index) = indexes.get_fulltext_index_mut(&fts_name) {
                let fts_field = fts_index.field.clone();
                for (original_doc, updated_doc) in updates {
                    // Only update if the indexed field was changed
                    let old_value = original_doc.get(&fts_field);
                    let new_value = updated_doc.get(&fts_field);

                    // Check if the field value actually changed
                    if old_value != new_value {
                        // Update the fulltext index for this document
                        // First remove old entry (handles type changes too)
                        if old_value.is_some() {
                            let _ = fts_index.remove(&original_doc.id);
                        }
                        // Then add new entry if it's a string
                        if let Some(new_val) = new_value {
                            if let Some(text) = new_val.as_str() {
                                // Log error but continue - document already persisted
                                if let Err(e) = fts_index.update(&updated_doc.id, text) {
                                    log_error!(
                                        "Failed to update fulltext index '{}' for doc {:?}: {:?}",
                                        fts_name,
                                        updated_doc.id,
                                        e
                                    );
                                }
                            }
                            // If new value is not a string, entry already removed above
                        }
                    }
                }
            }
        }

        // --- FUZZY INDEXES: Update for each document ---
        // Get fuzzy index info before mutable borrow
        let fuzzy_index_info: Vec<(String, String)> = indexes
            .list_fuzzy_indexes()
            .iter()
            .map(|idx| (idx.metadata.name.clone(), idx.metadata.field.clone()))
            .collect();

        for (fuzzy_name, fuzzy_field) in fuzzy_index_info {
            for (original_doc, updated_doc) in updates {
                let old_value = original_doc.get(&fuzzy_field);
                let new_value = updated_doc.get(&fuzzy_field);

                // Only update if the indexed field was changed
                if old_value != new_value {
                    if let Some(fuzzy_index) = indexes.get_fuzzy_index_mut(&fuzzy_name) {
                        // Remove old entry if it was a string
                        if let Some(old_val) = old_value {
                            if let Some(old_text) = old_val.as_str() {
                                fuzzy_index.remove_value(old_text, &original_doc.id);
                            }
                        }
                        // Add new entry if it's a string
                        if let Some(new_val) = new_value {
                            if let Some(new_text) = new_val.as_str() {
                                fuzzy_index.insert(new_text, updated_doc.id.clone());
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Batch add multiple documents to all indexes
    /// Single lock acquisition for performance - used by insert_many
    ///
    /// FIX #19: Refactored to use IndexManager.add_document_to_indexes()
    /// which properly handles compound indexes.
    fn batch_add_to_indexes(&self, docs: &[Document]) -> Result<()> {
        if docs.is_empty() {
            return Ok(());
        }

        let mut indexes = self.indexes.write();
        let id_index_name = format!("{}_id", self.name);

        for doc in docs {
            // Add to _id index (handled separately due to DocumentId type)
            if let Some(id_index) = indexes.get_btree_index_mut(&id_index_name) {
                let id_key = match &doc.id {
                    DocumentId::Int(i) => IndexKey::Int(*i),
                    DocumentId::String(s) => IndexKey::String(s.clone()),
                    DocumentId::ObjectId(oid) => IndexKey::String(oid.clone()),
                };
                id_index.insert(id_key, doc.id.clone())?;
            }

            // Add to all other indexes - delegate to IndexManager
            let doc_value = serde_json::to_value(doc)
                .map_err(|e| IronBaseError::Serialization(e.to_string()))?;
            indexes.add_document_to_indexes(&doc_value, &doc.id, Some(&id_index_name))?;
        }

        Ok(())
    }

    /// Check if a document would violate unique constraints
    /// exclude_id: Optional document ID to exclude from check (for updates)
    ///
    /// FIX #19: Refactored to use IndexManager.check_unique_constraints()
    /// which properly handles compound indexes.
    fn check_index_constraints(
        &self,
        doc: &Document,
        exclude_id: Option<&DocumentId>,
    ) -> Result<()> {
        let indexes = self.indexes.read();
        let id_index_name = format!("{}_id", self.name);

        // Convert Document to Value for IndexManager
        let doc_value =
            serde_json::to_value(doc).map_err(|e| IronBaseError::Serialization(e.to_string()))?;

        // Delegate to IndexManager - handles compound indexes correctly
        indexes.check_unique_constraints(&doc_value, exclude_id, Some(&id_index_name))
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
        self.check_not_closed()?;
        use crate::aggregation::Pipeline;

        // Parse pipeline
        let mut pipeline = Pipeline::from_json(pipeline_json)?;

        // INDEX OPTIMIZATION (2024-12):
        // If the first stage is $match, extract it and use indexed find_streaming()
        // instead of scanning all documents.
        //
        // Benefit: 10-1000x speedup for selective aggregations
        // Example: $match on indexed field reduces 49K docs to 1K before $group
        let match_query = pipeline
            .extract_leading_match()
            .unwrap_or_else(|| serde_json::json!({}));

        log_debug!(
            "aggregate: using query {:?} for initial document selection",
            match_query
        );

        // STREAMING OPTIMIZATION (2024-12):
        // Use cursor-based document loading to avoid loading all docs into memory.
        //
        // Memory comparison for 650K emails grouped by sender:
        // - OLD (find + execute): 650K × 800 bytes = ~500MB loaded upfront
        // - NEW (streaming): ~0MB for docs (processed one at a time) + ~640KB for groups
        //
        // The streaming pipeline:
        // 1. Uses find_streaming() with $match query (uses index if available!)
        // 2. Streams docs through remaining stages ($group, etc.)
        // 3. Materializes for final stages ($sort, $limit, etc.)

        // Get streaming cursor - uses index if match_query has indexed field
        let mut cursor = self.find_streaming(&match_query)?;

        // Create iterator that yields Result<Value> from cursor
        let doc_iter = std::iter::from_fn(move || match cursor.next() {
            Ok(Some(doc)) => Some(Ok(doc)),
            Ok(None) => None,
            Err(e) => Some(Err(e)),
        });

        // Execute remaining pipeline stages with streaming
        pipeline.execute_streaming(doc_iter)
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
            .ok_or_else(|| IronBaseError::CollectionNotFound(self.name.clone()))?;

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
            let id_value = old_doc.get("_id").ok_or(IronBaseError::DocumentNotFound)?;

            let doc_id = match id_value {
                Value::Number(n) if n.is_i64() => DocumentId::Int(n.as_i64().unwrap()),
                Value::Number(n) if n.is_u64() => {
                    let u = n.as_u64().unwrap();
                    if u > i64::MAX as u64 {
                        return Err(IronBaseError::Serialization(
                            "_id value too large for i64".to_string(),
                        ));
                    }
                    DocumentId::Int(u as i64)
                }
                Value::String(s) => DocumentId::String(s.clone()),
                _ => return Err(IronBaseError::Serialization("Invalid _id type".to_string())),
            };

            // Ensure new_doc has _id and _collection fields
            let new_doc_with_meta = if let Value::Object(mut map) = new_doc {
                map.insert("_id".to_string(), id_value.clone());
                map.insert("_collection".to_string(), Value::String(self.name.clone()));
                Value::Object(map)
            } else {
                return Err(IronBaseError::Serialization(
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
                    // FIX #19: Use get_nested_value to support dot notation (e.g., "profile.code")
                    let old_value = crate::value_utils::get_nested_value(&old_doc, field_name);
                    let new_value =
                        crate::value_utils::get_nested_value(&new_doc_for_tracking, field_name);

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
            let id_value = old_doc.get("_id").ok_or(IronBaseError::DocumentNotFound)?;

            let doc_id = match id_value {
                Value::Number(n) if n.is_i64() => DocumentId::Int(n.as_i64().unwrap()),
                Value::Number(n) if n.is_u64() => {
                    let u = n.as_u64().unwrap();
                    if u > i64::MAX as u64 {
                        return Err(IronBaseError::Serialization(
                            "_id value too large for i64".to_string(),
                        ));
                    }
                    DocumentId::Int(u as i64)
                }
                Value::String(s) => DocumentId::String(s.clone()),
                _ => return Err(IronBaseError::Serialization("Invalid _id type".to_string())),
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
                    // FIX #19: Use get_nested_value to support dot notation (e.g., "profile.code")
                    if let Some(old_val) =
                        crate::value_utils::get_nested_value(&old_doc, field_name)
                    {
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
        // PERF: Use read lock + read_data_at() for concurrent reads (pread-based)
        // This is ACID-safe because:
        // 1. We only read catalog and data - no modifications
        // 2. read_data_at() uses pread() which doesn't change file position
        // 3. Allows parallel document reads from multiple threads
        let storage = self.storage.read();
        let meta = storage
            .get_collection_meta(&self.name)
            .ok_or_else(|| IronBaseError::CollectionNotFound(self.name.clone()))?;

        log_trace!(
            "read_document_by_id({:?}) - catalog has {} entries",
            doc_id,
            meta.document_catalog.len()
        );

        // O(1) lookup in document_catalog (direct DocumentId lookup - no serialization!)
        if let Some(&offset) = meta.document_catalog.get(doc_id) {
            log_trace!("Found doc_id {:?} at offset {}", doc_id, offset);
            // Use read_data_at() - positioned read without changing file position
            let doc_bytes = storage.read_data_at(offset)?;
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
    ///
    /// When the `parallel` feature is enabled, JSON deserialization is parallelized
    /// across CPU cores for significantly faster processing of large collections.
    ///
    /// WARNING: This loads ALL documents into memory at once!
    /// For large collections, use collect_doc_ids + read_document_by_id streaming instead.
    ///
    /// NOTE: Currently unused after streaming refactor to prevent OOM, kept for potential use.
    #[allow(dead_code)]
    fn scan_documents_via_catalog(&self) -> Result<HashMap<DocumentId, Value>> {
        let mut storage = self.storage.write();

        // OPTIMIZATION: Clone only offsets, not the entire HashMap structure
        // Vec<(DocId, u64)> is simpler and has less overhead than HashMap clone
        let offsets: Vec<(DocumentId, u64)> = {
            let meta = storage
                .get_collection_meta(&self.name)
                .ok_or_else(|| IronBaseError::CollectionNotFound(self.name.clone()))?;
            log_debug!(
                "Collection '{}' has {} documents in catalog",
                self.name,
                meta.document_catalog.len()
            );
            meta.document_catalog
                .iter()
                .map(|(id, &offset)| (id.clone(), offset))
                .collect()
        };

        // OPTIMIZATION: Sort by offset for sequential disk reads
        // This eliminates random disk seeks (5-10ms each on spinning disks)
        let mut sorted_offsets = offsets;
        sorted_offsets.sort_by_key(|(_, offset)| *offset);

        // PHASE 1: Sequential disk I/O (disk-bound, must be sequential for optimal read)
        // Read raw bytes in offset order to minimize disk seeks
        let raw_docs: Vec<(DocumentId, Vec<u8>)> = sorted_offsets
            .into_iter()
            .filter_map(|(doc_id, offset)| {
                storage.read_data(offset).ok().map(|bytes| (doc_id, bytes))
            })
            .collect();

        // Release storage lock before CPU-intensive deserialization
        drop(storage);

        // PHASE 2: JSON deserialization (CPU-bound, parallelized when feature enabled)
        #[cfg(feature = "parallel")]
        let docs_by_id: HashMap<DocumentId, Value> = {
            crate::parallel::parallel_deserialize(raw_docs)
                .into_iter()
                .filter(|(_, doc)| {
                    // Skip tombstones (deleted documents)
                    !doc.get("_tombstone")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                })
                .collect()
        };

        #[cfg(not(feature = "parallel"))]
        let docs_by_id: HashMap<DocumentId, Value> = raw_docs
            .into_iter()
            .filter_map(|(doc_id, bytes)| {
                serde_json::from_slice::<Value>(&bytes)
                    .ok()
                    .map(|doc| (doc_id, doc))
            })
            .filter(|(_, doc)| {
                // Skip tombstones (deleted documents)
                !doc.get("_tombstone")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
            })
            .collect();

        Ok(docs_by_id)
    }

    /// 🚀 OPTIMIZED: Batch read documents by IDs in a single lock acquisition
    /// Instead of N lock acquisitions for N documents, we only acquire 1 lock
    fn batch_read_documents_by_ids(
        &self,
        doc_ids: &[DocumentId],
    ) -> Result<HashMap<DocumentId, Value>> {
        let mut storage = self.storage.write();
        let meta = storage
            .get_collection_meta(&self.name)
            .ok_or_else(|| IronBaseError::CollectionNotFound(self.name.clone()))?;

        // Clone only the offsets we need
        let offsets: Vec<(DocumentId, u64)> = doc_ids
            .iter()
            .filter_map(|id| {
                meta.document_catalog
                    .get(id)
                    .map(|&offset| (id.clone(), offset))
            })
            .collect();

        // Release the borrow on meta before reading (meta is already dropped after collect)

        let mut docs_by_id: HashMap<DocumentId, Value> = HashMap::with_capacity(offsets.len());

        for (doc_id, offset) in offsets {
            match storage.read_data(offset) {
                Ok(doc_bytes) => {
                    if let Ok(doc) = serde_json::from_slice::<Value>(&doc_bytes) {
                        // Skip tombstones
                        if !doc
                            .get("_tombstone")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false)
                        {
                            docs_by_id.insert(doc_id, doc);
                        }
                    }
                }
                Err(_) => continue,
            }
        }

        Ok(docs_by_id)
    }

    /// Scan documents in batches for memory-efficient processing
    ///
    /// This is designed for operations that need to process ALL documents
    /// but want to control memory usage (e.g., fulltext index building).
    ///
    /// # Arguments
    /// * `batch_size` - Number of documents to load per batch
    /// * `callback` - Called for each batch with (batch_number, documents)
    ///
    /// # Returns
    /// Total number of documents processed
    fn scan_documents_in_batches<F>(&self, batch_size: usize, mut callback: F) -> Result<usize>
    where
        F: FnMut(usize, HashMap<DocumentId, Value>) -> Result<()>,
    {
        // Step 1: Get all document offsets (hold lock briefly)
        let sorted_offsets: Vec<(DocumentId, u64)> = {
            let storage = self.storage.read();
            let meta = storage
                .get_collection_meta(&self.name)
                .ok_or_else(|| IronBaseError::CollectionNotFound(self.name.clone()))?;
            let mut offsets: Vec<(DocumentId, u64)> = meta
                .document_catalog
                .iter()
                .map(|(id, &offset)| (id.clone(), offset))
                .collect();
            // Sort by offset for sequential disk reads
            offsets.sort_by_key(|(_, offset)| *offset);
            offsets
        }; // storage lock released here

        let total_docs = sorted_offsets.len();
        let mut processed = 0;
        let mut batch_num = 0;

        // Step 2: Process in batches, releasing storage lock between batches
        // CRITICAL FIX: Use read lock + read_data_at (positioned read) instead of
        // write lock + read_data. This allows concurrent reads during index building
        // and prevents deadlock with concurrent batch inserts.
        for chunk in sorted_offsets.chunks(batch_size) {
            // Read raw bytes from disk using positioned read (no file position change)
            let raw_batch: Vec<(DocumentId, Vec<u8>)> = {
                let storage = self.storage.read(); // READ lock instead of WRITE lock
                chunk
                    .iter()
                    .filter_map(|(doc_id, offset)| {
                        storage
                            .read_data_at(*offset)  // pread - no position change
                            .ok()
                            .map(|bytes| (doc_id.clone(), bytes))
                    })
                    .collect()
            }; // storage lock released here

            // Deserialize and filter (no lock needed)
            let mut batch_docs: HashMap<DocumentId, Value> =
                HashMap::with_capacity(raw_batch.len());
            for (doc_id, bytes) in raw_batch {
                if let Ok(doc) = serde_json::from_slice::<Value>(&bytes) {
                    // Skip tombstones
                    if !doc
                        .get("_tombstone")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                    {
                        batch_docs.insert(doc_id, doc);
                    }
                }
            }

            let batch_count = batch_docs.len();
            callback(batch_num, batch_docs)?;
            processed += batch_count;
            batch_num += 1;

            log_debug!(
                "scan_documents_in_batches: processed batch {} ({}/{} docs)",
                batch_num,
                processed,
                total_docs
            );
        }

        Ok(processed)
    }

    /// 🚀 OPTIMIZED: Scan documents with early termination support
    /// Unlike scan_documents_via_catalog which loads ALL documents,
    /// this method stops early when skip + limit documents are found.
    /// This is critical for performance on large collections with pagination.
    fn scan_documents_with_early_termination(
        &self,
        parsed_query: &Query,
        skip: usize,
        limit: Option<usize>,
    ) -> Result<Vec<DocumentId>> {
        let mut storage = self.storage.write();

        // MEMORY OPTIMIZATION: Don't clone entire catalog HashMap!
        // Only extract what we need: Vec<(DocumentId, u64)> for offsets
        let (catalog_entries, catalog_len, live_count) = {
            let meta = storage
                .get_collection_meta(&self.name)
                .ok_or_else(|| IronBaseError::CollectionNotFound(self.name.clone()))?;
            log_debug!(
                "scan_documents_with_early_termination: collection '{}' has {} docs, skip={}, limit={:?}",
                self.name,
                meta.document_catalog.len(),
                skip,
                limit
            );
            let entries: Vec<(DocumentId, u64)> = meta
                .document_catalog
                .iter()
                .map(|(id, &offset)| (id.clone(), offset))
                .collect();
            let len = entries.len();
            let live = storage.get_live_count(&self.name).unwrap_or(0);
            (entries, len, live)
        };

        // 🚀 FAST PATH: Empty query - skip directly from catalog without disk I/O
        // This is critical for pagination performance on large collections
        // NOTE: Only use fast path if there are no tombstones (live_count == catalog.len())
        // Otherwise tombstones would be incorrectly counted in skip/limit calculations
        if parsed_query.is_match_all() && live_count == catalog_len as u64 {
            // No tombstones - safe to use fast path
            let effective_limit = limit.unwrap_or(usize::MAX);

            // OOM PROTECTION: Check memory before allocating for sorted keys
            let estimated_size = catalog_len.saturating_sub(skip).min(effective_limit);
            let mut pre_check = Vec::<DocumentId>::new();
            pre_check.try_reserve(estimated_size).map_err(|e| {
                IronBaseError::OutOfMemory(format!(
                    "Cannot allocate for {} document IDs ({}). \
                    Solutions: 1) Add 'limit' to your query, 2) Use an index, 3) Increase system memory.",
                    estimated_size, e
                ))
            })?;
            drop(pre_check); // Release the pre-check allocation

            // Sort keys for deterministic pagination across process restarts
            // HashMap iteration order is non-deterministic due to ASLR hash seeds
            let mut sorted_keys: Vec<DocumentId> =
                catalog_entries.into_iter().map(|(id, _)| id).collect();
            sorted_keys.sort();

            let doc_ids: Vec<DocumentId> = sorted_keys
                .into_iter()
                .skip(skip)
                .take(if effective_limit == 0 {
                    usize::MAX
                } else {
                    effective_limit
                })
                .collect();
            log_debug!(
                "scan_documents_with_early_termination: FAST PATH (empty query, no tombstones) - skip={}, limit={:?}, returning {} docs",
                skip,
                limit,
                doc_ids.len()
            );
            return Ok(doc_ids);
        }

        // Tombstones exist or query needs filtering - use slow path
        if parsed_query.is_match_all() {
            log_debug!(
                "scan_documents_with_early_termination: tombstones detected (live={}, catalog={}), using slow path",
                live_count,
                catalog_len
            );
        }

        let mut doc_ids = Vec::new();
        let mut skipped = 0usize;

        // OOM PROTECTION: Try to reserve for worst case (all docs match)
        // This catches OOM early before spending time on disk I/O
        let estimated_matches = limit.unwrap_or(catalog_len);
        doc_ids.try_reserve(estimated_matches.min(catalog_len)).map_err(|e| {
            IronBaseError::OutOfMemory(format!(
                "Cannot allocate for {} document IDs ({}). \
                Solutions: 1) Add 'limit' to your query, 2) Use an index, 3) Increase system memory.",
                estimated_matches.min(catalog_len), e
            ))
        })?;

        // BUG #1 FIX: Sort entries for deterministic pagination
        // HashMap iteration order is non-deterministic due to ASLR hash seeds
        // This ensures consistent results even when tombstones are present
        let mut sorted_entries = catalog_entries;
        sorted_entries.sort_by(|(a, _), (b, _)| a.cmp(b));

        // Iterate over sorted entries with early termination (for filtered queries)
        for (doc_id, offset) in sorted_entries {
            // Check if we've collected enough documents
            // MongoDB compatibility: limit(0) means "no limit"
            if let Some(limit_count) = limit {
                if limit_count > 0 && doc_ids.len() >= limit_count {
                    log_debug!(
                        "Early termination: collected {} docs (limit={})",
                        doc_ids.len(),
                        limit_count
                    );
                    break;
                }
            }

            // Read document from storage
            let doc_bytes = match storage.read_data(offset) {
                Ok(bytes) => bytes,
                Err(_) => continue, // Skip corrupted entries
            };

            // Parse document
            let doc: Value = match serde_json::from_slice(&doc_bytes) {
                Ok(d) => d,
                Err(_) => continue, // Skip corrupted JSON
            };

            // Skip tombstones
            if doc
                .get("_tombstone")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                continue;
            }

            // Apply query filter
            let document = Document::from_value_owned(doc)?;
            if parsed_query.matches(&document)? {
                // Apply skip
                if skipped < skip {
                    skipped += 1;
                    continue;
                }
                doc_ids.push(doc_id);
            }
        }

        log_debug!(
            "scan_documents_with_early_termination: returning {} doc IDs",
            doc_ids.len()
        );
        Ok(doc_ids)
    }

    fn collect_doc_ids(&self, query_json: &Value) -> Result<Vec<DocumentId>> {
        let (ids, _) = self
            .collect_doc_ids_with_options(query_json, None, None, false, 0, None, true, 0, None)?;
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
        original_skip: usize,          // For index-based sort: skip count
        original_limit: Option<usize>, // For index-based sort: enables early termination
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
            // FIX #20: Use compound-index-aware query planning
            // This ensures compound indexes are used for prefix field queries with range scans
            let indexes = self.indexes.read();
            let index_fields = indexes.list_indexes_with_compound_info();

            // Query planning with compound-index-aware field matching
            // NOTE: FIX #21 stale index workaround removed - shared IndexManagers fix this
            let plan_opt = QueryPlanner::analyze_query_with_fields(query_json, &index_fields)
                .map(|(_, plan)| plan);

            drop(indexes);
            plan_opt
        };

        let (doc_ids_vec, used_sort) = if let Some(plan) = plan {
            self.collect_doc_ids_from_plan(&parsed_query, plan, sort_field, sort_desc, skip, limit)?
        } else if let Some(sf) = sort_field {
            // 🚀 FIX #22: Use index for sort-only queries (no filter)
            // When query is empty but we have a sort field with an index,
            // use the B+ tree for ordered iteration instead of in-memory sort.
            // This is critical for large collections (e.g., 49,200 emails sorted by date).
            //
            // IMPORTANT: Only use this optimization when query is empty!
            // If there's a filter but no index for it, we must still scan and filter.
            let query_is_empty = Self::query_matches_all(query_json);

            if query_is_empty {
                // FIX: For index-based sorting, use original skip/limit for early termination.
                // This is critical for performance: when user specifies skip=10, limit=5 + sort,
                // the index can return only the correct 5 documents instead of loading all.
                // No artificial limit - try_reserve will catch OOM if needed.
                if let Some((_index_name, doc_ids)) =
                    self.try_index_sorted_scan(sf, sort_desc, original_skip, original_limit)?
                {
                    // Index-based sort optimization active - skip/limit already applied
                    (doc_ids, true)
                } else {
                    // No suitable index found, fall back to collection scan
                    let doc_ids =
                        self.scan_documents_with_early_termination(&parsed_query, skip, limit)?;
                    (doc_ids, false)
                }
            } else {
                // Query has filter but no index plan - must scan all documents
                let doc_ids =
                    self.scan_documents_with_early_termination(&parsed_query, skip, limit)?;
                (doc_ids, false)
            }
        } else {
            // 🚀 FIX: Use early termination scan instead of loading all documents
            // This is critical for performance on large collections with pagination.
            // Previously scan_documents_via_catalog() loaded ALL documents first,
            // making limit=1 take the same time as limit=50 on large collections.
            let doc_ids = self.scan_documents_with_early_termination(&parsed_query, skip, limit)?;
            (doc_ids, false)
        };

        if let Some(hash) = cache_hash {
            self.query_cache
                .insert(&self.name, hash, doc_ids_vec.clone());
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
                    is_compound,
                    ..
                } => {
                    if let Some(index) = indexes.get_btree_index(index_name) {
                        if is_compound {
                            // Compound index prefix query: use range scan with compound bounds
                            let (start, end) = index.build_prefix_range(key.clone());
                            index.range_scan(&start, &end, true, true)
                        } else {
                            // Single-field index: point lookup
                            index.range_scan(key, key, true, true)
                        }
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
                QueryPlan::SparseIndexScan { ref index_name, .. } => {
                    // Sparse index scan: return ALL doc_ids in the index
                    // Since sparse indexes only contain documents where the field exists,
                    // this effectively returns all documents matching $exists: true
                    if let Some(index) = indexes.get_btree_index(index_name) {
                        // Full range scan from minimum to maximum key
                        let start = IndexKey::Null;
                        let end = IndexKey::MaxKey;
                        index.range_scan(&start, &end, true, true)
                    } else {
                        vec![]
                    }
                }
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

                if parsed_query.matches(&document)? {
                    if skipped < skip {
                        skipped += 1;
                        continue;
                    }

                    results.push(doc_id.clone());
                    // MongoDB compatibility: limit(0) means "no limit"
                    if let Some(limit_count) = limit {
                        if limit_count > 0 && results.len() >= limit_count {
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

    /// Try to use an index for sorted iteration (for sort-only queries without filter)
    ///
    /// When a query is empty but has a sort field with an index, this method uses
    /// the B+ tree for ordered iteration instead of loading all documents and
    /// sorting in-memory.
    ///
    /// Returns `Some((index_name, doc_ids))` if an index was used, `None` otherwise.
    fn try_index_sorted_scan(
        &self,
        sort_field: &str,
        sort_desc: bool,
        skip: usize,
        limit: Option<usize>,
    ) -> Result<Option<(String, Vec<DocumentId>)>> {
        // Find an index for the sort field
        let indexes = self.indexes.read();
        let index_infos = indexes.list_indexes_with_compound_info();

        // Look for a single-field index on the sort field
        let matching_index = index_infos
            .iter()
            .find(|info| !info.is_compound && info.prefix_field == sort_field);

        let index_name = match matching_index {
            Some(info) => info.index_name.clone(),
            None => {
                drop(indexes);
                return Ok(None);
            }
        };

        // Get the B+ tree index
        let btree = match indexes.get_btree_index(&index_name) {
            Some(tree) => tree,
            None => {
                drop(indexes);
                return Ok(None);
            }
        };

        // 🚀 FIX #23: Use optimized reverse scan for DESC sort
        // Previously: range_scan() loaded ALL 49,200 doc IDs, then reversed in memory
        // Now: range_scan_reversed_with_limit() traverses from right-to-left with early termination
        let result = if sort_desc {
            // DESC sort: use optimized reverse scan with early termination
            btree.range_scan_reversed_with_limit(
                &crate::index::IndexKey::Null,
                &crate::index::IndexKey::MaxKey,
                skip,
                limit,
            )
        } else {
            // ASC sort: use standard range scan (TODO: add early termination for ASC too)
            let all_doc_ids = btree.range_scan(
                &crate::index::IndexKey::Null,
                &crate::index::IndexKey::MaxKey,
                true,
                true,
            );

            // Apply skip and limit
            match limit {
                Some(lim) if lim > 0 => all_doc_ids.into_iter().skip(skip).take(lim).collect(),
                _ => all_doc_ids.into_iter().skip(skip).collect(),
            }
        };
        drop(indexes);

        Ok(Some((index_name, result)))
    }
}
