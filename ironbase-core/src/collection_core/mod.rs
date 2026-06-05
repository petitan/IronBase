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
use crate::execution::ExecutionContext;
use crate::index::{IndexKey, IndexManager, RangeQueryMode, ScanOrder};
use crate::query::Query;
use crate::query_cache::{QueryCache, QueryHash};
use crate::query_planner::{LogicalOperator, QueryPlan, QueryPlanner};
use crate::storage::{RawStorage, Storage};
use crate::{log_debug, log_error, log_trace, log_warn};

mod aggregate;
mod constraints;
mod context;
mod count;
mod cursor;
mod distinct;
mod index_ops;
mod index_persistence;
mod query_executor;
mod raw_operations;
pub(crate) mod schema;
mod search;
mod topk;
mod tx;
mod update_operators;
mod vector_ops;

pub(crate) use self::context::QueryExecutionContext;
pub(crate) use self::vector_ops::doc_id_to_string;

// Public exports for Top-K algorithm
pub use topk::{topk_select, topk_select_with_skip};

// Public exports for index statistics
pub use index_ops::IndexStatisticsInfo;

// Public exports for Query Executor
pub use query_executor::{
    compare_docs_by_sort, topk_documents, ExecutionMethod, ExecutionStats, QueryOptions,
    QueryResult, SortDirection,
};

pub(crate) use self::constraints::BatchConstraintValidator;
pub(crate) use self::index_persistence::try_load_index_from_file;
pub(crate) use self::index_persistence::{
    build_btree_index_file_path, build_fulltext_index_file_path, build_fuzzy_index_file_path,
    persist_index_to_disk, try_load_fulltext_index_from_file, try_load_fuzzy_index_from_file,
};
use self::schema::CompiledSchema;

// Re-export the sealed RawOperations trait for crate-internal use
pub(crate) use self::raw_operations::RawOperations;

// Re-export prepared operation structs for WAL-first batch mode
pub(crate) use self::raw_operations::InsertOnePrepared;

// Re-export batch-mode prepared structs (WAL ORDERING FIX)
pub(crate) use self::raw_operations::{DeleteOnePreparedBatch, UpdateOnePreparedBatch};

// Re-export _many prepared structs (WAL ORDERING FIX for _many Batch mode)
pub(crate) use self::raw_operations::{DeleteManyPrepared, UpdateManyPrepared};

// Re-export FindCursor for streaming queries
pub use self::cursor::FindCursor;

// ============================================================================
// CONSTANTS
// ============================================================================

// Re-export from central limits module
use crate::limits::QUERY_CACHE_CAPACITY;

// OOM protection: try_reserve() fails fast on allocation pressure

