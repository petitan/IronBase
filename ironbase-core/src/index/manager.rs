//! Index Manager - Unified Index Lifecycle Management
//!
//! This module provides `IndexManager`, the central coordinator for all index types
//! within a single collection. It handles creation, lookup, updates, and persistence.
//!
//! # Index Types
//!
//! | Type | Use Case | File Extension |
//! |------|----------|----------------|
//! | B+ Tree | Equality/range queries | `.idx` |
//! | Fuzzy | Similarity search (Jaro-Winkler, Levenshtein) | `.fzidx` |
//! | Full-text | TF-IDF text search with stemming | `.ftidx` |
//! | Legacy | HashMap-based (deprecated) | N/A |
//!
//! # Per-Collection Instance
//!
//! Each collection has its own `IndexManager` instance, managed by `DatabaseCore`:
//!
//! ```text
//! DatabaseCore
//!   └── index_managers: HashMap<String, Arc<RwLock<IndexManager>>>
//!         ├── "users" → IndexManager { btree: [_id, email], fuzzy: [name] }
//!         ├── "posts" → IndexManager { btree: [_id], fulltext: [content] }
//!         └── "logs"  → IndexManager { btree: [_id, timestamp] }
//! ```
//!
//! # Index Selection for Queries
//!
//! The `select_best_index()` method implements query optimization:
//!
//! 1. **Exact match**: Find index on queried field
//! 2. **Compound prefix**: Use compound index if query matches prefix
//! 3. **Range optimization**: Prefer indexes for $gt, $lt, $gte, $lte
//!
//! # Persistence
//!
//! Indexes are persisted to separate files alongside the main `.mlite` database:
//!
//! ```text
//! database.mlite      # Main database file
//! database_users_id.idx       # B+ tree index
//! database_users_email.idx    # B+ tree index
//! database_posts_content.ftidx  # Full-text index
//! database_users_name.fzidx   # Fuzzy index
//! ```
//!
//! # Thread Safety
//!
//! `IndexManager` itself is NOT thread-safe. Concurrency is managed by
//! `DatabaseCore` which wraps each instance in `Arc<RwLock<IndexManager>>`.

use crate::document::DocumentId;
use crate::error::{IronBaseError, Result};
use crate::fulltext::{FtsLanguage, FtsOptions, FulltextIndex};
use crate::log_error;
use crate::value_utils::{get_all_nested_values, get_nested_value, path_crosses_array};
use crate::vector::{HnswIndex, VectorIndexConfig};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use super::btree::BPlusTree;
use super::fuzzy::{FuzzyAlgorithm, FuzzyIndex};
use super::key::{IndexKey, IndexPrefixInfo};
use super::legacy::{Index, IndexDefinition};

/// Generate a `get_*_index_mut` method that auto-marks the index as dirty.
///
/// All index types (btree, fuzzy, fulltext, vector) follow the same pattern:
/// check if the key exists in the index map, insert into the dirty set, return mutable ref.
macro_rules! impl_get_index_mut_dirty {
    ($method_name:ident, $index_map:ident, $dirty_set:ident, $index_type:ty) => {
        /// Get index (mutable), auto-marking it as dirty for checkpoint persistence.
        pub fn $method_name(&mut self, name: &str) -> Option<&mut $index_type> {
            if self.$index_map.contains_key(name) {
                self.$dirty_set.insert(name.to_string());
            }
            self.$index_map.get_mut(name)
        }
    };
}

/// Generate a `dirty_*_index_names` method that collects dirty index names.
///
/// All index types use the same HashSet-based dirty tracking.
macro_rules! impl_dirty_index_names {
    ($method_name:ident, $dirty_set:ident) => {
        /// Get names of dirty indexes for per-index flush.
        pub fn $method_name(&self) -> Vec<String> {
            self.$dirty_set.iter().cloned().collect()
        }
    };
}

/// Generate a `mark_*_dirty` method that inserts a name into the dirty set.
macro_rules! impl_mark_dirty {
    ($method_name:ident, $dirty_set:ident) => {
        /// Mark an index as dirty (modified since last flush).
        pub fn $method_name(&mut self, name: &str) {
            self.$dirty_set.insert(name.to_string());
        }
    };
}

/// Index Manager - manages all indexes for a collection
pub struct IndexManager {
    btree_indexes: HashMap<String, BPlusTree>,
    legacy_indexes: HashMap<String, Index>,
    /// Fuzzy text indexes for similarity search
    fuzzy_indexes: HashMap<String, FuzzyIndex>,
    /// Full-text search indexes with TF-IDF scoring
    fulltext_indexes: HashMap<String, FulltextIndex>,
    /// HNSW vector indexes for similarity search
    vector_indexes: HashMap<String, HnswIndex>,
    /// File paths for persistent indexes (for two-phase commit)
    index_file_paths: HashMap<String, PathBuf>,
    /// Dirty tracking for B+ tree indexes (modified since last flush)
    dirty_btree_indexes: HashSet<String>,
    /// Dirty tracking for fulltext indexes
    dirty_fulltext_indexes: HashSet<String>,
    /// Dirty tracking for fuzzy indexes
    dirty_fuzzy_indexes: HashSet<String>,
    /// Dirty tracking for vector indexes (consolidates HNSW internal dirty flag)
    dirty_vector_indexes: HashSet<String>,
}

impl IndexManager {
    /// Check if an index with this name already exists in ANY index type
    ///
    /// This prevents cross-type name collisions (e.g., a fuzzy index with the
    /// same name as a vector index).
    fn index_exists(&self, name: &str) -> bool {
        self.btree_indexes.contains_key(name)
            || self.legacy_indexes.contains_key(name)
            || self.fuzzy_indexes.contains_key(name)
            || self.fulltext_indexes.contains_key(name)
            || self.vector_indexes.contains_key(name)
    }

    pub fn new() -> Self {
        IndexManager {
            btree_indexes: HashMap::new(),
            legacy_indexes: HashMap::new(),
            fuzzy_indexes: HashMap::new(),
            fulltext_indexes: HashMap::new(),
            vector_indexes: HashMap::new(),
            index_file_paths: HashMap::new(),
            dirty_btree_indexes: HashSet::new(),
            dirty_fulltext_indexes: HashSet::new(),
            dirty_fuzzy_indexes: HashSet::new(),
            dirty_vector_indexes: HashSet::new(),
        }
    }

    /// Check if any indexes need flushing
    pub fn has_dirty_indexes(&self) -> bool {
        !self.dirty_btree_indexes.is_empty()
            || !self.dirty_fulltext_indexes.is_empty()
            || !self.dirty_fuzzy_indexes.is_empty()
            || !self.dirty_vector_indexes.is_empty()
    }

    impl_mark_dirty!(mark_btree_dirty, dirty_btree_indexes);
    impl_mark_dirty!(mark_fulltext_dirty, dirty_fulltext_indexes);
    impl_mark_dirty!(mark_fuzzy_dirty, dirty_fuzzy_indexes);
    impl_mark_dirty!(mark_vector_dirty, dirty_vector_indexes);

    /// Set file path for an index (required for two-phase commit)
    pub fn set_index_path(&mut self, index_name: &str, path: PathBuf) {
        self.index_file_paths.insert(index_name.to_string(), path);
    }

    /// Get file path for an index
    pub fn get_index_path(&self, index_name: &str) -> Option<&PathBuf> {
        self.index_file_paths.get(index_name)
    }

    /// Create B+ tree index (single field)
    ///
    /// The index is created with `building=true` to prevent the query planner
    /// from using it until it's fully populated. Call `set_index_ready()` after
    /// population is complete.
    ///
    /// # Arguments
    /// * `name` - Index name
    /// * `field` - Field to index
    /// * `unique` - Whether values must be unique
    /// * `sparse` - If true, documents missing the field are not indexed
    pub fn create_btree_index(
        &mut self,
        name: String,
        field: String,
        unique: bool,
        sparse: bool,
    ) -> Result<()> {
        if self.index_exists(&name) {
            return Err(IronBaseError::IndexError(format!(
                "Index already exists: {}",
                name
            )));
        }

        let mut tree = BPlusTree::new(name.clone(), field, unique, sparse);
        // Mark as building - caller must call set_index_ready() after population
        tree.set_building(true);
        self.btree_indexes.insert(name, tree);
        Ok(())
    }

    /// Create case-insensitive B+ tree index (single field)
    ///
    /// String values are lowercased before indexing for case-insensitive matching.
    /// The index is created with `building=true`. Call `set_index_ready()` after population.
    ///
    /// # Arguments
    /// * `name` - Index name (typically "{collection}_{field}_ci")
    /// * `field` - Field to index
    /// * `unique` - Whether values must be unique (after lowercasing)
    pub fn create_btree_index_ci(
        &mut self,
        name: String,
        field: String,
        unique: bool,
    ) -> Result<()> {
        if self.index_exists(&name) {
            return Err(IronBaseError::IndexError(format!(
                "Index already exists: {}",
                name
            )));
        }

        let mut tree = BPlusTree::new_ci(name.clone(), field, unique);
        tree.set_building(true);
        self.btree_indexes.insert(name, tree);
        Ok(())
    }

    /// Create compound B+ tree index (multiple fields)
    ///
    /// The index is created with `building=true`. Call `set_index_ready()` after population.
    ///
    /// # Arguments
    /// * `name` - Index name
    /// * `fields` - Ordered list of fields (e.g., ["country", "city"])
    /// * `unique` - Whether the compound key must be unique
    /// * `sparse` - If true, documents missing any field are not indexed
    ///
    /// # Example
    /// ```rust,ignore
    /// manager.create_compound_index(
    ///     "users_location".to_string(),
    ///     vec!["country".to_string(), "city".to_string()],
    ///     false,
    ///     false
    /// )?;
    /// ```
    pub fn create_compound_index(
        &mut self,
        name: String,
        fields: Vec<String>,
        unique: bool,
        sparse: bool,
    ) -> Result<()> {
        if self.index_exists(&name) {
            return Err(IronBaseError::IndexError(format!(
                "Index already exists: {}",
                name
            )));
        }

        if fields.is_empty() {
            return Err(IronBaseError::IndexError(
                "Compound index must have at least one field".to_string(),
            ));
        }

        let mut tree = BPlusTree::new_compound(name.clone(), fields, unique, sparse);
        tree.set_building(true);
        self.btree_indexes.insert(name, tree);
        Ok(())
    }

    /// Create legacy HashMap index
    pub fn create_index(&mut self, definition: IndexDefinition) -> Result<()> {
        let name = definition.name.clone();

        if self.index_exists(&name) {
            return Err(IronBaseError::IndexError(format!(
                "Index already exists: {}",
                name
            )));
        }

        self.legacy_indexes.insert(name, Index::new(definition));
        Ok(())
    }

    /// Drop index by name (supports B+ tree, legacy, fuzzy, fulltext, and vector indexes)
    pub fn drop_index(&mut self, name: &str) -> Result<()> {
        let removed = self.btree_indexes.remove(name).is_some()
            || self.legacy_indexes.remove(name).is_some()
            || self.fuzzy_indexes.remove(name).is_some()
            || self.fulltext_indexes.remove(name).is_some()
            || self.vector_indexes.remove(name).is_some();

        if !removed {
            return Err(IronBaseError::IndexError(format!(
                "Index not found: {}",
                name
            )));
        }
        // Also remove file path if it exists
        self.index_file_paths.remove(name);
        // Clean up dirty sets to prevent phantom dirty names during flush
        self.dirty_btree_indexes.remove(name);
        self.dirty_fuzzy_indexes.remove(name);
        self.dirty_fulltext_indexes.remove(name);
        self.dirty_vector_indexes.remove(name);
        Ok(())
    }

    /// Get B+ tree index
    pub fn get_btree_index(&self, name: &str) -> Option<&BPlusTree> {
        self.btree_indexes.get(name)
    }

    impl_get_index_mut_dirty!(
        get_btree_index_mut,
        btree_indexes,
        dirty_btree_indexes,
        BPlusTree
    );

    /// Add a pre-loaded BPlusTree index (from .idx file)
    pub fn add_loaded_index(&mut self, tree: BPlusTree) {
        let name = tree.metadata.name.clone();
        self.btree_indexes.insert(name, tree);
    }

    /// Get legacy index
    pub fn get_index(&self, name: &str) -> Option<&Index> {
        self.legacy_indexes.get(name)
    }

    /// Get legacy index (mutable)
    pub fn get_index_mut(&mut self, name: &str) -> Option<&mut Index> {
        self.legacy_indexes.get_mut(name)
    }

    /// List all index names
    pub fn list_indexes(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .btree_indexes
            .keys()
            .chain(self.legacy_indexes.keys())
            .chain(self.fuzzy_indexes.keys())
            .chain(self.fulltext_indexes.keys())
            .chain(self.vector_indexes.keys())
            .cloned()
            .collect();
        names.sort();
        names
    }

    /// List all indexes with their first field info (for QueryPlanner)
    ///
    /// Returns tuples of (index_name, first_field) where first_field is:
    /// - The field name for single-field indexes
    /// - The FIRST field for compound indexes (enables prefix queries!)
    ///
    /// For compound indexes, prefix queries use range scans internally.
    pub fn list_indexes_with_prefix_field(&self) -> Vec<(String, String)> {
        self.list_indexes_with_compound_info()
            .into_iter()
            .map(|info| (info.index_name, info.prefix_field))
            .collect()
    }

    /// List all indexes with full compound index information (for QueryPlanner v2)
    ///
    /// Returns `IndexPrefixInfo` for each index, including:
    /// - Single-field indexes: `is_compound = false`, `num_fields = 1`
    /// - Compound indexes: `is_compound = true`, `num_fields > 1`
    /// - Sparse indexes: `sparse = true` (only index documents where field exists)
    ///
    /// Compound indexes can be used for prefix queries via range scans.
    /// Sparse indexes can be used for $exists: true queries.
    pub fn list_indexes_with_compound_info(&self) -> Vec<IndexPrefixInfo> {
        let mut result: Vec<IndexPrefixInfo> = Vec::new();

        for (name, index) in &self.btree_indexes {
            // CRITICAL: Skip indexes that are currently being built
            // Using a partially-built index would return incomplete results!
            if index.metadata.building {
                continue;
            }
            let is_compound = index.metadata.is_compound();
            let prefix_field = if is_compound {
                // For compound indexes, return the first field to enable prefix queries
                index.metadata.fields.first().cloned().unwrap_or_default()
            } else {
                index.metadata.field.clone()
            };
            let num_fields = if is_compound {
                index.metadata.fields.len()
            } else {
                1
            };
            result.push(IndexPrefixInfo {
                index_name: name.clone(),
                prefix_field,
                is_compound,
                num_fields,
                sparse: index.metadata.sparse,
                distinct_count: index.metadata.stats.distinct_count,
                building: false, // Only ready indexes reach here
                num_keys: index.metadata.num_keys,
                case_insensitive: index.metadata.case_insensitive,
                null_count: index.metadata.stats.null_count,
                multikey_ratio: index.metadata.stats.multikey_ratio,
                histogram: index.metadata.stats.histogram.clone(),
                mcv: index.metadata.stats.mcv.clone(),
            });
        }

        // Legacy indexes are single-field only (never sparse, no stats, never building)
        for (name, index) in &self.legacy_indexes {
            result.push(IndexPrefixInfo {
                index_name: name.clone(),
                prefix_field: index.definition.field.clone(),
                is_compound: false,
                num_fields: 1,
                sparse: false,
                distinct_count: 0,       // No stats for legacy indexes
                building: false,         // Legacy indexes are always ready
                num_keys: 0,             // No size info for legacy indexes
                case_insensitive: false, // Legacy indexes are case-sensitive
                null_count: 0,           // No null count for legacy indexes
                multikey_ratio: 0.0,     // No multikey tracking for legacy indexes
                histogram: None,         // No histogram for legacy indexes
                mcv: None,               // No MCV for legacy indexes
            });
        }

        result.sort_by(|a, b| a.index_name.cmp(&b.index_name));
        result
    }