/// Threshold for logging a warning about large document loads
use crate::limits::LARGE_QUERY_WARNING_THRESHOLD;

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
                                                if !index.metadata.multikey {
                                                    let has_array = if index.metadata.is_compound()
                                                    {
                                                        index.metadata.fields.iter().any(|field| {
                                                            crate::value_utils::path_crosses_array(
                                                                &doc, field,
                                                            )
                                                        })
                                                    } else {
                                                        crate::value_utils::path_crosses_array(
                                                            &doc,
                                                            &index.metadata.field,
                                                        )
                                                    };
                                                    if has_array {
                                                        index.metadata.multikey = true;
                                                    }
                                                }
                                                let keys = index.extract_keys(&doc);
                                                let mut seen = HashSet::new();
                                                for key in keys {
                                                    if !seen.insert(key.clone()) {
                                                        continue;
                                                    }
                                                    let is_all_null =
                                                        IndexManager::is_key_all_null(&key);
                                                    if !is_all_null || index.metadata.unique {
                                                        let _ = index.insert(key, doc_id.clone());
                                                        rebuilt_count += 1;
                                                    }
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
                                        // FIX #25: Only skip documents already in the index, not the entire index
                                        // This ensures documents inserted after last flush are indexed on restart
                                        for ft_meta in &persisted_fulltext_indexes {
                                            if let Some(ft_index) =
                                                index_manager.get_fulltext_index_mut(&ft_meta.name)
                                            {
                                                // Skip if this specific document is already indexed
                                                // (loaded from .ftidx file)
                                                if ft_index.contains_doc(&doc_id) {
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
                                                        let pdid_field = ft_index
                                                            .parent_doc_id_field()
                                                            .to_string();
                                                        let parent_doc_id =
                                                            crate::value_utils::get_nested_value(
                                                                &doc,
                                                                &pdid_field,
                                                            )
                                                            .and_then(|v| {
                                                                v.as_str().map(|s| s.to_string())
                                                            });
                                                        let _ =
                                                            if let Some(ref pdid) = parent_doc_id {
                                                                ft_index.insert_with_parent_doc_id(
                                                                    &doc_id, text, pdid,
                                                                )
                                                            } else {
                                                                ft_index.insert(&doc_id, text)
                                                            };
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

    // ========== AUTO-EMBEDDING CONFIG ==========

    /// Set or clear the auto-embedding configuration for this collection.
    ///
    /// When enabled, documents inserted or updated will automatically have
    /// embeddings generated from the source field and stored in the target field.
    pub fn set_auto_embedding_config(
        &self,
        config: Option<crate::storage::AutoEmbeddingConfig>,
    ) -> Result<()> {
        self.check_not_closed()?;
        let mut storage = self.storage.write();
        let meta = storage
            .get_collection_meta_mut(&self.name)
            .ok_or_else(|| IronBaseError::CollectionNotFound(self.name.clone()))?;
        meta.auto_embedding_config = config;
        storage.flush()?;
        Ok(())
    }

    /// Get the auto-embedding configuration for this collection (if any)
    pub fn get_auto_embedding_config(&self) -> Result<Option<crate::storage::AutoEmbeddingConfig>> {
        self.check_not_closed()?;
        let storage = self.storage.read();
        Ok(storage
            .get_collection_meta(&self.name)
            .and_then(|meta| meta.auto_embedding_config.clone()))
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

        // ====================================================================
        // FAST PATH: Direct _id lookup O(1)
        // FIX #5-6: When query is {"_id": value}, skip catalog scanning entirely.
        // Uses normalize_document_id to handle string/int conversion.
        // ====================================================================
        if let Some(doc_id) = Self::extract_id_query(query_json) {
            // Try original ID first
            if let Some(doc) = self.read_document_by_id(&doc_id)? {
                return Ok(vec![doc]);
            }
            // Try normalized version (string "123" → int 123)
            if let Some(normalized) = Self::normalize_document_id(&doc_id) {
                if let Some(doc) = self.read_document_by_id(&normalized)? {
                    return Ok(vec![doc]);
                }
            }
            return Ok(Vec::new());
        }

        // ====================================================================
        // FAST PATH: _id $in query = O(k) lookups (k = number of IDs)
        // FIX #5: Uses normalize_document_id to handle string/int conversion.
        // ====================================================================
        if let Some(doc_ids) = Self::extract_id_in_query(query_json) {
            let mut results = Vec::new();
            results.try_reserve(doc_ids.len()).map_err(|e| {
                IronBaseError::InvalidQuery(format!(
                    "Out of memory: cannot allocate space for {} documents ({})",
                    doc_ids.len(),
                    e
                ))
            })?;
            for doc_id in doc_ids {
                // Try original ID first
                if let Some(doc) = self.read_document_by_id(&doc_id)? {
                    results.push(doc);
                } else if let Some(normalized) = Self::normalize_document_id(&doc_id) {
                    // Try normalized version (string "123" → int 123)
                    if let Some(doc) = self.read_document_by_id(&normalized)? {
                        results.push(doc);
                    }
                }
            }
            return Ok(results);
        }

        // ====================================================================
        // SLOW PATH: Complex queries (non-_id filters)
        // ====================================================================
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

        // Validate projection early — fail fast before query execution
        if let Some(ref proj) = options.projection {
            crate::find_options::validate_projection(proj)?;
        }

        // ====================================================================
        // FAST PATH: Direct _id lookup O(1)
        // FIX #5-6: When query is {"_id": value}, skip catalog scanning entirely.
        // Uses normalize_document_id to handle string/int conversion.
        // Note: _id queries always return max 1 doc, so skip/limit don't matter.
        // ====================================================================
        // Single _id query - always fast path (returns 0 or 1 doc)
        if options.sort.is_none() {
            if let Some(doc_id) = Self::extract_id_query(query_json) {
                // Try original ID first
                if let Some(doc) = self.read_document_by_id(&doc_id)? {
                    return Ok(vec![doc]);
                }
                // Try normalized version (string "123" → int 123)
                if let Some(normalized) = Self::normalize_document_id(&doc_id) {
                    if let Some(doc) = self.read_document_by_id(&normalized)? {
                        return Ok(vec![doc]);
                    }
                }
                return Ok(Vec::new());
            }

            // Fast path for _id $in query
            if let Some(doc_ids) = Self::extract_id_in_query(query_json) {
                let mut results = Vec::new();
                results.try_reserve(doc_ids.len()).map_err(|e| {
                    IronBaseError::InvalidQuery(format!(
                        "Out of memory: cannot allocate space for {} documents ({})",
                        doc_ids.len(),
                        e
                    ))
                })?;
                for doc_id in doc_ids {
                    if let Some(doc) = self.read_document_by_id(&doc_id)? {
                        results.push(doc);
                    } else if let Some(normalized) = Self::normalize_document_id(&doc_id) {
                        if let Some(doc) = self.read_document_by_id(&normalized)? {
                            results.push(doc);
                        }
                    }
                }
                return Ok(results);
            }
        }

        // ====================================================================
        // SLOW PATH: Complex queries with sort/skip/limit
        // ====================================================================

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
            ctx.original_skip,        // For index-based sort: skip
            ctx.original_limit,       // For index-based sort: enables early termination
            ctx.cancel_flag.as_ref(), // For cooperative cancellation
            ctx.deadline,             // For cooperative timeout
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
        // Use try_reserve to detect OOM before it happens
        let mut docs = Vec::new();
        docs.try_reserve(doc_count).map_err(|e| {
            IronBaseError::InvalidQuery(format!(
                "Out of memory: cannot allocate space for {} documents ({}). \
                Solutions: 1) Add 'limit' to your query, 2) Use an index, 3) Use find_streaming() for large results.",
                doc_count, e
            ))
        })?;

        // Response size tracking for OOM protection
        let max_response_bytes = ctx.max_response_bytes;
        let mut total_response_bytes: usize = 0;

        // Early projection optimization:
        // Apply projection immediately after loading if safe (no sort on excluded fields)
        // This reduces memory usage and enables accurate response size estimation
        let early_project = ctx.can_early_project();

        // Cancellation flag reference for cooperative timeout
        let cancel_flag = &ctx.cancel_flag;

        // 🚀 PERF FIX: Acquire storage read lock ONCE for all documents
        // Previously: N lock acquisitions + N HashMap lookups for N documents
        // Now: 1 lock acquisition + 1 HashMap lookup for N documents
        let storage = self.storage.read();
        let meta = storage
            .get_collection_meta(&self.name)
            .ok_or_else(|| IronBaseError::CollectionNotFound(self.name.clone()))?;

        let mut loaded = 0;
        for doc_id in doc_ids {
            // Check for cancellation/timeout every 100 documents
            if loaded % 100 == 0 {
                if let Some(ref flag) = cancel_flag {
                    if flag.load(std::sync::atomic::Ordering::Relaxed) {
                        drop(storage); // Release lock before returning error
                        return Err(IronBaseError::Cancelled(format!(
                            "Query cancelled after loading {} documents. \
                            The operation was aborted due to client disconnection.",
                            loaded
                        )));
                    }
                }
                if let Some(dl) = ctx.deadline {
                    if std::time::Instant::now() >= dl {
                        drop(storage); // Release lock before returning error
                        return Err(IronBaseError::Timeout(format!(
                            "Query timed out after loading {} documents. \
                            The operation exceeded the configured deadline.",
                            loaded
                        )));
                    }
                }
            }

            // Inline document loading (no per-doc lock/meta lookup!)
            let doc_opt = if let Some(&offset) = meta.document_catalog.get(&doc_id) {
                let doc_bytes = storage.read_data_at(offset)?;
                let doc: Value = serde_json::from_slice(&doc_bytes)?;
                // Check tombstone
                if doc
                    .get("_tombstone")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                {
                    None
                } else {
                    Some(doc)
                }
            } else {
                None
            };

            if let Some(doc) = doc_opt {
                // Apply early projection if safe (reduces memory, enables accurate size check)
                let doc = if early_project {
                    ctx.apply_early_projection(doc)?
                } else {
                    doc
                };

                // Track response size if limit is set
                // NOTE: With early projection, this checks the PROJECTED size (accurate)
                // Without early projection, this checks the FULL document size
                if let Some(max_bytes) = max_response_bytes {
                    let doc_size = crate::find_options::estimate_json_size(&doc);
                    if total_response_bytes.saturating_add(doc_size) > max_bytes {
                        return Err(IronBaseError::InvalidQuery(format!(
                            "Response size limit exceeded: loaded {} documents ({} bytes), \
                            next document would exceed {} byte limit. \
                            Solutions: 1) Add 'limit' to reduce results, \
                            2) Use 'projection' to exclude large fields, \
                            3) Use find_streaming() for large results.",
                            loaded, total_response_bytes, max_bytes
                        )));
                    }
                    total_response_bytes += doc_size;
                }

                docs.push(doc);
                loaded += 1;
                // Progress logging every 10,000 documents
                if loaded % 10_000 == 0 && doc_count > LARGE_QUERY_WARNING_THRESHOLD {
                    log_debug!(
                        "find on '{}': loaded {}/{} documents ({} bytes)",
                        self.name,
                        loaded,
                        doc_count,
                        total_response_bytes
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

        // 4c. Apply projection (skip if already applied early)
        let docs = if early_project {
            // Projection already applied during loading
            docs
        } else {
            ctx.apply_projection_to_docs(docs)?
        };

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
        let storage = self.storage.read();
        if storage.get_file_path().is_empty() {
            drop(storage);
            let (doc_ids, _) = self.collect_doc_ids_with_options(
                query_json, None, None, false, 0, None, true, 0, None, None, None,
            )?;
            return Ok(FindCursor::new(self, doc_ids));
        }
        let has_catalog = storage
            .get_collection_meta(&self.name)
            .map(|meta| !meta.document_catalog.is_empty())
            .unwrap_or(false);
        drop(storage);

        if QueryPlanner::extract_logical_clauses(query_json).is_some() {
            if has_catalog {
                let (doc_ids, _) = self.collect_doc_ids_with_options(
                    query_json, None, None, false, 0, None, true, 0, None, None, None,
                )?;
                return Ok(FindCursor::new(self, doc_ids));
            }
            return FindCursor::new_scan(self, query_json);
        }

        let index_fields = {
            let indexes = self.indexes.read();
            indexes.list_indexes_with_compound_info()
        };

        if let Some((_field, plan)) =
            QueryPlanner::analyze_query_with_fields(query_json, &index_fields)
        {
            if let Some(cursor) = FindCursor::new_index_scan_from_plan(self, query_json, plan)? {
                return Ok(cursor);
            }
        }

        if has_catalog {
            let (doc_ids, _) = self.collect_doc_ids_with_options(
                query_json, None, None, false, 0, None, true, 0, None, None, None,
            )?;
            return Ok(FindCursor::new(self, doc_ids));
        }

        FindCursor::new_scan(self, query_json)
    }

    /// Streaming cursor with cooperative cancellation support
    ///
    /// Same as `find_streaming()` but accepts optional cancellation parameters
    /// for timeout enforcement during document collection phase.
    ///
    /// # Arguments
    /// * `query_json` - Query filter
    /// * `cancel_flag` - Optional atomic flag for external cancellation
    /// * `deadline` - Optional deadline for timeout enforcement
    ///
    /// # Example
    /// ```rust,ignore
    /// use std::time::{Duration, Instant};
    ///
    /// let deadline = Instant::now() + Duration::from_secs(30);
    /// let cursor = collection.find_streaming_with_options(&query, None, Some(deadline))?;
    /// ```
    pub fn find_streaming_with_options(
        &self,
        query_json: &Value,
        cancel_flag: Option<&Arc<AtomicBool>>,
        deadline: Option<std::time::Instant>,
    ) -> Result<FindCursor<'_, S>> {
        self.check_not_closed()?;
        let storage = self.storage.read();
        if storage.get_file_path().is_empty() {
            drop(storage);
            let (doc_ids, _) = self.collect_doc_ids_with_options(
                query_json,
                None,
                None,
                false,
                0,
                None,
                true,
                0,
                None,
                cancel_flag,
                deadline,
            )?;
            return Ok(FindCursor::new(self, doc_ids));
        }
        let has_catalog = storage
            .get_collection_meta(&self.name)
            .map(|meta| !meta.document_catalog.is_empty())
            .unwrap_or(false);
        drop(storage);

        if QueryPlanner::extract_logical_clauses(query_json).is_some() {
            if has_catalog {
                let (doc_ids, _) = self.collect_doc_ids_with_options(
                    query_json,
                    None,
                    None,
                    false,
                    0,
                    None,
                    true,
                    0,
                    None,
                    cancel_flag,
                    deadline,
                )?;
                return Ok(FindCursor::new(self, doc_ids));
            }
            // Note: new_scan path doesn't support deadline yet (iterates lazily)
            // For aggregate, we rely on AggregationLimitContext.check_deadline() in pipeline
            return FindCursor::new_scan(self, query_json);
        }

        let index_fields = {
            let indexes = self.indexes.read();
            indexes.list_indexes_with_compound_info()
        };

        if let Some((_field, plan)) =
            QueryPlanner::analyze_query_with_fields(query_json, &index_fields)
        {
            if let Some(cursor) = FindCursor::new_index_scan_from_plan(self, query_json, plan)? {
                return Ok(cursor);
            }
        }

        if has_catalog {
            let (doc_ids, _) = self.collect_doc_ids_with_options(
                query_json,
                None,
                None,
                false,
                0,
                None,
                true,
                0,
                None,
                cancel_flag,
                deadline,
            )?;
            return Ok(FindCursor::new(self, doc_ids));
        }

        // Note: new_scan path doesn't support deadline yet
        FindCursor::new_scan(self, query_json)
    }

    /// Find one document matching query
    ///
    /// Uses QueryPlanner for index optimization when available.
    /// For `_id` queries, uses direct O(1) catalog lookup.
    pub fn find_one(&self, query_json: &Value) -> Result<Option<Value>> {
        self.check_not_closed()?;
        // OPTIMIZATION: Check if this is an _id equality query (O(1) lookup)
        // This is faster than going through QueryPlanner for the most common case
        // FIX #7: Uses normalize_document_id to handle string/int conversion
        // e.g., {"_id": "123"} should match DocumentId::Int(123)
        if let Some(query_obj) = query_json.as_object() {
            if query_obj.len() == 1 && query_obj.contains_key("_id") {
                if let Some(id_val) = query_obj.get("_id") {
                    // Direct O(1) lookup using document_catalog
                    if let Ok(doc_id) = serde_json::from_value::<DocumentId>(id_val.clone()) {
                        // Try original ID first
                        if let Some(doc) = self.read_document_by_id(&doc_id)? {
                            // Verify query still matches (for consistency)
                            let parsed_query = Query::from_json(query_json)?;
                            // PERF: from_value borrows+clones, still faster than to_string+from_json
                            let document = Document::from_value(&doc)?;

                            if parsed_query.matches(&document)? {
                                return Ok(Some(doc));
                            }
                        }
                        // Try normalized version (string "123" → int 123)
                        if let Some(normalized) = Self::normalize_document_id(&doc_id) {
                            if let Some(doc) = self.read_document_by_id(&normalized)? {
                                let parsed_query = Query::from_json(query_json)?;
                                let document = Document::from_value(&doc)?;

                                if parsed_query.matches(&document)? {
                                    return Ok(Some(doc));
                                }
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
            None, // No cancel_flag for find_one
            None, // No deadline for find_one
        )?;

        if let Some(doc_id) = doc_ids.first() {
            self.read_document_by_id(doc_id)
        } else {
            Ok(None)
        }
    }

    /// Find one document matching query with execution context for cancellation support.
    ///
    /// This is the cancellation-aware version of `find_one`.
    /// Note: find_one is already bounded (limit=1), so cancellation is rarely needed,
    /// but this method allows passing an execution context for consistency.
    pub fn find_one_with_ctx(
        &self,
        query_json: &Value,
        ctx: Option<&ExecutionContext>,
    ) -> Result<Option<Value>> {
        self.check_not_closed()?;

        // OPTIMIZATION: Check if this is an _id equality query (O(1) lookup)
        if let Some(query_obj) = query_json.as_object() {
            if query_obj.len() == 1 && query_obj.contains_key("_id") {
                if let Some(id_val) = query_obj.get("_id") {
                    if let Ok(doc_id) = serde_json::from_value::<DocumentId>(id_val.clone()) {
                        // Try original ID first
                        if let Some(doc) = self.read_document_by_id(&doc_id)? {
                            let parsed_query = Query::from_json(query_json)?;
                            let document = Document::from_value(&doc)?;

                            if parsed_query.matches(&document)? {
                                return Ok(Some(doc));
                            }
                        }
                        // Try normalized version (string "123" → int 123)
                        if let Some(normalized) = Self::normalize_document_id(&doc_id) {
                            if let Some(doc) = self.read_document_by_id(&normalized)? {
                                let parsed_query = Query::from_json(query_json)?;
                                let document = Document::from_value(&doc)?;

                                if parsed_query.matches(&document)? {
                                    return Ok(Some(doc));
                                }
                            }
                        }
                    }
                    return Ok(None);
                }
            }
        }

        // Extract cancel_flag and deadline from ExecutionContext if available
        let cancel_flag = ctx.and_then(|c| c.cancel_flag().cloned());
        let deadline = ctx.and_then(|c| c.deadline());

        // Use QueryPlanner with limit=1 - enables index usage for indexed fields
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
            cancel_flag.as_ref(),
            deadline,
        )?;

        if let Some(doc_id) = doc_ids.first() {
            self.read_document_by_id(doc_id)
        } else {
            Ok(None)
        }
    }

    /// Find one document matching query with options (projection support).
    ///
    /// This is the extended version of `find_one` that supports projection
    /// at the core level, consistent with `find_with_options`.
    ///
    /// # Arguments
    /// * `query_json` - MongoDB-style query filter
    /// * `options` - FindOptions (only projection is used; sort/limit/skip are ignored for find_one)
    ///
    /// # Example
    /// ```rust,ignore
    /// use ironbase_core::find_options::FindOptions;
    /// use std::collections::HashMap;
    ///
    /// let mut projection = HashMap::new();
    /// projection.insert("name".to_string(), 1);
    /// projection.insert("email".to_string(), 1);
    ///
    /// let options = FindOptions::new().with_projection(projection);
    /// let doc = collection.find_one_with_options(&query, options)?;
    /// ```
    pub fn find_one_with_options(
        &self,
        query_json: &Value,
        options: crate::find_options::FindOptions,
    ) -> Result<Option<Value>> {
        // Get the document using existing find_one logic
        let doc = self.find_one(query_json)?;

        // Apply projection if specified and document found
        match (doc, options.projection) {
            (Some(d), Some(ref proj)) => {
                let projected = crate::find_options::apply_projection(&d, proj)?;
                Ok(Some(projected))
            }
            (doc, _) => Ok(doc),
        }
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
    fn find_with_index(
        &self,
        parsed_query: Query,
        plan: QueryPlan,
        cancel_flag: Option<&Arc<AtomicBool>>,
        deadline: Option<std::time::Instant>,
    ) -> Result<Vec<Value>> {
        let (doc_ids, _) = self.collect_doc_ids_from_plan(
            &parsed_query,
            plan,
            None,
            false,
            0,
            None,
            cancel_flag,
            deadline,
        )?;
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
        // Mark _id index dirty for checkpoint persistence
        indexes.mark_btree_dirty(&id_index_name);

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
        let t = std::time::Instant::now();
        let mut indexes = self.indexes.write();
        let lock_wait_ms = t.elapsed().as_millis() as u64;
        if lock_wait_ms > 50 {
            tracing::warn!(lock_wait_ms, collection = %self.name, "insert: indexes.write() slow acquire (add_to_indexes)");
        }
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

        // NOTE: Single write lock held for entire batch — typical hold time 100-300ms
        // (20K updates × 10 indexes). Concurrent inserts/queries on this collection
        // are blocked during this window. Splitting is non-trivial because btree,
        // fulltext, fuzzy, and HNSW updates must see a consistent document state.
        let mut indexes = self.indexes.write();
        let id_index_name = format!("{}_id", self.name);
        let index_names: Vec<String> = indexes.list_indexes();

        // Preflight: reject compound indexes with multiple array fields (MongoDB restriction)
        for index_name in &index_names {
            if index_name == &id_index_name {
                continue;
            }
            if let Some(index) = indexes.get_btree_index(index_name) {
                if index.metadata.is_compound() {
                    for (_, updated_doc) in updates {
                        let updated_value = serde_json::to_value(updated_doc)?;
                        if IndexManager::count_compound_array_fields(
                            &updated_value,
                            &index.metadata.fields,
                        ) > 1
                        {
                            return Err(IronBaseError::IndexError(format!(
                                "Compound index '{}' cannot index multiple array fields: {}",
                                index.metadata.name,
                                index.metadata.fields.join(", ")
                            )));
                        }
                    }
                }
            }
        }

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
            // Mark _id index dirty for checkpoint persistence
            indexes.mark_btree_dirty(&id_index_name);
        }

        // --- OTHER INDEXES: multi-key aware updates ---
        for index_name in &index_names {
            if index_name == &id_index_name {
                continue;
            }

            if let Some(index) = indexes.get_btree_index_mut(index_name) {
                for (original_doc, updated_doc) in updates {
                    let old_doc_value = match serde_json::to_value(original_doc) {
                        Ok(v) => v,
                        Err(e) => {
                            log_warn!(
                                "B+tree index '{}' skipping doc {:?}: failed to serialize original doc: {}",
                                index_name,
                                original_doc.id,
                                e
                            );
                            continue;
                        }
                    };
                    let new_doc_value = match serde_json::to_value(updated_doc) {
                        Ok(v) => v,
                        Err(e) => {
                            log_warn!(
                                "B+tree index '{}' skipping doc {:?}: failed to serialize updated doc: {}",
                                index_name,
                                updated_doc.id,
                                e
                            );
                            continue;
                        }
                    };

                    let old_keys: HashSet<IndexKey> =
                        index.extract_keys(&old_doc_value).into_iter().collect();
                    let new_keys: HashSet<IndexKey> =
                        index.extract_keys(&new_doc_value).into_iter().collect();

                    let doc_id = &original_doc.id;

                    for key in old_keys.difference(&new_keys) {
                        if let Err(e) = index.delete(key, doc_id) {
                            log_warn!(
                                "B+tree index '{}' delete failed for doc {:?}: {:?}",
                                index_name,
                                doc_id,
                                e
                            );
                        }
                    }

                    for key in new_keys.difference(&old_keys) {
                        index.insert(key.clone(), doc_id.clone())?;
                    }
                }
            }
            // Mark dirty for checkpoint persistence
            indexes.mark_btree_dirty(index_name);
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
                        // Dispatch on the new value's string-ness:
                        // - string → update() handles remove+insert internally
                        // - non-string / None → plain remove() of prior entry
                        match new_value.and_then(|v| v.as_str()) {
                            Some(text) => {
                                let pdid_field = fts_index.parent_doc_id_field().to_string();
                                let parent_doc_id = updated_doc
                                    .get(&pdid_field)
                                    .and_then(|v| v.as_str().map(|s| s.to_string()));
                                // Log error but continue - document already persisted
                                let update_result = if let Some(ref pdid) = parent_doc_id {
                                    fts_index.update_with_parent_doc_id(&updated_doc.id, text, pdid)
                                } else {
                                    fts_index.update(&updated_doc.id, text)
                                };
                                if let Err(e) = update_result {
                                    log_error!(
                                        "Failed to update fulltext index '{}' for doc {:?}: {:?}",
                                        fts_name,
                                        updated_doc.id,
                                        e
                                    );
                                }
                            }
                            None => {
                                // New value is None or non-string. Only drop the old
                                // entry if there was a prior string indexed.
                                if old_value.and_then(|v| v.as_str()).is_some() {
                                    let _ = fts_index.remove(&original_doc.id);
                                }
                            }
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

        // --- VECTOR (HNSW) INDEXES: Update for each document ---
        // Collect vector index info before mutable borrow
        let vector_index_info: Vec<(String, usize)> = indexes
            .list_vector_indexes()
            .iter()
            .map(|idx| (idx.config().field.clone(), idx.config().dim))
            .collect();

        for (vec_field, vec_dim) in vector_index_info {
            if vec_field.is_empty() {
                continue; // Skip if no field configured (legacy index)
            }
            for (original_doc, updated_doc) in updates {
                let old_value = original_doc.get(&vec_field);
                let new_value = updated_doc.get(&vec_field);

                // Only update if the vector field actually changed
                if old_value != new_value {
                    let doc_id = &original_doc.id;
                    let id_str = match doc_id {
                        DocumentId::Int(i) => i.to_string(),
                        DocumentId::String(s) => s.clone(),
                        DocumentId::ObjectId(oid) => oid.clone(),
                    };

                    if let Some(index) =
                        indexes.get_vector_index_for_field_mut(&self.name, &vec_field)
                    {
                        // Remove old vector if it existed (unconditional - remove() is ID-based)
                        if old_value.and_then(|v| v.as_array()).is_some() {
                            index.remove(&id_str);
                        }

                        // Insert new vector if it exists with correct dimension
                        if let Some(arr) = new_value.and_then(|v| v.as_array()) {
                            let vector: Vec<f32> = arr
                                .iter()
                                .filter_map(|v| v.as_f64().map(|f| f as f32))
                                .collect();
                            if vector.len() == vec_dim {
                                // Swallow (log-and-continue), NOT propagate — see
                                // add_document_to_indexes. update_*_persist writes
                                // storage AFTER batch_update_indexes returns, so a
                                // propagated Err would diverge the index (NEW
                                // state) from storage (OLD docs). P0-2's
                                // RAM-derived ceiling makes a hit reachable only at
                                // true RAM exhaustion.
                                if let Err(e) = index.insert(&id_str, &vector) {
                                    log_error!(
                                        "Failed to update vector index for field '{}': {:?} (document stored; vector not indexed)",
                                        vec_field,
                                        e
                                    );
                                }
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
    fn batch_add_to_indexes(&self, docs: &[&Document]) -> Result<()> {
        if docs.is_empty() {
            return Ok(());
        }

        let mut indexes = self.indexes.write();
        let id_index_name = format!("{}_id", self.name);

        for &doc in docs {
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
        // Step 1: Capture stable document order (hold lock briefly)
        let ordered_ids: Vec<DocumentId> = {
            let storage = self.storage.read();
            let meta = storage
                .get_collection_meta(&self.name)
                .ok_or_else(|| IronBaseError::CollectionNotFound(self.name.clone()))?;
            if !meta.document_order.is_empty() {
                meta.document_order.clone()
            } else {
                let mut offsets: Vec<(DocumentId, u64)> = meta
                    .document_catalog
                    .iter()
                    .map(|(id, &offset)| (id.clone(), offset))
                    .collect();
                offsets.sort_by_key(|(_, offset)| *offset);
                offsets.into_iter().map(|(id, _)| id).collect()
            }
        }; // storage lock released here

        let total_docs = ordered_ids.len();
        let mut processed = 0;
        let mut batch_num = 0;

        // Step 2: Process in batches, releasing storage lock between batches
        // CRITICAL FIX: Use read lock + read_data_at (positioned read) instead of
        // write lock + read_data. This allows concurrent reads during index building
        // and prevents deadlock with concurrent batch inserts.
        for chunk in ordered_ids.chunks(batch_size) {
            // Read raw bytes from disk using positioned read (no file position change)
            let raw_batch: Vec<(DocumentId, Vec<u8>)> = {
                let storage = self.storage.read(); // READ lock instead of WRITE lock
                let meta = storage
                    .get_collection_meta(&self.name)
                    .ok_or_else(|| IronBaseError::CollectionNotFound(self.name.clone()))?;
                chunk
                    .iter()
                    .filter_map(|doc_id| {
                        meta.document_catalog.get(doc_id).and_then(|offset| {
                            storage
                                .read_data_at(*offset) // pread - no position change
                                .ok()
                                .map(|bytes| (doc_id.clone(), bytes))
                        })
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
        cancel_flag: Option<&Arc<AtomicBool>>,
        deadline: Option<std::time::Instant>,
    ) -> Result<Vec<DocumentId>> {
        // CRITICAL FIX: Only hold storage read lock briefly for catalog snapshot.
        // Release BEFORE document iteration to allow concurrent inserts/checkpoints.
        // parking_lot::RwLock: read lock blocks write lock acquisition, so holding
        // it for the entire 130K-doc scan would starve insert_one for minutes.

        // MEMORY OPTIMIZATION: Don't clone entire catalog HashMap!
        // Only extract what we need: Vec<(DocumentId, u64)> for offsets
        let (catalog_entries, catalog_len, live_count) = {
            let storage = self.storage.read();
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

            // OOM PROTECTION: Check memory BEFORE collecting catalog entries
            // This is the first major allocation - catches OOM early
            let catalog_size = meta.document_catalog.len();
            let mut pre_check = Vec::<(DocumentId, u64)>::new();
            pre_check.try_reserve(catalog_size).map_err(|e| {
                IronBaseError::OutOfMemory(format!(
                    "Cannot allocate for {} catalog entries ({}). \
                    Collection too large for available memory.",
                    catalog_size, e
                ))
            })?;
            drop(pre_check);

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
            // Check deadline/cancellation before fast path (large catalogs can still take time to sort)
            if let Some(flag) = cancel_flag {
                if flag.load(std::sync::atomic::Ordering::Relaxed) {
                    return Err(IronBaseError::Cancelled(
                        "Query cancelled before execution".into(),
                    ));
                }
            }
            if let Some(dl) = deadline {
                if std::time::Instant::now() >= dl {
                    return Err(IronBaseError::Timeout(
                        "Query timed out before execution".into(),
                    ));
                }
            }

            // No tombstones - safe to use fast path
            // MongoDB compat: limit=0 means "no limit" (return all)
            let effective_limit = match limit {
                Some(0) | None => usize::MAX,
                Some(n) => n,
            };

            // NOTE: OOM protection already done above when collecting catalog_entries

            // 🚀 PERF FIX: For small skip+limit, use heap-based partial sort O(n log k)
            // instead of full sort O(n log n). Critical for pagination on large collections!
            let need = skip.saturating_add(effective_limit);
            let doc_ids: Vec<DocumentId> = if effective_limit != usize::MAX
                && need < catalog_len
                && need < crate::limits::HEAP_PAGINATION_THRESHOLD
            {
                // Use max-heap to find smallest `need` elements in O(n log k)
                use std::collections::BinaryHeap;

                let mut heap: BinaryHeap<DocumentId> = BinaryHeap::with_capacity(need + 1);
                for (id, _) in catalog_entries {
                    if heap.len() < need {
                        heap.push(id);
                    } else if let Some(max) = heap.peek() {
                        if &id < max {
                            heap.pop();
                            heap.push(id);
                        }
                    }
                }

                // Extract and sort just the small result set O(k log k)
                let mut smallest: Vec<_> = heap.into_iter().collect();
                smallest.sort();

                smallest
                    .into_iter()
                    .skip(skip)
                    .take(effective_limit)
                    .collect()
            } else {
                // Large skip or no limit - use full sort (deterministic pagination)
                let mut sorted_keys: Vec<DocumentId> =
                    catalog_entries.into_iter().map(|(id, _)| id).collect();
                sorted_keys.sort();

                sorted_keys
                    .into_iter()
                    .skip(skip)
                    .take(if effective_limit == 0 {
                        usize::MAX
                    } else {
                        effective_limit
                    })
                    .collect()
            };
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

        // OOM PROTECTION: Reserve for expected results (limit or catalog size)
        // Note: catalog_entries already allocated, so this is likely to succeed
        // but we keep it as a safety check for the result Vec
        let estimated_matches = limit.unwrap_or(catalog_len).min(catalog_len);
        doc_ids.try_reserve(estimated_matches).map_err(|e| {
            IronBaseError::OutOfMemory(format!(
                "Cannot allocate for {} document IDs ({}). \
                Solutions: 1) Add 'limit' to your query, 2) Use an index, 3) Increase system memory.",
                estimated_matches, e
            ))
        })?;

        // BUG #1 FIX: Sort entries for deterministic pagination
        // HashMap iteration order is non-deterministic due to ASLR hash seeds
        // This ensures consistent results even when tombstones are present
        let mut sorted_entries = catalog_entries;
        sorted_entries.sort_by(|(a, _), (b, _)| a.cmp(b));

        // Iterate over sorted entries with early termination (for filtered queries)
        for (scanned, (doc_id, offset)) in sorted_entries.into_iter().enumerate() {
            // Check for cancellation/timeout every 100 documents BEFORE expensive operations
            if scanned % 100 == 0 {
                if let Some(flag) = cancel_flag {
                    if flag.load(std::sync::atomic::Ordering::Relaxed) {
                        return Err(IronBaseError::Cancelled(format!(
                            "Query cancelled after scanning {} documents. \
                            The operation was aborted due to client disconnection.",
                            scanned
                        )));
                    }
                }
                if let Some(dl) = deadline {
                    if std::time::Instant::now() >= dl {
                        return Err(IronBaseError::Timeout(format!(
                            "Query timed out after scanning {} documents. \
                            The operation exceeded the configured deadline.",
                            scanned
                        )));
                    }
                }
            }

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

            // Read document from storage (brief read lock per document,
            // released before parsing to allow concurrent inserts)
            let read_result = {
                let storage = self.storage.read();
                storage.read_data_at(offset)
            };
            let doc_bytes = match read_result {
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
        let (ids, _) = self.collect_doc_ids_with_options(
            query_json, None, None, false, 0, None, true, 0, None, None, None,
        )?;
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
        cancel_flag: Option<&Arc<AtomicBool>>, // For cooperative cancellation
        deadline: Option<std::time::Instant>, // For cooperative timeout
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
            if let Some((logical_op, clauses)) = QueryPlanner::extract_logical_clauses(query_json) {
                // Try clause merge optimization first: rewrite $and/$or to implicit form
                // so the standard planner can use a single index scan instead of HashSet merge.
                let indexes = self.indexes.read();
                let index_fields = indexes.list_indexes_with_compound_info();
                drop(indexes);

                if let Some(merged_query) =
                    QueryPlanner::try_merge_logical_clauses(logical_op, &clauses, &index_fields)
                {
                    // Plan with merged query, but post-filter with ORIGINAL query for correctness
                    if let Some((_, plan)) =
                        QueryPlanner::analyze_query_with_fields(&merged_query, &index_fields)
                    {
                        return self.collect_doc_ids_from_plan(
                            &parsed_query,
                            plan,
                            sort_field,
                            sort_desc,
                            skip,
                            limit,
                            cancel_flag,
                            deadline,
                        );
                    }
                }

                // Fallback: original per-clause logical operator handling
                if let Some(result) = self.collect_doc_ids_for_logical_operator(
                    &parsed_query,
                    logical_op,
                    &clauses,
                    sort_field,
                    sort_desc,
                    skip,
                    limit,
                    cancel_flag,
                    deadline,
                )? {
                    return Ok(result);
                }
            }
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
            self.collect_doc_ids_from_plan(
                &parsed_query,
                plan,
                sort_field,
                sort_desc,
                skip,
                limit,
                cancel_flag,
                deadline,
            )?
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
                    let doc_ids = self.scan_documents_with_early_termination(
                        &parsed_query,
                        skip,
                        limit,
                        cancel_flag,
                        deadline,
                    )?;
                    (doc_ids, false)
                }
            } else {
                // Query has filter but no index plan - must scan all documents
                let doc_ids = self.scan_documents_with_early_termination(
                    &parsed_query,
                    skip,
                    limit,
                    cancel_flag,
                    deadline,
                )?;
                (doc_ids, false)
            }
        } else {
            // 🚀 FIX: Use early termination scan instead of loading all documents
            // This is critical for performance on large collections with pagination.
            // Previously scan_documents_via_catalog() loaded ALL documents first,
            // making limit=1 take the same time as limit=50 on large collections.
            let doc_ids = self.scan_documents_with_early_termination(
                &parsed_query,
                skip,
                limit,
                cancel_flag,
                deadline,
            )?;
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
        cancel_flag: Option<&Arc<AtomicBool>>,
        deadline: Option<std::time::Instant>,
    ) -> Result<(Vec<DocumentId>, bool)> {
        // Check deadline/cancellation at start
        if let Some(flag) = cancel_flag {
            if flag.load(std::sync::atomic::Ordering::Relaxed) {
                return Err(IronBaseError::Cancelled(
                    "Query cancelled before index scan".into(),
                ));
            }
        }
        if let Some(dl) = deadline {
            if std::time::Instant::now() >= dl {
                return Err(IronBaseError::Timeout(
                    "Query timed out before index scan".into(),
                ));
            }
        }
        let mut index_limit_applied = false;
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
                        let mode = RangeQueryMode::Scan {
                            skip: 0,
                            limit: None,
                            order: ScanOrder::Asc,
                        };
                        if is_compound {
                            // Compound index prefix query: use range scan with compound bounds
                            let (start, end) = index.build_prefix_range(key.clone());
                            index
                                .range_query(&start, &end, true, true, mode)
                                .unwrap_docs()
                        } else {
                            // Single-field index: point lookup
                            index.range_query(key, key, true, true, mode).unwrap_docs()
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
                        let mode = RangeQueryMode::Scan {
                            skip: 0,
                            limit: None,
                            order: ScanOrder::Asc,
                        };
                        index
                            .range_query(start_key, end_key, inclusive_start, inclusive_end, mode)
                            .unwrap_docs()
                    } else {
                        vec![]
                    }
                }
                QueryPlan::RegexPrefixScan {
                    ref index_name,
                    ref prefix,
                    exact,
                    ref field,
                    ..
                } => {
                    if let Some(index) = indexes.get_btree_index(index_name) {
                        let can_apply_limit = exact
                            && !index.metadata.multikey
                            && Self::is_simple_regex_query(parsed_query.to_json(), field);
                        let (scan_skip, scan_limit) = if can_apply_limit {
                            index_limit_applied = true;
                            (skip, limit)
                        } else {
                            (0, None)
                        };
                        let start = IndexKey::String(prefix.clone());
                        let end = IndexKey::String(format!("{}\u{10ffff}", prefix));
                        let mode = RangeQueryMode::Scan {
                            skip: scan_skip,
                            limit: scan_limit,
                            order: ScanOrder::Asc,
                        };
                        index
                            .range_query(&start, &end, true, true, mode)
                            .unwrap_docs()
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
                        let mode = RangeQueryMode::Scan {
                            skip: 0,
                            limit: None,
                            order: ScanOrder::Asc,
                        };
                        index
                            .range_query(&start, &end, true, true, mode)
                            .unwrap_docs()
                    } else {
                        vec![]
                    }
                }
                QueryPlan::MultiRegexPrefixScan {
                    ref index_name,
                    ref prefixes,
                    ..
                } => {
                    // Multi-regex prefix scan: union of multiple prefix range scans
                    if let Some(index) = indexes.get_btree_index(index_name) {
                        let mut all_doc_ids = Vec::new();
                        for prefix in prefixes {
                            let start = IndexKey::String(prefix.clone());
                            let end = IndexKey::String(format!("{}\u{10ffff}", prefix));
                            let mode = RangeQueryMode::Scan {
                                skip: 0,
                                limit: None,
                                order: ScanOrder::Asc,
                            };
                            let ids = index
                                .range_query(&start, &end, true, true, mode)
                                .unwrap_docs();
                            all_doc_ids.extend(ids);
                        }
                        // Sort and dedup to remove duplicates from overlapping ranges
                        all_doc_ids.sort_unstable();
                        all_doc_ids.dedup();
                        all_doc_ids
                    } else {
                        vec![]
                    }
                }
                QueryPlan::MultiValueScan {
                    ref index_name,
                    ref keys,
                    ..
                } => {
                    // Multi-value scan: O(k) index lookups for $in queries
                    // Much faster than collection scan O(n) when k << n
                    if let Some(index) = indexes.get_btree_index(index_name) {
                        let mut all_doc_ids = Vec::new();
                        for key in keys {
                            let mode = RangeQueryMode::Scan {
                                skip: 0,
                                limit: None,
                                order: ScanOrder::Asc,
                            };
                            // Point lookup for each key
                            let ids = index.range_query(key, key, true, true, mode).unwrap_docs();
                            all_doc_ids.extend(ids);
                        }
                        // Sort and dedup (in case of duplicates from multikey index)
                        all_doc_ids.sort_unstable();
                        all_doc_ids.dedup();
                        all_doc_ids
                    } else {
                        vec![]
                    }
                }
            }
        };

        let uses_index_sort = match (&plan, sort_field) {
            (QueryPlan::IndexScan { ref field, .. }, Some(sf)) if field == sf => true,
            (QueryPlan::IndexRangeScan { ref field, .. }, Some(sf)) if field == sf => true,
            (QueryPlan::RegexPrefixScan { ref field, .. }, Some(sf)) if field == sf => true,
            (QueryPlan::MultiRegexPrefixScan { ref field, .. }, Some(sf)) if field == sf => true,
            _ => false,
        };

        if uses_index_sort && sort_desc {
            doc_ids.reverse();
        }

        // Deduplicate multikey index hits while preserving scan order.
        let mut seen_doc_ids = HashSet::new();
        doc_ids.retain(|doc_id| seen_doc_ids.insert(doc_id.clone()));

        // Apply skip/limit while verifying query
        let mut results = Vec::new();
        let mut skipped = 0usize;
        let (match_skip, match_limit) = if let QueryPlan::RegexPrefixScan {
            exact, ref field, ..
        } = plan
        {
            if index_limit_applied
                && exact
                && Self::is_simple_regex_query(parsed_query.to_json(), field)
            {
                (0, None)
            } else {
                (skip, limit)
            }
        } else {
            (skip, limit)
        };

        for doc_id in doc_ids {
            if let Some(doc) = self.read_document_by_id(&doc_id)? {
                let document = Document::from_value_owned(doc)?;

                if parsed_query.matches(&document)? {
                    if skipped < match_skip {
                        skipped += 1;
                        continue;
                    }

                    results.push(doc_id.clone());
                    // MongoDB compatibility: limit(0) means "no limit"
                    if let Some(limit_count) = match_limit {
                        if limit_count > 0 && results.len() >= limit_count {
                            break;
                        }
                    }
                }
            }
        }

        Ok((results, uses_index_sort))
    }

    fn collect_doc_ids_for_logical_operator(
        &self,
        parsed_query: &Query,
        logical_op: LogicalOperator,
        clauses: &[Value],
        sort_field: Option<&str>,
        sort_desc: bool,
        skip: usize,
        limit: Option<usize>,
        cancel_flag: Option<&Arc<AtomicBool>>,
        deadline: Option<std::time::Instant>,
    ) -> Result<Option<(Vec<DocumentId>, bool)>> {
        let indexes = self.indexes.read();
        let index_fields = indexes.list_indexes_with_compound_info();
        drop(indexes);

        let mut clause_plans = Vec::new();
        for clause in clauses {
            let clause_query = Query::from_json(clause)?;
            let plan_opt =
                QueryPlanner::analyze_query_with_fields(clause, &index_fields).map(|(_, p)| p);
            clause_plans.push((clause_query, plan_opt));
        }

        let candidate_ids = match logical_op {
            LogicalOperator::And => {
                let target_limit = limit.unwrap_or(usize::MAX);
                let mut indexed_sets: Vec<Vec<DocumentId>> = Vec::new();
                for (clause_query, plan_opt) in &clause_plans {
                    if let Some(plan) = plan_opt.clone() {
                        let (ids, _) = self.collect_doc_ids_from_plan(
                            clause_query,
                            plan,
                            None,
                            false,
                            0,
                            Some(target_limit),
                            cancel_flag,
                            deadline,
                        )?;
                        indexed_sets.push(ids);
                    }
                }

                if indexed_sets.is_empty() {
                    return Ok(None);
                }

                indexed_sets.sort_by_key(|ids| ids.len());
                let mut base = indexed_sets.remove(0);
                for other in indexed_sets {
                    let other_set: HashSet<_> = other.into_iter().collect();
                    base.retain(|id| other_set.contains(id));
                    if base.is_empty() {
                        break;
                    }
                }
                base
            }
            LogicalOperator::Or => {
                let target_limit = limit.unwrap_or(usize::MAX);
                let mut seen = HashSet::new();
                let mut union = Vec::new();

                // Partition: indexed clauses first (fast), non-indexed last (slow)
                let (indexed, non_indexed): (Vec<_>, Vec<_>) =
                    clause_plans.iter().partition(|(_, plan)| plan.is_some());

                // 1. Run indexed clauses first
                for (clause_query, plan_opt) in &indexed {
                    if union.len() >= target_limit {
                        break;
                    }

                    let Some(plan) = plan_opt.clone() else {
                        continue;
                    };
                    let remaining = target_limit.saturating_sub(union.len());
                    let (ids, _) = self.collect_doc_ids_from_plan(
                        clause_query,
                        plan,
                        None,
                        false,
                        0,
                        Some(remaining),
                        cancel_flag,
                        deadline,
                    )?;

                    for id in ids {
                        if seen.insert(id.clone()) {
                            union.push(id);
                            if union.len() >= target_limit {
                                break;
                            }
                        }
                    }
                }

                // 2. Run non-indexed clauses only if we still need more results
                for (clause_query, _) in &non_indexed {
                    if union.len() >= target_limit {
                        break;
                    }

                    let remaining = target_limit.saturating_sub(union.len());
                    let ids = self.scan_documents_with_early_termination(
                        clause_query,
                        0,
                        Some(remaining),
                        cancel_flag,
                        deadline,
                    )?;

                    for id in ids {
                        if seen.insert(id.clone()) {
                            union.push(id);
                            if union.len() >= target_limit {
                                break;
                            }
                        }
                    }
                }

                union
            }
            LogicalOperator::Nor => {
                let mut excluded = HashSet::new();
                for (clause_query, plan_opt) in &clause_plans {
                    let ids = if let Some(plan) = plan_opt.clone() {
                        let (ids, _) = self.collect_doc_ids_from_plan(
                            clause_query,
                            plan,
                            None,
                            false,
                            0,
                            None,
                            cancel_flag,
                            deadline,
                        )?;
                        ids
                    } else {
                        self.scan_documents_with_early_termination(
                            clause_query,
                            0,
                            None,
                            cancel_flag,
                            deadline,
                        )?
                    };
                    for id in ids {
                        excluded.insert(id);
                    }
                }
                let all_ids = self.scan_documents_with_early_termination(
                    &Query::new(),
                    0,
                    None,
                    cancel_flag,
                    deadline,
                )?;
                all_ids
                    .into_iter()
                    .filter(|id| !excluded.contains(id))
                    .collect()
            }
        };

        let filtered = self.filter_doc_ids_by_query(
            parsed_query,
            candidate_ids,
            skip,
            limit,
            cancel_flag,
            deadline,
        )?;
        let _ = (sort_field, sort_desc);
        Ok(Some((filtered, false)))
    }

    /// Filter document IDs by executing query matching.
    ///
    /// This is where expensive operations (regex matching, document loading) occur.
    /// Deadline/cancellation is checked BEFORE each batch of 100 documents to ensure
    /// timely response to timeout requests.
    ///
    /// # Timeout Behavior (v1.0.182+)
    /// - Checks every 100 iterations for cancellation/timeout
    /// - Returns `IronBaseError::Timeout` if deadline exceeded
    /// - Returns `IronBaseError::Cancelled` if cancel_flag is set
    fn filter_doc_ids_by_query(
        &self,
        parsed_query: &Query,
        doc_ids: Vec<DocumentId>,
        skip: usize,
        limit: Option<usize>,
        cancel_flag: Option<&Arc<AtomicBool>>,
        deadline: Option<std::time::Instant>,
    ) -> Result<Vec<DocumentId>> {
        let mut results = Vec::new();
        let mut skipped = 0usize;

        for (iteration, doc_id) in doc_ids.into_iter().enumerate() {
            // Check cancellation/timeout every 100 iterations (BEFORE expensive regex matching)
            if iteration % 100 == 0 {
                if let Some(flag) = cancel_flag {
                    if flag.load(std::sync::atomic::Ordering::Relaxed) {
                        return Err(IronBaseError::Cancelled(
                            "Query cancelled during filter".into(),
                        ));
                    }
                }
                if let Some(dl) = deadline {
                    if std::time::Instant::now() >= dl {
                        return Err(IronBaseError::Timeout(
                            "Query timed out during filter".into(),
                        ));
                    }
                }
            }

            if let Some(doc) = self.read_document_by_id(&doc_id)? {
                let document = Document::from_value_owned(doc)?;

                if parsed_query.matches(&document)? {
                    if skipped < skip {
                        skipped += 1;
                        continue;
                    }

                    results.push(doc_id.clone());
                    if let Some(limit_count) = limit {
                        if limit_count > 0 && results.len() >= limit_count {
                            break;
                        }
                    }
                }
            }
        }

        Ok(results)
    }

    fn query_matches_all(query_json: &Value) -> bool {
        match query_json {
            Value::Null => true,
            Value::Object(map) => map.is_empty(),
            _ => false,
        }
    }

    fn is_simple_regex_query(query_json: &Value, field: &str) -> bool {
        let map = match query_json {
            Value::Object(map) => map,
            _ => return false,
        };
        if map.len() != 1 {
            return false;
        }
        let value = match map.get(field) {
            Some(v) => v,
            None => return false,
        };
        let cond = match value.as_object() {
            Some(obj) => obj,
            None => return false,
        };

        let has_regex = cond.get("$regex").and_then(|v| v.as_str()).is_some();
        if !has_regex {
            return false;
        }

        let options_ok = match cond.get("$options").and_then(|v| v.as_str()) {
            Some(opts) => opts.is_empty(),
            None => true,
        };
        if !options_ok {
            return false;
        }

        cond.len() == if cond.contains_key("$options") { 2 } else { 1 }
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

    /// Normalize a DocumentId by converting string representations to int if possible.
    ///
    /// This handles the common case where insert_many returns IDs as strings like "178755"
    /// but the catalog stores them as DocumentId::Int(178755).
    /// Without this, {"_id": {"$in": ["178755"]}} would fail to match DocumentId::Int(178755).
    fn normalize_document_id(doc_id: &DocumentId) -> Option<DocumentId> {
        match doc_id {
            DocumentId::String(s) => {
                // Try to parse as integer
                s.parse::<i64>().ok().map(DocumentId::Int)
            }
            _ => None, // Int and ObjectId don't need normalization
        }
    }

    /// Extract DocumentId list from {"_id": {"$in": [id1, id2, ...]}} query
    ///
    /// Used by delete_many fast path to avoid collection scan when deleting
    /// by a list of known _ids. Returns None if query format doesn't match.
    fn extract_id_in_query(query_json: &Value) -> Option<Vec<DocumentId>> {
        // Pattern: {"_id": {"$in": [id1, id2, ...]}}
        let Value::Object(map) = query_json else {
            return None;
        };
        if map.len() != 1 {
            return None;
        }
        let Some(Value::Object(id_map)) = map.get("_id") else {
            return None;
        };
        if id_map.len() != 1 {
            return None;
        }
        let Some(Value::Array(arr)) = id_map.get("$in") else {
            return None;
        };

        // Try to parse all values as DocumentId
        let mut ids = Vec::with_capacity(arr.len());
        for val in arr {
            let Ok(doc_id) = serde_json::from_value::<DocumentId>(val.clone()) else {
                // If any ID fails to parse, fall back to slow path
                return None;
            };
            ids.push(doc_id);
        }
        Some(ids)
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

        // 🔧 REFACTORED: Use unified range_query API for both ASC and DESC
        // Both paths now support early termination with O(limit) memory
        use crate::index::RangeQueryResult;

        let order = if sort_desc {
            ScanOrder::Desc
        } else {
            ScanOrder::Asc
        };

        let mode = RangeQueryMode::Scan { skip, limit, order };

        let result = btree.range_query(
            &crate::index::IndexKey::Null,
            &crate::index::IndexKey::MaxKey,
            true,
            true,
            mode,
        );

        let doc_ids = match result {
            RangeQueryResult::Docs(ids) => ids,
            _ => unreachable!("Scan mode always returns Docs"),
        };
        drop(indexes);

        Ok(Some((index_name, doc_ids)))
    }
}