    /// Set an index to "building" state (query planner will ignore it)
    ///
    /// Call this BEFORE starting to populate the index to prevent
    /// queries from using a partially populated index.
    pub fn set_index_building(&mut self, name: &str, building: bool) -> Result<()> {
        if let Some(index) = self.btree_indexes.get_mut(name) {
            index.set_building(building);
            return Ok(());
        }
        if let Some(index) = self.fuzzy_indexes.get_mut(name) {
            index.set_building(building);
            return Ok(());
        }
        if let Some(index) = self.fulltext_indexes.get_mut(name) {
            index.set_building(building);
            return Ok(());
        }
        Err(IronBaseError::IndexError(format!(
            "Index not found: {}",
            name
        )))
    }

    /// Mark an index as ready (query planner can use it)
    ///
    /// Call this AFTER the index has been fully populated.
    pub fn set_index_ready(&mut self, name: &str) -> Result<()> {
        self.set_index_building(name, false)
    }

    // ========== FUZZY INDEX OPERATIONS ==========

    /// Create a fuzzy text index
    ///
    /// # Arguments
    /// * `name` - Index name
    /// * `field` - Field to index
    /// * `algorithm` - Similarity algorithm (JaroWinkler, Levenshtein, DamerauLevenshtein)
    /// * `threshold` - Minimum similarity threshold (0.0-1.0)
    ///
    /// # Example
    /// ```rust,ignore
    /// manager.create_fuzzy_index(
    ///     "name_fuzzy",
    ///     "name",
    ///     FuzzyAlgorithm::JaroWinkler,
    ///     0.8
    /// )?;
    /// ```
    pub fn create_fuzzy_index(
        &mut self,
        name: String,
        field: String,
        algorithm: FuzzyAlgorithm,
        threshold: f64,
    ) -> Result<()> {
        self.create_fuzzy_index_with_storage(name, field, algorithm, threshold, None)
    }

    /// Create fuzzy index with disk-based storage
    ///
    /// The index is created with `building=true`. Call `set_index_ready()` after population.
    ///
    /// # Arguments
    /// * `name` - Unique index name
    /// * `field` - Field to index
    /// * `algorithm` - Fuzzy matching algorithm
    /// * `threshold` - Minimum similarity threshold
    /// * `storage_path` - Path to store the .fzidx file (None = memory-only)
    pub fn create_fuzzy_index_with_storage(
        &mut self,
        name: String,
        field: String,
        algorithm: FuzzyAlgorithm,
        threshold: f64,
        storage_path: Option<PathBuf>,
    ) -> Result<()> {
        if self.index_exists(&name) {
            return Err(IronBaseError::IndexError(format!(
                "Index already exists: {}",
                name
            )));
        }

        let mut index = if let Some(path) = storage_path {
            self.index_file_paths.insert(name.clone(), path.clone());
            FuzzyIndex::new_with_storage(&name, &field, algorithm, threshold, path)
        } else {
            FuzzyIndex::new(&name, &field, algorithm, threshold)
        };
        index.set_building(true);

        self.fuzzy_indexes.insert(name, index);
        Ok(())
    }

    /// Get fuzzy index
    pub fn get_fuzzy_index(&self, name: &str) -> Option<&FuzzyIndex> {
        self.fuzzy_indexes.get(name)
    }

    impl_get_index_mut_dirty!(
        get_fuzzy_index_mut,
        fuzzy_indexes,
        dirty_fuzzy_indexes,
        FuzzyIndex
    );

    /// Get fuzzy index for a field (if one exists)
    pub fn get_fuzzy_index_for_field(&self, field: &str) -> Option<&FuzzyIndex> {
        self.fuzzy_indexes
            .values()
            .find(|idx| idx.metadata.field == field)
    }

    /// List all fuzzy indexes
    pub fn list_fuzzy_indexes(&self) -> Vec<&FuzzyIndex> {
        self.fuzzy_indexes.values().collect()
    }

    /// Add a pre-loaded FuzzyIndex
    pub fn add_loaded_fuzzy_index(&mut self, index: FuzzyIndex) {
        let name = index.metadata.name.clone();
        self.fuzzy_indexes.insert(name, index);
    }

    // ========== FULL-TEXT INDEX METHODS ==========

    /// Create full-text search index with language support
    ///
    /// # Arguments
    /// * `name` - Unique index name
    /// * `field` - Field to index (supports dot notation)
    /// * `language` - Language for stemming and stop words
    /// * `min_word_length` - Minimum word length to index (default: 2)
    /// * `accent_folding` - Whether to apply accent folding (default: true)
    pub fn create_fulltext_index(
        &mut self,
        name: String,
        field: String,
        language: FtsLanguage,
        min_word_length: Option<usize>,
        accent_folding: Option<bool>,
    ) -> Result<()> {
        self.create_fulltext_index_with_storage(
            name,
            field,
            language,
            min_word_length,
            accent_folding,
            None,
        )
    }

    /// Create full-text search index with disk-based storage
    ///
    /// The index is created with `building=true`. Call `set_index_ready()` after population.
    ///
    /// # Arguments
    /// * `name` - Unique index name
    /// * `field` - Field to index (supports dot notation)
    /// * `language` - Language for stemming and stop words
    /// * `min_word_length` - Minimum word length to index (default: 2)
    /// * `accent_folding` - Whether to apply accent folding (default: true)
    /// * `storage_path` - Path to store the .ftidx file (None = memory-only)
    pub fn create_fulltext_index_with_storage(
        &mut self,
        name: String,
        field: String,
        language: FtsLanguage,
        min_word_length: Option<usize>,
        accent_folding: Option<bool>,
        storage_path: Option<PathBuf>,
    ) -> Result<()> {
        if self.index_exists(&name) {
            return Err(IronBaseError::IndexError(format!(
                "Index already exists: {}",
                name
            )));
        }

        let options = FtsOptions::with_settings(
            language,
            min_word_length.unwrap_or(2),
            accent_folding.unwrap_or(true),
        );

        let mut index = if let Some(path) = storage_path {
            self.index_file_paths.insert(name.clone(), path.clone());
            FulltextIndex::new_with_storage(&name, &field, options, path)?
        } else {
            FulltextIndex::new(&name, &field, options)
        };
        index.set_building(true);
        self.fulltext_indexes.insert(name, index);
        Ok(())
    }

    /// Get fulltext index by name
    pub fn get_fulltext_index(&self, name: &str) -> Option<&FulltextIndex> {
        self.fulltext_indexes.get(name)
    }

    impl_get_index_mut_dirty!(
        get_fulltext_index_mut,
        fulltext_indexes,
        dirty_fulltext_indexes,
        FulltextIndex
    );

    /// Get fulltext index for a field (if one exists)
    pub fn get_fulltext_index_for_field(&self, field: &str) -> Option<&FulltextIndex> {
        self.fulltext_indexes
            .values()
            .find(|idx| idx.field == field)
    }

    /// List all fulltext indexes
    pub fn list_fulltext_indexes(&self) -> Vec<&FulltextIndex> {
        self.fulltext_indexes.values().collect()
    }

    /// Drop a fulltext index by name
    ///
    /// Returns Ok(()) if the index was removed, Err if not found.
    /// Used for cleanup on failed index creation.
    pub fn drop_fulltext_index(&mut self, name: &str) -> Result<()> {
        if self.fulltext_indexes.remove(name).is_some() {
            self.index_file_paths.remove(name);
            // Clean up dirty set to prevent phantom dirty name during flush
            self.dirty_fulltext_indexes.remove(name);
            Ok(())
        } else {
            Err(IronBaseError::IndexError(format!(
                "Fulltext index not found: {}",
                name
            )))
        }
    }

    /// Check if any index in this manager has unique constraint
    ///
    /// Used for hybrid locking: collections with unique indexes use collection-level lock,
    /// collections without unique indexes can use per-document locking for better concurrency.
    pub fn has_unique_index(&self) -> bool {
        self.btree_indexes.values().any(|idx| idx.metadata.unique)
    }

    /// Add a pre-loaded FulltextIndex
    pub fn add_loaded_fulltext_index(&mut self, index: FulltextIndex) {
        let name = index.name.clone();
        self.fulltext_indexes.insert(name, index);
    }

    /// Flush dirty fulltext indexes to disk
    ///
    /// This should be called before database close or during checkpoint
    /// to persist fulltext index changes to .ftidx files.
    /// Only indexes marked as dirty are written.
    ///
    /// Returns the number of indexes flushed.
    /// On error, dirty flags are preserved for retry on next flush.
    pub fn flush_fulltext_indexes(&mut self, watermark: u64) -> Result<usize> {
        // Collect names first to avoid borrowing issues
        let dirty_names: Vec<String> = self.dirty_fulltext_indexes.iter().cloned().collect();
        let mut count = 0;

        for name in dirty_names {
            if let Some(index) = self.fulltext_indexes.get_mut(&name) {
                index.set_flushed_tx_id(watermark);
                index.save_to_file()?;
                // Only remove from dirty set AFTER successful write
                self.dirty_fulltext_indexes.remove(&name);
                count += 1;
            }
        }
        Ok(count)
    }

    /// Flush dirty fuzzy indexes to .fzidx files
    ///
    /// This should be called before database close to enable fast restart
    /// (clean shutdown optimization - skip fuzzy index rebuild on next open).
    /// Only indexes marked as dirty are written.
    ///
    /// Returns the number of indexes flushed.
    /// On error, dirty flags are preserved for retry on next flush.
    pub fn flush_fuzzy_indexes(&mut self, watermark: u64) -> Result<usize> {
        // Collect names first to avoid borrowing issues
        let dirty_names: Vec<String> = self.dirty_fuzzy_indexes.iter().cloned().collect();
        let mut count = 0;

        for name in dirty_names {
            if let Some(index) = self.fuzzy_indexes.get_mut(&name) {
                if index.storage_path().is_some() {
                    index.set_flushed_tx_id(watermark);
                    index.save_to_file()?;
                    // Only remove from dirty set AFTER successful write
                    self.dirty_fuzzy_indexes.remove(&name);
                    count += 1;
                }
            }
        }
        Ok(count)
    }

    /// Flush dirty B+ tree indexes to .idx files
    ///
    /// This should be called before database close to enable fast restart
    /// (clean shutdown optimization - skip index rebuild on next open).
    /// Only indexes marked as dirty are written.
    ///
    /// Returns the number of indexes flushed.
    /// On error, dirty flags are preserved for retry on next flush.
    pub fn flush_btree_indexes(&mut self, db_path: &str, watermark: u64) -> Result<usize> {
        use crate::collection_core::persist_index_to_disk;

        // Collect names first to avoid borrowing issues
        let dirty_names: Vec<String> = self.dirty_btree_indexes.iter().cloned().collect();
        let mut count = 0;

        for name in dirty_names {
            if let Some(tree) = self.btree_indexes.get_mut(&name) {
                tree.set_flushed_tx_id(watermark);
                persist_index_to_disk(db_path, &name, |file| tree.save_to_file(file))?;
                // Only remove from dirty set AFTER successful write
                self.dirty_btree_indexes.remove(&name);
                count += 1;
            }
        }
        Ok(count)
    }

    // ========== VECTOR INDEX METHODS ==========

    /// Create HNSW vector index for similarity search
    ///
    /// The index is created with dirty=true. Call `flush_vector_indexes()` before
    /// database close to persist to .hnsw files.
    ///
    /// # Arguments
    /// * `name` - Unique index name (e.g., "collection_vec_field")
    /// * `field` - Field containing the embedding vectors
    /// * `config` - Vector index configuration (dimension, metric, HNSW params)
    /// * `storage_path` - Path to store the .hnsw cache file (None = memory-only)
    pub fn create_vector_index(
        &mut self,
        name: String,
        _field: String,
        config: VectorIndexConfig,
        storage_path: Option<PathBuf>,
    ) -> Result<()> {
        if self.index_exists(&name) {
            return Err(IronBaseError::IndexError(format!(
                "Index already exists: {}",
                name
            )));
        }

        // Validate config
        config
            .validate()
            .map_err(|e| IronBaseError::IndexError(e.to_string()))?;

        let index = HnswIndex::new(config);

        if let Some(path) = storage_path {
            self.index_file_paths.insert(name.clone(), path);
        }

        // Store field in the path map for metadata (we use naming convention)
        // The field is embedded in the index name: {collection}_vec_{field}
        self.vector_indexes.insert(name, index);
        Ok(())
    }

    /// Get vector index by name
    pub fn get_vector_index(&self, name: &str) -> Option<&HnswIndex> {
        self.vector_indexes.get(name)
    }

    /// Get vector index by name (mutable), auto-marking as dirty
    pub fn get_vector_index_mut(&mut self, name: &str) -> Option<&mut HnswIndex> {
        if self.vector_indexes.contains_key(name) {
            self.dirty_vector_indexes.insert(name.to_string());
        }
        self.vector_indexes.get_mut(name)
    }

    /// Get vector index for a field (if one exists)
    ///
    /// # Arguments
    /// * `collection_name` - Collection name (for building index name)
    /// * `field` - Field name to search for
    pub fn get_vector_index_for_field(
        &self,
        collection_name: &str,
        field: &str,
    ) -> Option<&HnswIndex> {
        let expected_name = format!("{}_vec_{}", collection_name, field);
        self.vector_indexes.get(&expected_name)
    }

    /// Get vector index for a field (mutable)
    pub fn get_vector_index_for_field_mut(
        &mut self,
        collection_name: &str,
        field: &str,
    ) -> Option<&mut HnswIndex> {
        let expected_name = format!("{}_vec_{}", collection_name, field);
        if self.vector_indexes.contains_key(&expected_name) {
            self.dirty_vector_indexes.insert(expected_name.clone());
        }
        self.vector_indexes.get_mut(&expected_name)
    }

    /// List all vector indexes
    pub fn list_vector_indexes(&self) -> Vec<&HnswIndex> {
        self.vector_indexes.values().collect()
    }

    /// Add a pre-loaded HnswIndex (from .hnsw file)
    pub fn add_loaded_vector_index(&mut self, name: String, index: HnswIndex) {
        self.vector_indexes.insert(name, index);
    }

    /// Drop a vector index by name
    pub fn drop_vector_index(&mut self, name: &str) -> Result<()> {
        if self.vector_indexes.remove(name).is_some() {
            self.index_file_paths.remove(name);
            self.dirty_vector_indexes.remove(name);
            Ok(())
        } else {
            Err(IronBaseError::IndexError(format!(
                "Vector index not found: {}",
                name
            )))
        }
    }

    /// Atomically persist a single HNSW index to its cache path via
    /// temp file + rename. Returns the final file size in bytes.
    ///
    /// Writes to `{cache_path}.hnsw.tmp`, fsyncs, then uses
    /// `fs_utils::atomic_rename_and_sync` to swap into place and fsync the
    /// parent directory. A crash mid-write leaves `.hnsw.tmp` orphaned and
    /// the original `.hnsw` untouched (or absent); a crash after rename
    /// leaves the new file durable. Startup cleanup in `storage::mod::open`
    /// removes stale `.hnsw.tmp` files.
    fn persist_hnsw_to_file(index: &HnswIndex, cache_path: &PathBuf) -> Result<u64> {
        use std::io::BufWriter;

        let temp_path = cache_path.with_extension("hnsw.tmp");
        let file = std::fs::File::create(&temp_path).map_err(|e| {
            IronBaseError::Io(std::io::Error::other(format!(
                "Failed to create HNSW temp file '{}': {}",
                temp_path.display(),
                e
            )))
        })?;
        let mut writer = BufWriter::new(file);
        index.save_to_writer(&mut writer)?;
        std::io::Write::flush(&mut writer).map_err(|e| {
            IronBaseError::Io(std::io::Error::other(format!(
                "Failed to flush HNSW temp file '{}': {}",
                temp_path.display(),
                e
            )))
        })?;
        let file = writer.into_inner().map_err(|e| {
            IronBaseError::Io(std::io::Error::other(format!(
                "BufWriter into_inner failed for '{}': {}",
                temp_path.display(),
                e
            )))
        })?;
        file.sync_all().map_err(|e| {
            IronBaseError::Io(std::io::Error::other(format!(
                "Failed to fsync HNSW temp file '{}': {}",
                temp_path.display(),
                e
            )))
        })?;
        drop(file);

        crate::fs_utils::atomic_rename_and_sync(&temp_path, cache_path)?;

        let file_size = cache_path.metadata().map(|m| m.len()).unwrap_or(0);
        Ok(file_size)
    }

    /// Flush all vector indexes to .hnsw files
    ///
    /// This should be called before database close to enable fast restart.
    /// Only dirty indexes (modified since last save) are written.
    ///
    /// Writes go through a temp file + atomic rename so a crash mid-flush
    /// cannot produce a torn `.hnsw` file that loads "successfully" with
    /// garbage.
    ///
    /// Returns the number of indexes flushed.
    pub fn flush_vector_indexes(&mut self, db_path: &str, watermark: u64) -> Result<usize> {
        let mut count = 0;
        let dirty_names: Vec<String> = self.dirty_vector_indexes.iter().cloned().collect();
        for name in dirty_names {
            if let Some(index) = self.vector_indexes.get_mut(&name) {
                index.set_flushed_tx_id(watermark);
                let cache_path = Self::build_vector_cache_path(db_path, &name);
                let file_size = Self::persist_hnsw_to_file(index, &cache_path)?;
                index.mark_clean();
                self.dirty_vector_indexes.remove(&name);
                crate::log_debug!(
                    "Flushed vector index '{}' to {} ({:.1} MB, atomic)",
                    name,
                    cache_path.display(),
                    file_size as f64 / (1024.0 * 1024.0)
                );
                count += 1;
            }
        }
        Ok(count)
    }

    // ========== PER-INDEX FLUSH (for checkpoint lock contention fix) ==========

    impl_dirty_index_names!(dirty_btree_index_names, dirty_btree_indexes);
    impl_dirty_index_names!(dirty_fulltext_index_names, dirty_fulltext_indexes);
    impl_dirty_index_names!(dirty_fuzzy_index_names, dirty_fuzzy_indexes);
    impl_dirty_index_names!(dirty_vector_index_names, dirty_vector_indexes);

    /// Rebuild vector indexes that have excessive orphan nodes.
    ///
    /// Returns the number of indexes rebuilt.
    /// Called during checkpoint to compact HNSW indexes with >30% orphan ratio.
    pub fn rebuild_vector_indexes_if_needed(&mut self) -> Result<usize> {
        let mut rebuilt = 0;
        let names: Vec<String> = self.vector_indexes.keys().cloned().collect();
        for name in names {
            if let Some(index) = self.vector_indexes.get_mut(&name) {
                if index.rebuild_if_needed()? {
                    self.dirty_vector_indexes.insert(name.clone());
                    rebuilt += 1;
                }
            }
        }
        Ok(rebuilt)
    }

    /// Rebuild ALL non-empty vector indexes **unconditionally**.
    ///
    /// Used by `db_compact` (the explicit/auto heavy maintenance op) to fully
    /// rebuild the HNSW graphs from their active vectors. Unlike the checkpoint
    /// path (`rebuild_vector_indexes_if_needed`, orphan-ratio gated), this does
    /// NOT gate on orphan count: a graph can be degraded (poor connectivity, e.g.
    /// built by an older/buggy version) with zero orphans, and only a forced
    /// rebuild repairs it. `rebuild()` reconstructs from `id_to_index` (the active
    /// vectors), so it both removes orphans and re-links the graph correctly.
    /// Returns the number of indexes rebuilt.
    pub fn rebuild_all_vector_indexes(&mut self) -> Result<usize> {
        let mut rebuilt = 0;
        let names: Vec<String> = self.vector_indexes.keys().cloned().collect();
        for name in names {
            if let Some(index) = self.vector_indexes.get_mut(&name) {
                let active = index.len();
                if active == 0 {
                    continue; // nothing to rebuild
                }
                let orphans = index.orphan_count();
                index.rebuild()?;
                self.dirty_vector_indexes.insert(name.clone());
                crate::log_info!(
                    "HNSW index '{}' rebuilt during compact: {} active vectors, {} orphans removed",
                    name,
                    active,
                    orphans
                );
                rebuilt += 1;
            }
        }
        Ok(rebuilt)
    }

    /// Flush a single fulltext index to disk
    ///
    /// Returns `Ok(true)` if flushed, `Ok(false)` if not dirty or not found.
    /// On error, the dirty flag is preserved for retry.
    ///
    /// `watermark` is the current `DatabaseCore::watermark_tx_id()` —
    /// stamped into the persisted file so crash recovery knows which WAL
    /// Operations need replay (task #26).
    pub fn flush_one_fulltext_index(&mut self, name: &str, watermark: u64) -> Result<bool> {
        if !self.dirty_fulltext_indexes.contains(name) {
            return Ok(false);
        }
        if let Some(index) = self.fulltext_indexes.get_mut(name) {
            index.set_flushed_tx_id(watermark);
            index.save_to_file()?;
            self.dirty_fulltext_indexes.remove(name);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Two-phase flush Phase 1: Take snapshot from a dirty fulltext index.
    ///
    /// Returns None if the index is not dirty or not found.
    /// After this call, the index is in "flush in progress" state:
    /// - inverted_index moved to frozen_inverted (search still works)
    /// - file_handle closed (inserts use doc_tokens_memory)
    ///
    /// `watermark` is stamped into the index before the snapshot so the
    /// persisted file (serialized later in Phase 2) records it (task #26).
    pub(crate) fn take_fulltext_flush_snapshot(
        &mut self,
        name: &str,
        watermark: u64,
    ) -> Option<crate::fulltext::FulltextFlushSnapshot> {
        if !self.dirty_fulltext_indexes.contains(name) {
            return None;
        }
        let index = self.fulltext_indexes.get_mut(name)?;
        index.set_flushed_tx_id(watermark);
        index.take_flush_snapshot()
    }

    /// Two-phase flush error recovery: rollback a failed flush.
    ///
    /// Restores inverted_index from frozen_inverted and re-opens file_handle.
    /// Must be called if serialize_flush() fails to prevent data loss on the
    /// next checkpoint (which would overwrite frozen_inverted).
    pub(crate) fn rollback_fulltext_flush(&mut self, name: &str) {
        if let Some(index) = self.fulltext_indexes.get_mut(name) {
            index.rollback_flush();
        }
    }

    /// Two-phase flush Phase 3: Commit flush results back to the index.
    ///
    /// Installs new token_offsets, re-opens file_handle, flushes buffered
    /// doc_tokens, and conditionally clears the dirty flag.
    ///
    /// The dirty flag is only cleared if `inverted_index` is empty after commit.
    /// During Phase 2 (no lock held), concurrent inserts may have added entries
    /// to `inverted_index` — those entries still need a subsequent flush to persist.
    /// Without this check, a server restart would lose Phase 2 entries because:
    /// 1. dirty cleared → next checkpoint skips fulltext flush
    /// 2. close()/Drop skips flush (not dirty)
    /// 3. restart loads stale .ftidx → Phase 2 entries gone
    pub(crate) fn commit_fulltext_flush(
        &mut self,
        name: &str,
        result: crate::fulltext::FulltextFlushResult,
    ) -> Result<()> {
        if let Some(index) = self.fulltext_indexes.get_mut(name) {
            index.commit_flush(result)?;
            if !index.has_pending_entries() {
                self.dirty_fulltext_indexes.remove(name);
            }
        }
        Ok(())
    }

    /// Flush a single fuzzy index to disk
    ///
    /// Returns `Ok(true)` if flushed, `Ok(false)` if not dirty, no storage path, or not found.
    /// On error, the dirty flag is preserved for retry.
    ///
    /// `watermark` is stamped into the metadata for WAL-replay recovery
    /// (task #26).
    pub fn flush_one_fuzzy_index(&mut self, name: &str, watermark: u64) -> Result<bool> {
        if !self.dirty_fuzzy_indexes.contains(name) {
            return Ok(false);
        }
        if let Some(index) = self.fuzzy_indexes.get_mut(name) {
            if index.storage_path().is_some() {
                index.set_flushed_tx_id(watermark);
                index.save_to_file()?;
                self.dirty_fuzzy_indexes.remove(name);
                Ok(true)
            } else {
                Ok(false)
            }
        } else {
            Ok(false)
        }
    }

    /// Flush a single B+ tree index to disk
    ///
    /// Returns `Ok(true)` if flushed, `Ok(false)` if not dirty or not found.
    /// On error, the dirty flag is preserved for retry.
    pub fn flush_one_btree_index(
        &mut self,
        name: &str,
        db_path: &str,
        watermark: u64,
    ) -> Result<bool> {
        use crate::collection_core::persist_index_to_disk;

        if !self.dirty_btree_indexes.contains(name) {
            return Ok(false);
        }
        if let Some(tree) = self.btree_indexes.get_mut(name) {
            tree.set_flushed_tx_id(watermark);
            persist_index_to_disk(db_path, name, |file| tree.save_to_file(file))?;
            self.dirty_btree_indexes.remove(name);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Flush a single vector index to disk
    ///
    /// Returns `Ok(true)` if flushed, `Ok(false)` if not dirty or not found.
    /// On error, the index remains dirty for retry. Writes via temp file +
    /// atomic rename — see `persist_hnsw_to_file`.
    ///
    /// `watermark` is stamped into the v3 cache file header for WAL-replay
    /// recovery (task #26).
    pub fn flush_one_vector_index(
        &mut self,
        name: &str,
        db_path: &str,
        watermark: u64,
    ) -> Result<bool> {
        if !self.dirty_vector_indexes.contains(name) {
            return Ok(false);
        }
        if let Some(index) = self.vector_indexes.get_mut(name) {
            index.set_flushed_tx_id(watermark);
            let cache_path = Self::build_vector_cache_path(db_path, name);
            let file_size = Self::persist_hnsw_to_file(index, &cache_path)?;
            index.mark_clean();
            self.dirty_vector_indexes.remove(name);
            crate::log_debug!(
                "Flushed vector index '{}' to {} ({:.1} MB, atomic)",
                name,
                cache_path.display(),
                file_size as f64 / (1024.0 * 1024.0)
            );
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Build the path for HNSW cache file
    pub fn build_vector_cache_path(db_path: &str, index_name: &str) -> PathBuf {
        let db_path_buf = PathBuf::from(db_path);
        let parent = db_path_buf.parent().unwrap_or(&db_path_buf);
        parent.join(format!("{}.hnsw", index_name))
    }

    /// Try to load vector index from .hnsw cache file
    ///
    /// Returns Some(HnswIndex) if loaded successfully, None if file doesn't exist or is corrupt.
    pub fn try_load_vector_index(db_path: &str, index_name: &str) -> Option<HnswIndex> {
        let cache_path = Self::build_vector_cache_path(db_path, index_name);
        if !cache_path.exists() {
            return None;
        }

        match std::fs::read(&cache_path) {
            Ok(bytes) => match HnswIndex::from_bytes(&bytes) {
                Ok(index) => {
                    crate::log_debug!(
                        "Loaded vector index '{}' from {} ({} vectors)",
                        index_name,
                        cache_path.display(),
                        index.len()
                    );
                    Some(index)
                }
                Err(e) => {
                    crate::log_warn!(
                        "Failed to deserialize vector index '{}' from {}: {}",
                        index_name,
                        cache_path.display(),
                        e
                    );
                    None
                }
            },
            Err(e) => {
                crate::log_warn!(
                    "Failed to read vector index file {}: {}",
                    cache_path.display(),
                    e
                );
                None
            }
        }
    }

    // ========== INDEX STATISTICS ==========

    /// Refresh statistics for all B+ tree indexes.
    ///
    /// This method traverses all leaf nodes in each index and computes:
    /// - `distinct_count`: number of unique key values
    /// - `null_count`: number of null key values
    ///
    /// The query planner uses these statistics to estimate selectivity
    /// and choose the best index for a query.
    ///
    /// # Performance
    /// - Time: O(n) per index where n = total keys
    /// - Memory: O(k) per index where k = number of leaf nodes (pointers only)
    pub fn refresh_all_stats(&mut self) {
        for (_, index) in self.btree_indexes.iter_mut() {
            index.refresh_stats();
        }
    }

    /// Refresh statistics for a specific B+ tree index.
    ///
    /// # Arguments
    /// * `index_name` - Name of the index to refresh
    ///
    /// # Errors
    /// Returns `IndexError` if the index does not exist.
    pub fn refresh_index_stats(&mut self, index_name: &str) -> Result<()> {
        if let Some(index) = self.btree_indexes.get_mut(index_name) {
            index.refresh_stats();
            return Ok(());
        }
        Err(IronBaseError::IndexError(format!(
            "Index not found: {}",
            index_name
        )))
    }

    /// Check for stale statistics and refresh if needed
    ///
    /// Uses staleness heuristic: >10% change OR >10K writes since last refresh.
    /// Should be called after batch operations for automatic statistics maintenance.
    ///
    /// # Returns
    /// Number of indexes that were refreshed
    pub fn check_and_refresh_stale_stats(&mut self) -> usize {
        let mut refreshed_count = 0;

        for (name, index) in self.btree_indexes.iter_mut() {
            if index.needs_stats_refresh() {
                tracing::debug!(
                    "Auto-refreshing stats for index '{}' (staleness: {:.2})",
                    name,
                    index.staleness_ratio()
                );
                index.refresh_stats();
                refreshed_count += 1;
            }
        }

        refreshed_count
    }

    // ========== LAZY LOADING MONITORING ==========

    /// Get total memory usage across all indexes (bytes)
    ///
    /// Uses the `LazyLoadable::memory_usage_bytes()` trait method for each index.
    pub fn total_memory_usage(&self) -> usize {
        use super::traits::IndexTrait;

        let btree_mem: usize = self
            .btree_indexes
            .values()
            .map(|idx| idx.memory_usage_bytes())
            .sum();

        let fuzzy_mem: usize = self
            .fuzzy_indexes
            .values()
            .map(|idx| idx.memory_usage_bytes())
            .sum();

        let fulltext_mem: usize = self
            .fulltext_indexes
            .values()
            .map(|idx| idx.memory_usage_bytes())
            .sum();

        let vector_mem: usize = self
            .vector_indexes
            .values()
            .map(|idx| idx.memory_usage_bytes())
            .sum();

        btree_mem + fuzzy_mem + fulltext_mem + vector_mem
    }

    /// Count indexes currently in lazy mode (not fully loaded)
    pub fn lazy_index_count(&self) -> usize {
        use super::traits::LazyLoadable;

        let btree_lazy = self
            .btree_indexes
            .values()
            .filter(|idx| idx.is_lazy_mode())
            .count();

        let fuzzy_lazy = self
            .fuzzy_indexes
            .values()
            .filter(|idx| idx.is_lazy_mode())
            .count();

        let fulltext_lazy = self
            .fulltext_indexes
            .values()
            .filter(|idx| idx.is_lazy_mode())
            .count();

        // HNSW doesn't support lazy loading yet
        btree_lazy + fuzzy_lazy + fulltext_lazy
    }

    /// Get total index count across all types
    pub fn total_index_count(&self) -> usize {
        self.btree_indexes.len()
            + self.fuzzy_indexes.len()
            + self.fulltext_indexes.len()
            + self.vector_indexes.len()
    }

    /// Log lazy loading status (for startup diagnostics)
    pub fn log_lazy_status(&self) {
        let total = self.total_index_count();
        let lazy = self.lazy_index_count();
        let mem_mb = self.total_memory_usage() / (1024 * 1024);

        tracing::info!(
            total_indexes = total,
            lazy_indexes = lazy,
            loaded_indexes = total - lazy,
            memory_mb = mem_mb,
            "IndexManager initialized"
        );
    }

    // ========== CENTRALIZED INDEX OPERATIONS (FIX #19) ==========

    /// Add a document to all indexes (B+ tree and fuzzy)
    ///
    /// Properly handles both single-field and compound indexes using extract_key().
    /// For unique indexes: includes null keys (MongoDB treats null as a value).
    /// For non-unique indexes: skips null keys (no query benefit).
    /// For fuzzy indexes: only indexes string values.
    ///
    /// # Arguments
    /// * `doc` - The document as JSON Value
    /// * `doc_id` - The document ID
    /// * `exclude_index` - Optional index name to skip (e.g., "_id" index handled separately)
    pub fn add_document_to_indexes(
        &mut self,
        doc: &serde_json::Value,
        doc_id: &DocumentId,
        exclude_index: Option<&str>,
    ) -> Result<()> {
        // B+ tree indexes
        let index_names: Vec<String> = self.btree_indexes.keys().cloned().collect();

        for index_name in index_names {
            if let Some(excluded) = exclude_index {
                if index_name == excluded {
                    continue;
                }
            }

            if let Some(index) = self.btree_indexes.get_mut(&index_name) {
                if !index.metadata.multikey {
                    let has_array = if index.metadata.is_compound() {
                        index
                            .metadata
                            .fields
                            .iter()
                            .any(|field| path_crosses_array(doc, field))
                    } else {
                        path_crosses_array(doc, &index.metadata.field)
                    };
                    if has_array {
                        index.metadata.multikey = true;
                    }
                }
                if index.metadata.is_compound()
                    && Self::count_compound_array_fields(doc, &index.metadata.fields) > 1
                {
                    return Err(IronBaseError::IndexError(format!(
                        "Compound index '{}' cannot index multiple array fields: {}",
                        index.metadata.name,
                        index.metadata.fields.join(", ")
                    )));
                }
                if index.metadata.sparse
                    && index.metadata.is_compound()
                    && index
                        .metadata
                        .fields
                        .iter()
                        .any(|field| get_all_nested_values(doc, field).is_empty())
                {
                    continue;
                }

                let keys = index.extract_keys(doc);
                let mut seen = HashSet::new();
                for index_key in keys {
                    if !seen.insert(index_key.clone()) {
                        continue;
                    }
                    let is_null = Self::is_key_all_null(&index_key);

                    // Sparse indexes: NEVER include null keys (that's the whole point)
                    // Non-sparse unique indexes: include null keys (null is a value, enforce uniqueness)
                    // Non-sparse non-unique indexes: skip null keys (no query benefit)
                    let should_index =
                        !is_null || (index.metadata.unique && !index.metadata.sparse);
                    if should_index {
                        index.insert(index_key, doc_id.clone())?;
                        // Mark dirty for checkpoint persistence
                        self.dirty_btree_indexes.insert(index_name.clone());
                    }
                }
            }
        }

        // Fuzzy indexes
        let fuzzy_names: Vec<String> = self.fuzzy_indexes.keys().cloned().collect();

        for index_name in fuzzy_names {
            if let Some(excluded) = exclude_index {
                if index_name == excluded {
                    continue;
                }
            }

            if let Some(index) = self.fuzzy_indexes.get_mut(&index_name) {
                // Get field value - only index string values
                if let Some(value) = get_nested_value(doc, &index.metadata.field) {
                    if let Some(s) = value.as_str() {
                        index.insert(s, doc_id.clone());
                        // Mark dirty for checkpoint persistence
                        self.dirty_fuzzy_indexes.insert(index_name.clone());
                    }
                }
            }
        }

        // Fulltext indexes
        let fulltext_names: Vec<String> = self.fulltext_indexes.keys().cloned().collect();

        for index_name in fulltext_names {
            if let Some(excluded) = exclude_index {
                if index_name == excluded {
                    continue;
                }
            }

            if let Some(index) = self.fulltext_indexes.get_mut(&index_name) {
                // Get field value - only index string values
                if let Some(value) = get_nested_value(doc, &index.field) {
                    if let Some(s) = value.as_str() {
                        // Extract parent doc_id for chunk→document mapping (RAG collections)
                        let pdid_field = index.parent_doc_id_field().to_string();
                        let parent_doc_id = get_nested_value(doc, &pdid_field)
                            .and_then(|v| v.as_str().map(|s| s.to_string()));
                        // Log error but continue - document already persisted
                        let insert_result = if let Some(ref pdid) = parent_doc_id {
                            index.insert_with_parent_doc_id(doc_id, s, pdid)
                        } else {
                            index.insert(doc_id, s)
                        };
                        if let Err(e) = insert_result {
                            log_error!(
                                "Failed to insert into fulltext index '{}': {:?}",
                                index_name,
                                e
                            );
                        } else {
                            // Mark dirty only if insert succeeded
                            self.dirty_fulltext_indexes.insert(index_name.clone());
                        }
                    }
                }
            }
        }

        // Vector indexes - auto-index documents with vector fields
        let vector_names: Vec<String> = self.vector_indexes.keys().cloned().collect();

        for index_name in vector_names {
            if let Some(excluded) = exclude_index {
                if index_name == excluded {
                    continue;
                }
            }

            if let Some(index) = self.vector_indexes.get_mut(&index_name) {
                let field = &index.config().field;
                if field.is_empty() {
                    continue; // Skip if no field configured (legacy index)
                }

                // Get vector field value
                if let Some(value) = get_nested_value(doc, field) {
                    if let Some(arr) = value.as_array() {
                        // Convert to f32 vector
                        let vector: Vec<f32> = arr
                            .iter()
                            .filter_map(|v| v.as_f64().map(|f| f as f32))
                            .collect();

                        // Check dimension matches
                        if vector.len() == index.config().dim {
                            let id_str = match doc_id {
                                DocumentId::Int(i) => i.to_string(),
                                DocumentId::String(s) => s.clone(),
                                DocumentId::ObjectId(oid) => oid.clone(),
                            };
                            // Insert into HNSW - log error but don't fail (document already persisted)
                            if let Err(e) = index.insert(&id_str, &vector) {
                                log_error!(
                                    "Failed to insert into vector index '{}': {:?}",
                                    index_name,
                                    e
                                );
                            } else {
                                self.dirty_vector_indexes.insert(index_name.clone());
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Remove a document from all indexes (B+ tree and fuzzy)
    ///
    /// Properly handles both single-field and compound indexes.
    /// For unique indexes: removes null keys (they were inserted).
    /// For non-unique indexes: skips null keys (they weren't inserted).
    /// For fuzzy indexes: removes by document ID.
    ///
    /// # Arguments
    /// * `doc` - The document as JSON Value
    /// * `doc_id` - The document ID
    /// * `exclude_index` - Optional index name to skip
    pub fn remove_document_from_indexes(
        &mut self,
        doc: &serde_json::Value,
        doc_id: &DocumentId,
        exclude_index: Option<&str>,
    ) -> Result<()> {
        // B+ tree indexes
        let index_names: Vec<String> = self.btree_indexes.keys().cloned().collect();

        for index_name in index_names {
            if let Some(excluded) = exclude_index {
                if index_name == excluded {
                    continue;
                }
            }

            if let Some(index) = self.btree_indexes.get_mut(&index_name) {
                if index.metadata.sparse
                    && index.metadata.is_compound()
                    && index
                        .metadata
                        .fields
                        .iter()
                        .any(|field| get_all_nested_values(doc, field).is_empty())
                {
                    continue;
                }
                let keys = index.extract_keys(doc);
                let mut seen = HashSet::new();
                for index_key in keys {
                    if !seen.insert(index_key.clone()) {
                        continue;
                    }
                    let is_null = Self::is_key_all_null(&index_key);

                    // Mirror the insert logic: only delete what was inserted
                    let was_indexed = !is_null || (index.metadata.unique && !index.metadata.sparse);
                    if was_indexed {
                        index.delete(&index_key, doc_id)?;
                        // Mark dirty for checkpoint persistence (mirrors add_document_to_indexes:1281)
                        self.dirty_btree_indexes.insert(index_name.clone());
                    }
                }
            }
        }

        // Fuzzy indexes - remove by document ID
        let fuzzy_names: Vec<String> = self.fuzzy_indexes.keys().cloned().collect();

        for index_name in fuzzy_names {
            if let Some(excluded) = exclude_index {
                if index_name == excluded {
                    continue;
                }
            }

            if let Some(index) = self.fuzzy_indexes.get_mut(&index_name) {
                index.remove(doc_id);
                // Mark dirty for checkpoint persistence
                self.dirty_fuzzy_indexes.insert(index_name.clone());
            }
        }

        // Fulltext indexes - remove by document ID
        let fulltext_names: Vec<String> = self.fulltext_indexes.keys().cloned().collect();

        for index_name in fulltext_names {
            if let Some(excluded) = exclude_index {
                if index_name == excluded {
                    continue;
                }
            }

            if let Some(index) = self.fulltext_indexes.get_mut(&index_name) {
                // Log error but continue - document already deleted
                if let Err(e) = index.remove(doc_id) {
                    log_error!(
                        "Failed to remove from fulltext index '{}': {:?}",
                        index_name,
                        e
                    );
                } else {
                    // Mark dirty only if remove succeeded
                    self.dirty_fulltext_indexes.insert(index_name.clone());
                }
            }
        }

        // Vector indexes - remove by document ID
        let vector_names: Vec<String> = self.vector_indexes.keys().cloned().collect();

        for index_name in vector_names {
            if let Some(excluded) = exclude_index {
                if index_name == excluded {
                    continue;
                }
            }

            if let Some(index) = self.vector_indexes.get_mut(&index_name) {
                let id_str = match doc_id {
                    DocumentId::Int(i) => i.to_string(),
                    DocumentId::String(s) => s.clone(),
                    DocumentId::ObjectId(oid) => oid.clone(),
                };
                index.remove(&id_str);
                self.dirty_vector_indexes.insert(index_name.clone());
            }
        }

        Ok(())
    }

    /// Check if a document would violate unique constraints
    ///
    /// Checks all unique indexes to see if the document's values already exist.
    /// Properly handles compound unique indexes.
    /// MongoDB behavior: null is a value, so duplicate nulls are rejected.
    ///
    /// # Arguments
    /// * `doc` - The document as JSON Value
    /// * `exclude_doc_id` - Optional document ID to exclude (for updates)
    /// * `exclude_index` - Optional index name to skip (e.g., "_id" handled separately)
    pub fn check_unique_constraints(
        &self,
        doc: &serde_json::Value,
        exclude_doc_id: Option<&DocumentId>,
        exclude_index: Option<&str>,
    ) -> Result<()> {
        for (index_name, index) in &self.btree_indexes {
            if let Some(excluded) = exclude_index {
                if index_name == excluded {
                    continue;
                }
            }

            // Only check unique indexes
            if !index.metadata.unique {
                continue;
            }

            if index.metadata.is_compound()
                && Self::count_compound_array_fields(doc, &index.metadata.fields) > 1
            {
                return Err(IronBaseError::IndexError(format!(
                    "Compound index '{}' cannot index multiple array fields: {}",
                    index.metadata.name,
                    index.metadata.fields.join(", ")
                )));
            }

            let keys = index.extract_keys(doc);
            let mut seen = HashSet::new();
            for index_key in keys {
                if !seen.insert(index_key.clone()) {
                    continue;
                }

                if let Some(existing_id) = index.search(&index_key) {
                    if exclude_doc_id != Some(&existing_id) {
                        let fields_str = if index.metadata.is_compound() {
                            index.metadata.fields.join(", ")
                        } else {
                            index.metadata.field.clone()
                        };
                        return Err(IronBaseError::IndexError(format!(
                            "Duplicate key: {:?} in field(s) '{}' (unique index)",
                            index_key, fields_str
                        )));
                    }
                }
            }
        }

        Ok(())
    }

    /// Helper: Check if an IndexKey is all Null
    pub fn is_key_all_null(key: &IndexKey) -> bool {
        match key {
            IndexKey::Null => true,
            IndexKey::Compound(keys) => keys.iter().all(|k| matches!(k, IndexKey::Null)),
            _ => false,
        }
    }

    pub(crate) fn count_compound_array_fields(doc: &serde_json::Value, fields: &[String]) -> usize {
        fields
            .iter()
            .filter(|field| path_crosses_array(doc, field))
            .count()
    }
}

impl Default for IndexManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test that index names must be unique across ALL index types.
    /// Previously, fuzzy and fulltext creation did not check vector_indexes,
    /// and btree/legacy only checked their own map.
    #[test]
    fn test_cross_type_index_name_collision() {
        let mut mgr = IndexManager::new();

        // Create a vector index first
        let config = VectorIndexConfig::new(128).with_field("embedding");
        mgr.create_vector_index(
            "shared_name".to_string(),
            "embedding".to_string(),
            config,
            None,
        )
        .unwrap();

        // Fuzzy with same name should fail
        let err = mgr
            .create_fuzzy_index(
                "shared_name".to_string(),
                "name".to_string(),
                FuzzyAlgorithm::JaroWinkler,
                0.8,
            )
            .unwrap_err();
        assert!(err.to_string().contains("Index already exists"));

        // Fulltext with same name should fail
        let err = mgr
            .create_fulltext_index(
                "shared_name".to_string(),
                "content".to_string(),
                crate::fulltext::FtsLanguage::English,
                None,
                None,
            )
            .unwrap_err();
        assert!(err.to_string().contains("Index already exists"));

        // Btree with same name should fail
        let err = mgr
            .create_btree_index("shared_name".to_string(), "age".to_string(), false, false)
            .unwrap_err();
        assert!(err.to_string().contains("Index already exists"));

        // Btree CI with same name should fail
        let err = mgr
            .create_btree_index_ci("shared_name".to_string(), "email".to_string(), false)
            .unwrap_err();
        assert!(err.to_string().contains("Index already exists"));

        // Compound with same name should fail
        let err = mgr
            .create_compound_index(
                "shared_name".to_string(),
                vec!["a".to_string(), "b".to_string()],
                false,
                false,
            )
            .unwrap_err();
        assert!(err.to_string().contains("Index already exists"));

        // Legacy with same name should fail
        let err = mgr
            .create_index(IndexDefinition {
                name: "shared_name".to_string(),
                field: "x".to_string(),
                index_type: crate::index::legacy::IndexType::Regular,
                unique: false,
            })
            .unwrap_err();
        assert!(err.to_string().contains("Index already exists"));
    }

    /// Test the reverse: btree index blocks vector/fuzzy/fulltext creation
    #[test]
    fn test_btree_blocks_other_types() {
        let mut mgr = IndexManager::new();

        // Create btree first
        mgr.create_btree_index("idx1".to_string(), "field".to_string(), false, false)
            .unwrap();

        // Vector with same name should fail
        let config = VectorIndexConfig::new(64).with_field("vec");
        let err = mgr
            .create_vector_index("idx1".to_string(), "vec".to_string(), config, None)
            .unwrap_err();
        assert!(err.to_string().contains("Index already exists"));

        // Fuzzy with same name should fail
        let err = mgr
            .create_fuzzy_index(
                "idx1".to_string(),
                "name".to_string(),
                FuzzyAlgorithm::JaroWinkler,
                0.8,
            )
            .unwrap_err();
        assert!(err.to_string().contains("Index already exists"));

        // Fulltext with same name should fail
        let err = mgr
            .create_fulltext_index(
                "idx1".to_string(),
                "text".to_string(),
                crate::fulltext::FtsLanguage::English,
                None,
                None,
            )
            .unwrap_err();
        assert!(err.to_string().contains("Index already exists"));
    }
}
