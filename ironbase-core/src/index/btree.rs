//! B+ Tree Index Implementation
//!
//! This module provides the core B+ tree data structure used for database indexing.
//! B+ trees are self-balancing, ordered search structures optimized for disk-based
//! storage and range queries.
//!
//! # B+ Tree Properties
//!
//! - **All data in leaves**: Internal nodes only contain routing keys
//! - **Balanced**: All leaf nodes are at the same depth
//! - **Linked leaves**: Leaf nodes form a linked list for range scans
//! - **High fanout**: Each node holds up to 128 keys (16KB pages)
//!
//! # Node Structure
//!
//! ```text
//! Internal Node:                    Leaf Node:
//! ┌─────────────────────────┐       ┌─────────────────────────┐
//! │ keys: [K1, K2, K3, ...] │       │ keys: [K1, K2, K3, ...] │
//! │ children: [C0, C1, C2]  │       │ doc_ids: [D1, D2, D3]   │
//! │ children_offsets: [...]  │       │ next_leaf_offset: u64   │
//! └─────────────────────────┘       └─────────────────────────┘
//! ```
//!
//! # Key Types
//!
//! The [`IndexKey`] enum supports multiple data types:
//! - `Null` - Missing or null fields
//! - `Int(i64)` - Integer values
//! - `Float(f64)` - Floating point values
//! - `String(String)` - String values
//! - `Bool(bool)` - Boolean values
//! - `Compound(Vec<IndexKey>)` - Multi-field compound keys
//!
//! # File Format (.idx)
//!
//! ```text
//! ┌────────────────────────────────────────┐
//! │ Root Offset (8 bytes)                  │
//! ├────────────────────────────────────────┤
//! │ Node 1 (variable size, JSON)           │
//! │ [node_type: u8][data_len: u32][json]   │
//! ├────────────────────────────────────────┤
//! │ Node 2...                              │
//! └────────────────────────────────────────┘
//! ```
//!
//! # Split Algorithm
//!
//! When a node exceeds [`MAX_KEYS_PER_NODE`] (128 keys):
//!
//! 1. **Leaf split**: Divide keys at midpoint, promote middle key to parent
//! 2. **Internal split**: Similar, but child pointers are redistributed
//! 3. **Root split**: Create new root with single key pointing to two children
//!
//! # Unique Index Enforcement
//!
//! For unique indexes, `insert()` returns `DuplicateKey` error if:
//! - Key already exists AND
//! - Key is not `IndexKey::Null` (null values can be duplicated)
//!
//! This matches MongoDB's behavior where null/missing fields don't violate uniqueness.
//!
//! # Thread Safety
//!
//! BPlusTree is NOT thread-safe internally. Concurrency is managed by
//! `IndexManager` which is wrapped in `Arc<RwLock<>>`.

use crate::document::DocumentId;
use crate::error::{IronBaseError, Result};
use crate::log_warn;
use crate::value_utils::get_all_nested_values;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;

use super::key::IndexKey;

// ============================================================================
// Range Query Types - Unified API for all range operations
// ============================================================================

/// Scan order for range queries
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanOrder {
    /// Ascending order (smallest to largest)
    Asc,
    /// Descending order (largest to smallest)
    Desc,
}

/// Mode for range_query() - determines what operation to perform
#[derive(Debug, Clone)]
pub enum RangeQueryMode {
    /// Count entries in range without materializing them - O(1) memory
    Count,
    /// Scan entries with optional skip/limit and order - O(limit) memory
    Scan {
        skip: usize,
        limit: Option<usize>,
        order: ScanOrder,
    },
}

/// Result of a range_query() operation
#[derive(Debug)]
pub enum RangeQueryResult {
    /// Count result (from RangeQueryMode::Count)
    Count(usize),
    /// Document IDs (from RangeQueryMode::Scan)
    Docs(Vec<DocumentId>),
}

impl RangeQueryResult {
    /// Unwrap as count — caller must ensure this is a Count variant
    pub fn unwrap_count(self) -> usize {
        match self {
            RangeQueryResult::Count(c) => c,
            RangeQueryResult::Docs(_) => {
                unreachable!("RangeQueryResult::unwrap_count() called on Docs variant — this is a bug in the caller (used RangeQueryMode::Scan but called unwrap_count)")
            }
        }
    }

    /// Unwrap as docs — caller must ensure this is a Docs variant
    pub fn unwrap_docs(self) -> Vec<DocumentId> {
        match self {
            RangeQueryResult::Docs(d) => d,
            RangeQueryResult::Count(_) => {
                unreachable!("RangeQueryResult::unwrap_docs() called on Count variant — this is a bug in the caller (used RangeQueryMode::Count but called unwrap_docs)")
            }
        }
    }
}

// ============================================================================
// Node page constants (for file-based persistence)
// ============================================================================

// Re-export from central limits module
use crate::limits::MAX_KEYS_PER_NODE;
pub use crate::limits::NODE_PAGE_SIZE;

const NODE_TYPE_INTERNAL: u8 = 0;
const NODE_TYPE_LEAF: u8 = 1;

/// B+ Tree Node types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) enum BTreeNode {
    Internal(InternalNode),
    Leaf(LeafNode),
}

impl Default for BTreeNode {
    fn default() -> Self {
        BTreeNode::Leaf(LeafNode::default())
    }
}

/// Internal node (non-leaf) - contains routing keys and child pointers
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct InternalNode {
    pub keys: Vec<IndexKey>,
    /// In-memory children (used during tree operations)
    /// Note: Box is intentional for recursive B+ tree structure ownership semantics
    #[serde(skip)]
    #[allow(clippy::vec_box)]
    pub children: Vec<Box<BTreeNode>>,
    /// File offsets for persisted children (used for disk-based trees)
    pub children_offsets: Vec<u64>,
}

/// Leaf node - contains actual data pointers
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct LeafNode {
    pub keys: Vec<IndexKey>,
    pub document_ids: Vec<DocumentId>,
    #[serde(default)]
    pub next_leaf_offset: u64, // File offset to next leaf node (0 = none)
}

/// B+ Tree - main index structure
#[derive(Debug, Clone)]
pub struct BPlusTree {
    root: Box<BTreeNode>,
    pub metadata: IndexMetadata,
    /// Runtime statistics counters (not persisted)
    /// Tracks changes since last statistics refresh for staleness detection
    counters: StatisticsCounters,
    /// Source file path for lazy loading (None = fully in-memory)
    source_path: Option<PathBuf>,
    /// Lazy loading mode - true if tree data is not yet in memory
    lazy_mode: bool,
    /// Persisted file size in bytes (for threshold comparison)
    persisted_size: u64,
}

/// Index statistics for selectivity estimation
///
/// Used by query planner to select the best index when multiple indexes
/// could satisfy a query.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IndexStats {
    /// Estimated number of unique values (distinct count)
    /// Higher values mean lower selectivity
    #[serde(default)]
    pub distinct_count: u64,

    /// Number of NULL/missing values in the index
    #[serde(default)]
    pub null_count: u64,

    /// Ratio of documents with multikey arrays (0.0-1.0)
    /// Higher values indicate more multikey overhead
    #[serde(default)]
    pub multikey_ratio: f32,

    /// Unix timestamp of last statistics update
    #[serde(default)]
    pub last_analyzed: u64,

    /// Sampling rate used for statistics (0.0-1.0)
    /// 1.0 means full scan, 0.1 means 10% sample
    #[serde(default)]
    pub sample_rate: f32,

    /// Histogram for range query selectivity estimation
    /// Only populated for indexes with 100k+ entries
    #[serde(default)]
    pub histogram: Option<Histogram>,

    /// Most Common Values for equality selectivity estimation
    /// Only populated for indexes with 1000+ entries
    /// See `MostCommonValues` for PostgreSQL-style frequency tracking
    #[serde(default)]
    pub mcv: Option<MostCommonValues>,
}

impl IndexStats {
    /// Calculate selectivity estimate (fraction of docs that match a single value)
    ///
    /// Lower selectivity = better index for this query
    /// Returns 1.0 if no statistics available (worst case assumption)
    pub fn selectivity(&self, total_docs: u64) -> f64 {
        if self.distinct_count == 0 || total_docs == 0 {
            return 1.0; // No stats - assume full scan
        }
        // Selectivity = 1 / distinct_count
        // Estimated rows for equality = total_docs / distinct_count
        1.0 / self.distinct_count as f64
    }

    /// Estimate number of documents for an equality query
    pub fn estimate_rows(&self, total_docs: u64) -> u64 {
        if self.distinct_count == 0 {
            return total_docs;
        }
        (total_docs / self.distinct_count).max(1)
    }

    /// Validate and fix statistics against index size
    ///
    /// Called after deserializing to ensure consistency.
    /// Fixes any invalid values to safe defaults.
    ///
    /// # Arguments
    /// * `num_keys` - Total number of keys in the index
    ///
    /// # Returns
    /// `true` if any fixes were applied, `false` if stats were valid
    pub fn validate_and_fix(&mut self, num_keys: u64) -> bool {
        let mut fixed = false;

        // distinct_count cannot exceed num_keys
        if self.distinct_count > num_keys {
            self.distinct_count = num_keys;
            fixed = true;
        }

        // null_count cannot exceed num_keys
        if self.null_count > num_keys {
            self.null_count = num_keys;
            fixed = true;
        }

        // multikey_ratio must be in [0.0, 1.0]
        if self.multikey_ratio < 0.0 || self.multikey_ratio > 1.0 || self.multikey_ratio.is_nan() {
            self.multikey_ratio = 0.0;
            fixed = true;
        }

        // sample_rate must be in [0.0, 1.0]
        if self.sample_rate < 0.0 || self.sample_rate > 1.0 || self.sample_rate.is_nan() {
            self.sample_rate = 0.0;
            fixed = true;
        }

        fixed
    }
}

// ============================================================================
// Statistics Counters for Staleness Tracking
// ============================================================================

/// Runtime statistics counters for tracking index changes between full refreshes.
///
/// These counters are NOT persisted - they track changes since the last
/// statistics refresh to determine when statistics become stale.
///
/// # Usage
/// - Incremented on each insert/delete operation
/// - Checked periodically to determine if refresh is needed
/// - Reset after each statistics refresh
#[derive(Debug, Clone, Default)]
pub struct StatisticsCounters {
    /// Number of write operations (inserts) since last statistics refresh
    pub writes_since_refresh: u64,

    /// Number of delete operations since last statistics refresh
    pub deletes_since_refresh: u64,

    /// Total number of writes for overall tracking
    pub total_writes: u64,

    /// Snapshot of num_keys at the time of last statistics refresh
    /// Used to calculate staleness ratio
    pub num_keys_at_last_refresh: u64,
}

impl StatisticsCounters {
    /// Calculate staleness ratio (0.0 = fresh, 1.0 = completely stale)
    ///
    /// Staleness is measured as: (writes + deletes) / num_keys_at_last_refresh
    /// A ratio of 0.1 means 10% of the index has changed since last refresh.
    pub fn staleness_ratio(&self, _current_num_keys: u64) -> f64 {
        if self.num_keys_at_last_refresh == 0 {
            return 1.0; // Never analyzed = completely stale
        }

        let changes = self.writes_since_refresh + self.deletes_since_refresh;
        // Staleness = changes / original_size, clamped to [0.0, 1.0]
        (changes as f64 / self.num_keys_at_last_refresh as f64).min(1.0)
    }

    /// Check if statistics should be refreshed
    ///
    /// Returns true if:
    /// - More than 10% of the index has changed since last refresh, OR
    /// - More than 10,000 writes since last refresh (for small indexes)
    pub fn needs_refresh(&self, current_num_keys: u64) -> bool {
        let staleness = self.staleness_ratio(current_num_keys);
        staleness > 0.10 || self.writes_since_refresh > 10_000
    }

    /// Reset counters after a statistics refresh
    pub fn mark_refreshed(&mut self, current_num_keys: u64) {
        self.writes_since_refresh = 0;
        self.deletes_since_refresh = 0;
        self.num_keys_at_last_refresh = current_num_keys;
    }

    /// Increment write counter (called on insert)
    #[inline]
    pub fn record_write(&mut self) {
        self.writes_since_refresh += 1;
        self.total_writes += 1;
    }

    /// Increment delete counter (called on delete)
    #[inline]
    pub fn record_delete(&mut self) {
        self.deletes_since_refresh += 1;
    }
}

// ============================================================================
// Histogram for Range Query Selectivity Estimation
// ============================================================================

/// Default number of histogram buckets
fn default_bucket_count() -> u32 {
    64
}

/// Equi-depth histogram for range selectivity estimation
///
/// Each bucket contains approximately the same number of values.
/// Used by query planner to estimate selectivity for range queries
/// when uniform distribution assumption would be too inaccurate.
///
/// Only built for indexes with 100,000+ entries (below that, uniform is acceptable).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Histogram {
    /// Bucket boundaries (sorted) - contains bucket_count-1 values
    /// bucket[i] contains values where boundaries[i-1] <= v < boundaries[i]
    #[serde(default)]
    pub boundaries: Vec<IndexKey>,

    /// Minimum value in the index
    #[serde(default)]
    pub min_value: Option<IndexKey>,

    /// Maximum value in the index
    #[serde(default)]
    pub max_value: Option<IndexKey>,

    /// Number of buckets (default: 64)
    #[serde(default = "default_bucket_count")]
    pub bucket_count: u32,
}

impl Histogram {
    /// Minimum number of entries required to build a histogram
    pub const MIN_ENTRIES_FOR_HISTOGRAM: usize = 100_000;

    /// Estimate selectivity for a range query
    ///
    /// Returns the fraction of data that falls within the given range [start, end).
    /// If histogram is empty, falls back to uniform distribution assumption (0.33).
    ///
    /// # Arguments
    /// * `start` - Range start bound (None = unbounded)
    /// * `end` - Range end bound (None = unbounded)
    ///
    /// # Returns
    /// Selectivity estimate between 0.0 and 1.0
    pub fn estimate_range_selectivity(
        &self,
        start: Option<&IndexKey>,
        end: Option<&IndexKey>,
    ) -> f64 {
        if self.boundaries.is_empty() || self.bucket_count == 0 {
            return 0.33; // Fallback to uniform assumption
        }

        let start_bucket = match start {
            Some(v) => self.find_bucket(v),
            None => 0,
        };

        let end_bucket = match end {
            Some(v) => self.find_bucket(v),
            None => self.bucket_count as usize,
        };

        // Ensure at least 1 bucket is counted
        let covered = end_bucket.saturating_sub(start_bucket).max(1);
        covered as f64 / self.bucket_count as f64
    }

    /// Find which bucket a value falls into using binary search
    fn find_bucket(&self, value: &IndexKey) -> usize {
        self.boundaries.binary_search(value).unwrap_or_else(|i| i)
    }

    /// Check if histogram has valid data
    pub fn is_valid(&self) -> bool {
        !self.boundaries.is_empty() && self.bucket_count > 0
    }
}

// ============================================================================
// Most Common Values (MCV) for Equality Selectivity Estimation
// ============================================================================

/// Most Common Values (MCV) for equality query selectivity estimation.
///
/// Stores the top N most frequent values with their frequencies.
/// Used by the query planner to estimate selectivity for equality queries
/// on skewed distributions (e.g., status fields with 90% "active").
///
/// # PostgreSQL-style Design
///
/// Similar to PostgreSQL's `pg_statistic` MCV arrays, this enables the planner
/// to distinguish between common values (use frequency) and rare values
/// (use uniform distribution over remaining values).
///
/// # Memory Efficiency
///
/// Only stores DEFAULT_MCV_SIZE (20) values, bounded memory usage.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MostCommonValues {
    /// (value, frequency) pairs sorted by frequency descending
    #[serde(default)]
    pub values: Vec<(IndexKey, u64)>,

    /// Total number of keys when MCV was computed
    /// Used to calculate selectivity: freq / total
    #[serde(default)]
    pub total_keys: u64,

    /// Sum of frequencies of all MCV entries
    /// Remaining frequency = total_keys - mcv_frequency_sum
    #[serde(default)]
    pub mcv_frequency_sum: u64,
}

impl MostCommonValues {
    /// Default number of MCV entries to track
    pub const DEFAULT_MCV_SIZE: usize = 20;

    /// Minimum index size to build MCV (smaller indexes use uniform assumption)
    pub const MIN_ENTRIES_FOR_MCV: usize = 1_000;

    /// Maximum distinct values to track before switching to sampling
    /// Prevents OOM on high-cardinality fields
    pub const MAX_TRACKED_DISTINCT: usize = 100_000;

    /// Estimate selectivity for a specific value
    ///
    /// Returns:
    /// - MCV frequency/total if value is in MCV list
    /// - Uniform distribution estimate for values not in MCV
    ///
    /// # Algorithm
    ///
    /// For value V:
    /// 1. If V is in MCV: selectivity = freq(V) / total_keys
    /// 2. If V is not in MCV: selectivity = (total_keys - mcv_sum) / (distinct - mcv_count) / total_keys
    pub fn estimate_selectivity(&self, value: &IndexKey, distinct_count: u64) -> f64 {
        if self.total_keys == 0 {
            // No data - fallback to uniform based on distinct count
            return 1.0 / distinct_count.max(1) as f64;
        }

        // Check if value is in MCV
        if let Some((_, freq)) = self.values.iter().find(|(k, _)| k == value) {
            return *freq as f64 / self.total_keys as f64;
        }

        // Value not in MCV - estimate based on remaining frequency
        let remaining_keys = self.total_keys.saturating_sub(self.mcv_frequency_sum);
        let remaining_distinct = distinct_count.saturating_sub(self.values.len() as u64);

        if remaining_distinct == 0 || remaining_keys == 0 {
            // All distinct values are in MCV - value doesn't exist
            return 0.0;
        }

        // Assume uniform distribution among non-MCV values
        (remaining_keys as f64 / remaining_distinct as f64) / self.total_keys as f64
    }

    /// Check if MCV data is valid
    pub fn is_valid(&self) -> bool {
        !self.values.is_empty() && self.total_keys > 0
    }
}

/// Index metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexMetadata {
    pub name: String,
    /// Primary field for single-field indexes (backward compatibility)
    pub field: String,
    /// All fields for compound indexes (e.g., ["country", "city", "zipcode"])
    /// For single-field indexes, this will contain just one field matching `field`
    #[serde(default)]
    pub fields: Vec<String>,
    pub unique: bool,
    pub sparse: bool,
    #[serde(default)]
    pub multikey: bool,
    /// If true, string values are stored lowercased for case-insensitive matching
    #[serde(default)]
    pub case_insensitive: bool,
    pub num_keys: u64,
    pub tree_height: u32,
    #[serde(default)]
    pub root_offset: u64, // File offset to root node (0 = in-memory only)
    /// Statistics for query planning
    #[serde(default)]
    pub stats: IndexStats,
    /// True while index is being built - query planner should ignore this index
    /// to prevent using partially populated indexes (which would return incomplete results)
    #[serde(default)]
    pub building: bool,
}

impl IndexMetadata {
    /// Check if this is a compound index (multiple fields)
    pub fn is_compound(&self) -> bool {
        self.fields.len() > 1
    }
}

impl BPlusTree {
    /// Create new B+ tree index (single field)
    ///
    /// # Arguments
    /// * `name` - Index name
    /// * `field` - Field to index
    /// * `unique` - Whether values must be unique
    /// * `sparse` - If true, documents missing the field are not indexed
    pub fn new(name: String, field: String, unique: bool, sparse: bool) -> Self {
        // Start with empty leaf node as root
        let root = Box::new(BTreeNode::Leaf(LeafNode {
            keys: Vec::new(),
            document_ids: Vec::new(),
            next_leaf_offset: 0,
        }));

        BPlusTree {
            root,
            metadata: IndexMetadata {
                name,
                field: field.clone(),
                fields: vec![field], // Single-field index
                unique,
                sparse,
                multikey: false,
                case_insensitive: false,
                num_keys: 0,
                tree_height: 1,
                root_offset: 0,
                stats: IndexStats::default(),
                building: false,
            },
            counters: StatisticsCounters::default(),
            source_path: None,
            lazy_mode: false,
            persisted_size: 0,
        }
    }

    /// Create new case-insensitive B+ tree index (single field)
    ///
    /// String values are stored lowercased for case-insensitive matching.
    /// Non-string values are stored as-is.
    ///
    /// # Arguments
    /// * `name` - Index name (should end with "_ci" by convention)
    /// * `field` - Field to index
    /// * `unique` - Whether values must be unique (case-insensitively)
    pub fn new_ci(name: String, field: String, unique: bool) -> Self {
        let root = Box::new(BTreeNode::Leaf(LeafNode {
            keys: Vec::new(),
            document_ids: Vec::new(),
            next_leaf_offset: 0,
        }));

        BPlusTree {
            root,
            metadata: IndexMetadata {
                name,
                field: field.clone(),
                fields: vec![field],
                unique,
                sparse: true,
                multikey: false,
                case_insensitive: true,
                num_keys: 0,
                tree_height: 1,
                root_offset: 0,
                stats: IndexStats::default(),
                building: false,
            },
            counters: StatisticsCounters::default(),
            source_path: None,
            lazy_mode: false,
            persisted_size: 0,
        }
    }

    /// Create new compound B+ tree index (multiple fields)
    ///
    /// # Arguments
    /// * `name` - Index name
    /// * `fields` - List of fields in order (e.g., ["country", "city"])
    /// * `unique` - Whether the compound key must be unique
    /// * `sparse` - If true, documents missing any field are not indexed
    ///
    /// # Example
    /// ```rust,ignore
    /// let index = BPlusTree::new_compound(
    ///     "users_location".to_string(),
    ///     vec!["country".to_string(), "city".to_string()],
    ///     false,
    ///     false
    /// );
    /// ```
    pub fn new_compound(name: String, fields: Vec<String>, unique: bool, sparse: bool) -> Self {
        assert!(
            !fields.is_empty(),
            "Compound index must have at least one field"
        );

        let root = Box::new(BTreeNode::Leaf(LeafNode {
            keys: Vec::new(),
            document_ids: Vec::new(),
            next_leaf_offset: 0,
        }));

        let primary_field = fields[0].clone();

        BPlusTree {
            root,
            metadata: IndexMetadata {
                name,
                field: primary_field, // First field for backward compatibility
                fields,               // All fields for compound key
                unique,
                sparse,
                multikey: false,
                case_insensitive: false,
                num_keys: 0,
                tree_height: 1,
                root_offset: 0,
                stats: IndexStats::default(),
                building: false,
            },
            counters: StatisticsCounters::default(),
            source_path: None,
            lazy_mode: false,
            persisted_size: 0,
        }
    }

    /// Check if this index is currently being built
    pub fn is_building(&self) -> bool {
        self.metadata.building
    }

    /// Set the building flag (for index creation lifecycle)
    pub fn set_building(&mut self, building: bool) {
        self.metadata.building = building;
    }

    /// Get read-only access to statistics counters
    pub fn counters(&self) -> &StatisticsCounters {
        &self.counters
    }

    /// Check if statistics need to be refreshed based on staleness
    pub fn needs_stats_refresh(&self) -> bool {
        self.counters.needs_refresh(self.metadata.num_keys)
    }

    /// Get staleness ratio (0.0 = fresh, 1.0 = completely stale)
    pub fn staleness_ratio(&self) -> f64 {
        self.counters.staleness_ratio(self.metadata.num_keys)
    }

    /// Extract compound key from a document
    ///
    /// For compound indexes, creates an IndexKey::Compound from multiple fields
    /// For single-field indexes, returns a simple IndexKey
    pub fn extract_key(&self, doc: &serde_json::Value) -> IndexKey {
        self.extract_keys(doc)
            .into_iter()
            .next()
            .unwrap_or(IndexKey::Null)
    }

    /// Extract all keys for a document (supports multi-key indexes on arrays)
    ///
    /// For case-insensitive indexes, string values are lowercased.
    pub fn extract_keys(&self, doc: &serde_json::Value) -> Vec<IndexKey> {
        let ci = self.metadata.case_insensitive;

        if self.metadata.is_compound() {
            let field_values: Vec<Vec<IndexKey>> = self
                .metadata
                .fields
                .iter()
                .map(|field| {
                    let values = get_all_nested_values(doc, field);
                    if values.is_empty() {
                        vec![IndexKey::Null]
                    } else {
                        values
                            .into_iter()
                            .map(|v| Self::value_to_key(v, ci))
                            .collect()
                    }
                })
                .collect();

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

            combinations.into_iter().map(IndexKey::Compound).collect()
        } else {
            let values = crate::value_utils::get_all_nested_values(doc, &self.metadata.field);
            if values.is_empty() {
                vec![IndexKey::Null]
            } else {
                values
                    .into_iter()
                    .map(|v| Self::value_to_key(v, ci))
                    .collect()
            }
        }
    }

    /// Convert JSON value to IndexKey, optionally lowercasing strings for CI indexes
    fn value_to_key(value: &serde_json::Value, case_insensitive: bool) -> IndexKey {
        if case_insensitive {
            if let serde_json::Value::String(s) = value {
                return IndexKey::String(s.to_lowercase());
            }
        }
        IndexKey::from(value.clone())
    }

    /// Search for a key in the index
    pub fn search(&self, key: &IndexKey) -> Option<DocumentId> {
        self.search_in_node(&self.root, key)
    }

    fn search_in_node(&self, node: &BTreeNode, key: &IndexKey) -> Option<DocumentId> {
        match node {
            BTreeNode::Internal(internal) => {
                // Find which child to descend into
                let child_index = self.find_child_index(&internal.keys, key);

                // Use in-memory children if available
                if child_index < internal.children.len() {
                    self.search_in_node(&internal.children[child_index], key)
                } else if child_index < internal.children_offsets.len() {
                    // Lazy loading: load child from disk on-demand
                    if let Some(ref path) = self.source_path {
                        let offset = internal.children_offsets[child_index];
                        match File::open(path) {
                            Ok(mut file) => match Self::load_node(&mut file, offset) {
                                Ok(child_node) => self.search_in_node(&child_node, key),
                                Err(e) => {
                                    log_warn!(
                                        "[LAZY_LOAD_ERROR] Failed to load node at offset {}: {:?}",
                                        offset,
                                        e
                                    );
                                    None
                                }
                            },
                            Err(e) => {
                                log_warn!(
                                    "[LAZY_LOAD_ERROR] Failed to open file {:?}: {:?}",
                                    path,
                                    e
                                );
                                None
                            }
                        }
                    } else {
                        log_warn!(
                            "[LAZY_LOAD_ERROR] No source_path for lazy loading! children={}, offsets={}",
                            internal.children.len(),
                            internal.children_offsets.len()
                        );
                        None
                    }
                } else {
                    log_warn!(
                        "[LAZY_LOAD_ERROR] child_index {} out of bounds: children={}, offsets={}",
                        child_index,
                        internal.children.len(),
                        internal.children_offsets.len()
                    );
                    None
                }
            }
            BTreeNode::Leaf(leaf) => {
                // Binary search in leaf
                match leaf.keys.binary_search(key) {
                    Ok(index) => Some(leaf.document_ids[index].clone()),
                    Err(_) => None,
                }
            }
        }
    }

    /// Insert key-value pair into index
    /// Handles automatic node splitting when nodes become too large
    pub fn insert(&mut self, key: IndexKey, doc_id: DocumentId) -> Result<()> {
        // Ensure tree is fully loaded for traversal
        self.ensure_fully_loaded()?;

        // Check unique constraint
        if self.metadata.unique && self.search(&key).is_some() {
            return Err(IronBaseError::IndexError(format!(
                "Duplicate key: {:?} (unique index)",
                key
            )));
        }

        self.insert_unchecked(key, doc_id)
    }

    /// Insert without unique constraint check (used by build_from_sorted after pre-check)
    fn insert_unchecked(&mut self, key: IndexKey, doc_id: DocumentId) -> Result<()> {
        // Perform recursive insert directly on root (mutates in place)
        let split_result = Self::insert_into_node(&mut self.root, key, doc_id)?;

        // Handle split at root level - need to promote to new root
        if let Some((separator, right_child)) = split_result {
            // Take current root and wrap it as left child of new root
            let old_root = std::mem::take(&mut self.root);

            // Create new root with the split result (reuse Box allocation)
            *self.root = BTreeNode::Internal(InternalNode {
                keys: vec![separator],
                children: vec![old_root, right_child],
                children_offsets: Vec::new(),
            });
            self.metadata.tree_height += 1;
        }

        self.metadata.num_keys += 1;
        // Track write for staleness detection
        self.counters.record_write();
        Ok(())
    }

    /// Estimate serialized size of a node (approximate, for early split detection)
    ///
    /// JSON serialization overhead is significant:
    /// - LeafNode: `{"keys":[...],"document_ids":[...],"next_leaf_offset":0}`
    /// - Each key: `{"String":"value"},` or `{"Int":123},`
    /// - Each doc_id: `{"String":"ObjectId24chars"},` (~40 bytes)
    ///
    /// Bug fix (2026-01): Previous estimates were too low, causing "Node size exceeds
    /// page size" errors when many documents share the same long key (e.g., chunks
    /// from the same source file).
    fn estimate_node_size(node: &BTreeNode) -> usize {
        match node {
            BTreeNode::Leaf(leaf) => {
                let key_sizes: usize = leaf.keys.iter().map(Self::estimate_key_size).sum();
                // doc_id JSON: {"String":"..."},  or {"Int":...}, - typically ~40 bytes
                let doc_id_overhead = leaf.document_ids.len() * 40;
                // Base overhead: {"keys":[],"document_ids":[],"next_leaf_offset":0}
                80 + key_sizes + doc_id_overhead
            }
            BTreeNode::Internal(internal) => {
                let key_sizes: usize = internal.keys.iter().map(Self::estimate_key_size).sum();
                // children are skipped in serialization, but offsets are stored
                80 + key_sizes + internal.children_offsets.len() * 12
            }
        }
    }

    /// Estimate serialized size of a key in JSON format
    ///
    /// JSON enum serialization adds wrapper: `{"Variant":"value"}`
    /// - Null: `{"Null":null}` = 13
    /// - Bool: `{"Bool":true}` = 13
    /// - Int: `{"Int":1234567890}` = ~20
    /// - Float: `{"Float":1.234567890}` = ~25
    /// - String: `{"String":"..."}` = len + 14 + comma
    fn estimate_key_size(key: &IndexKey) -> usize {
        match key {
            IndexKey::Null => 15,
            IndexKey::Bool(_) => 15,
            IndexKey::Int(_) => 25,
            IndexKey::Float(_) => 30,
            IndexKey::String(s) => s.len() + 20,
            IndexKey::Compound(keys) => {
                // {"Compound":[...]}
                keys.iter().map(Self::estimate_key_size).sum::<usize>() + 15
            }
            IndexKey::MaxKey => 15,
        }
    }

    /// Recursively insert into a node (mutates in place)
    /// Returns an optional split result (separator key, new right sibling) if node was split
    fn insert_into_node(
        node: &mut Box<BTreeNode>,
        key: IndexKey,
        doc_id: DocumentId,
    ) -> Result<Option<(IndexKey, Box<BTreeNode>)>> {
        match &mut **node {
            BTreeNode::Leaf(leaf) => {
                // Insert into leaf
                let insert_pos = leaf.keys.binary_search(&key).unwrap_or_else(|pos| pos);
                leaf.keys.insert(insert_pos, key);
                leaf.document_ids.insert(insert_pos, doc_id);

                // Check if split is needed (by count OR by size for long keys)
                let needs_split = leaf.keys.len() > MAX_KEYS_PER_NODE
                    || Self::estimate_node_size(&BTreeNode::Leaf(leaf.clone()))
                        > NODE_PAGE_SIZE - 200;

                if needs_split {
                    Ok(Some(Self::split_leaf(leaf)))
                } else {
                    Ok(None)
                }
            }
            BTreeNode::Internal(internal) => {
                // Find which child to descend into
                // B+ tree convention: left child has keys < separator, right child has keys >= separator
                let child_idx = match internal.keys.binary_search(&key) {
                    Ok(pos) => pos + 1, // Key equals separator: go to right child
                    Err(pos) => pos,    // Key less than separator: go to left child
                };

                // Get the child (must exist for internal nodes)
                if child_idx >= internal.children.len() {
                    return Err(IronBaseError::IndexError(
                        "Internal node has no children".to_string(),
                    ));
                }

                // Recursive insert (mutates child in place)
                let child_split =
                    Self::insert_into_node(&mut internal.children[child_idx], key, doc_id)?;

                // Handle child split
                if let Some((separator, right_child)) = child_split {
                    // Insert separator and new child into this internal node
                    internal.keys.insert(child_idx, separator);
                    internal.children.insert(child_idx + 1, right_child);

                    // Check if this internal node needs to split (by count OR by size)
                    let needs_split = internal.keys.len() > MAX_KEYS_PER_NODE
                        || Self::estimate_node_size(&BTreeNode::Internal(internal.clone()))
                            > NODE_PAGE_SIZE - 200;

                    if needs_split {
                        Ok(Some(Self::split_internal(internal)))
                    } else {
                        Ok(None)
                    }
                } else {
                    Ok(None)
                }
            }
        }
    }

    /// Split a leaf node, returning (separator key, new right leaf)
    fn split_leaf(leaf: &mut LeafNode) -> (IndexKey, Box<BTreeNode>) {
        let mid = leaf.keys.len() / 2;

        // Split keys and document_ids at midpoint
        let right_keys = leaf.keys.split_off(mid);
        let right_doc_ids = leaf.document_ids.split_off(mid);

        // Separator is the first key of the right leaf
        let separator = right_keys[0].clone();

        // Create new right leaf
        let right_leaf = BTreeNode::Leaf(LeafNode {
            keys: right_keys,
            document_ids: right_doc_ids,
            next_leaf_offset: leaf.next_leaf_offset, // Right leaf inherits next pointer
        });

        // Update left leaf's next pointer (would point to right leaf in file-based impl)
        // For in-memory, we don't need this, but keep consistent
        leaf.next_leaf_offset = 0;

        (separator, Box::new(right_leaf))
    }

    /// Split an internal node, returning (separator key, new right internal)
    fn split_internal(internal: &mut InternalNode) -> (IndexKey, Box<BTreeNode>) {
        let mid = internal.keys.len() / 2;

        // The middle key becomes the separator (promoted to parent)
        let separator = internal.keys.remove(mid);

        // Split keys and children
        let right_keys = internal.keys.split_off(mid);
        let right_children = internal.children.split_off(mid + 1);
        let right_offsets = if internal.children_offsets.len() > mid + 1 {
            internal.children_offsets.split_off(mid + 1)
        } else {
            Vec::new()
        };

        // Create new right internal node
        let right_internal = BTreeNode::Internal(InternalNode {
            keys: right_keys,
            children: right_children,
            children_offsets: right_offsets,
        });

        (separator, Box::new(right_internal))
    }

    /// Build index from pre-sorted entries with automatic splitting
    ///
    /// Uses the insert() method for each entry, which handles automatic node
    /// splitting. This is O(n log n) but guarantees correct B+ tree structure.
    ///
    /// # Arguments
    /// * `entries` - MUST be sorted by key in ascending order
    /// * `check_unique` - If true, checks for duplicate keys and returns error
    ///
    /// # Returns
    /// * `Ok(())` on success
    /// * `Err(IndexError)` if unique constraint violated and check_unique is true
    pub fn build_from_sorted(
        &mut self,
        entries: Vec<(IndexKey, DocumentId)>,
        check_unique: bool,
    ) -> Result<()> {
        // Collect stats while building: count distinct keys and nulls
        let mut distinct_count: u64 = 0;
        let mut null_count: u64 = 0;
        let mut last_key: Option<&IndexKey> = None;

        // Check unique constraint if required - O(n) scan for adjacent duplicates
        // Also collect distinct count in the same pass
        if !entries.is_empty() {
            for i in 0..entries.len() {
                let current_key = &entries[i].0;

                // Count distinct keys (since sorted, just check if different from prev)
                if last_key != Some(current_key) {
                    distinct_count += 1;
                    last_key = Some(current_key);
                }

                // Count nulls
                if matches!(current_key, IndexKey::Null) {
                    null_count += 1;
                }

                // Check unique constraint
                if check_unique && i > 0 && entries[i - 1].0 == entries[i].0 {
                    return Err(IronBaseError::IndexError(format!(
                        "Duplicate key: {:?} (unique index)",
                        entries[i].0
                    )));
                }
            }
        }

        // Insert each entry using insert_unchecked (skips per-entry unique check)
        // Unique constraint was already validated above in O(n) for sorted data
        for (key, doc_id) in entries {
            self.insert_unchecked(key, doc_id)?;
        }

        // Update statistics
        self.metadata.stats.distinct_count = distinct_count;
        self.metadata.stats.null_count = null_count;
        self.metadata.stats.last_analyzed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.metadata.stats.sample_rate = 1.0; // Full scan during build

        Ok(())
    }

    /// Clear all entries from the index, resetting to empty state
    ///
    /// Resets the B+ tree to a single empty leaf node.
    /// Used before rebuild operations to prevent duplicate entries.
    pub fn clear(&mut self) {
        // Reset to empty leaf node (reuse existing Box allocation)
        *self.root = BTreeNode::Leaf(LeafNode {
            keys: Vec::new(),
            document_ids: Vec::new(),
            next_leaf_offset: 0,
        });
        // Reset metadata counts
        self.metadata.num_keys = 0;
        self.metadata.tree_height = 1;
    }

    /// Delete key-document pair from index
    /// Supports multi-level B+ trees by recursively finding the leaf
    pub fn delete(&mut self, key: &IndexKey, doc_id: &DocumentId) -> Result<()> {
        // Ensure tree is fully loaded for traversal
        self.ensure_fully_loaded()?;

        let deleted = Self::delete_from_node(&mut self.root, key, doc_id);
        if deleted {
            self.metadata.num_keys -= 1;
            // Track delete for staleness detection
            self.counters.record_delete();
        }
        Ok(())
    }

    /// Recursively delete from a node, returns true if deletion occurred
    ///
    /// BUG FIX: For non-unique indexes, multiple documents can have the same key.
    /// binary_search only returns ONE position, so we must scan ALL entries with
    /// the same key to find the correct doc_id.
    fn delete_from_node(node: &mut Box<BTreeNode>, key: &IndexKey, doc_id: &DocumentId) -> bool {
        match **node {
            BTreeNode::Leaf(ref mut leaf) => {
                // For non-unique indexes, multiple entries can have the same key.
                // binary_search returns ANY matching position, not necessarily the one
                // with our doc_id. We must scan all entries with this key.

                // Find the FIRST position where key could be (lower bound)
                let start_pos = leaf.keys.partition_point(|k| k < key);

                // Scan all entries with this key to find our doc_id
                for pos in start_pos..leaf.keys.len() {
                    if &leaf.keys[pos] != key {
                        // Past the matching keys, not found
                        break;
                    }
                    if &leaf.document_ids[pos] == doc_id {
                        leaf.keys.remove(pos);
                        leaf.document_ids.remove(pos);
                        return true;
                    }
                }
                false
            }
            BTreeNode::Internal(ref mut internal) => {
                // Find which child might contain the key
                // B+ tree convention: left child has keys < separator, right child has keys >= separator
                let child_idx = match internal.keys.binary_search(key) {
                    Ok(pos) => pos + 1, // Key equals separator: go to right child
                    Err(pos) => pos,    // Key less than separator: go to left child
                };

                if child_idx < internal.children.len() {
                    Self::delete_from_node(&mut internal.children[child_idx], key, doc_id)
                } else {
                    false
                }
            }
        }
    }

    /// Get all entries from the index as a Vec
    /// This allows O(n) extraction for batch rebuild operations
    ///
    /// NOTE: This method now supports multi-level B+ trees through recursive traversal.
    /// For Internal nodes, it recursively collects entries from all children.
    ///
    /// LAZY MODE SUPPORT: If the index is in lazy mode (not fully loaded from disk),
    /// this method will read entries directly from the file without modifying the tree.
    /// This ensures correct results even when the index hasn't been fully loaded.
    ///
    /// OOM Protection: Pre-allocates based on known num_keys.
    pub fn get_all_entries(&self) -> Vec<(IndexKey, DocumentId)> {
        // LAZY MODE: If index is in lazy mode, read directly from file
        // This fixes the bug where lazy indexes returned empty results
        if self.lazy_mode {
            if let Some(ref path) = self.source_path {
                match File::open(path) {
                    Ok(mut file) => {
                        match self.get_all_entries_with_file(&mut file) {
                            Ok(entries) => {
                                tracing::debug!(
                                    index = %self.metadata.name,
                                    num_entries = entries.len(),
                                    "get_all_entries: read from lazy index file"
                                );
                                return entries;
                            }
                            Err(e) => {
                                tracing::warn!(
                                    index = %self.metadata.name,
                                    error = %e,
                                    "get_all_entries: failed to read lazy index file, falling back to in-memory"
                                );
                                // Fall through to in-memory path
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            index = %self.metadata.name,
                            path = %path.display(),
                            error = %e,
                            "get_all_entries: failed to open lazy index file"
                        );
                        // Fall through to in-memory path (will likely return empty)
                    }
                }
            }
        }

        // IN-MEMORY PATH: Index is fully loaded
        let mut results = Vec::new();
        // OOM protection: try to pre-allocate based on known entry count
        let estimated = self.metadata.num_keys as usize;
        if results.try_reserve(estimated).is_err() {
            // If we can't allocate, return empty vec (caller should handle)
            tracing::warn!(
                "get_all_entries: failed to reserve {} entries for index '{}'",
                estimated,
                self.metadata.name
            );
            return results;
        }
        self.collect_entries_recursive(&self.root, &mut results);
        results
    }

    /// Recursively collect all entries from a B+ tree node
    /// Traverses Internal nodes and collects from all Leaf nodes
    fn collect_entries_recursive(
        &self,
        node: &BTreeNode,
        results: &mut Vec<(IndexKey, DocumentId)>,
    ) {
        match node {
            BTreeNode::Leaf(leaf) => {
                // Collect all entries from this leaf
                for (key, doc_id) in leaf.keys.iter().zip(leaf.document_ids.iter()) {
                    results.push((key.clone(), doc_id.clone()));
                }
            }
            BTreeNode::Internal(internal) => {
                // Use in-memory children if available
                if !internal.children.is_empty() {
                    for child in &internal.children {
                        self.collect_entries_recursive(child, results);
                    }
                }
                // If no in-memory children, would need file handle (use get_all_entries_with_file)
            }
        }
    }

    /// Get all entries with file handle support for multi-level persistent trees
    ///
    /// This method can traverse Internal nodes by loading children from disk.
    ///
    /// OOM Protection: Pre-allocates based on known num_keys.
    pub fn get_all_entries_with_file(
        &self,
        file: &mut File,
    ) -> Result<Vec<(IndexKey, DocumentId)>> {
        let mut results = Vec::new();
        // OOM protection: try to pre-allocate based on known entry count
        let estimated = self.metadata.num_keys as usize;
        results.try_reserve(estimated).map_err(|_| {
            IronBaseError::OutOfMemory(format!(
                "Cannot allocate {} entries for index '{}'",
                estimated, self.metadata.name
            ))
        })?;
        self.collect_entries_recursive_with_file(&self.root, file, &mut results)?;
        Ok(results)
    }

    /// Recursively collect entries with file handle for disk-based child loading
    fn collect_entries_recursive_with_file(
        &self,
        node: &BTreeNode,
        file: &mut File,
        results: &mut Vec<(IndexKey, DocumentId)>,
    ) -> Result<()> {
        match node {
            BTreeNode::Leaf(leaf) => {
                for (key, doc_id) in leaf.keys.iter().zip(leaf.document_ids.iter()) {
                    results.push((key.clone(), doc_id.clone()));
                }
                Ok(())
            }
            BTreeNode::Internal(internal) => {
                // Traverse all children by loading them from disk
                for &child_offset in &internal.children_offsets {
                    if child_offset > 0 {
                        let child_node = Self::load_node(file, child_offset)?;
                        self.collect_entries_recursive_with_file(&child_node, file, results)?;
                    }
                }
                Ok(())
            }
        }
    }

    /// Streaming traversal of all entries without materializing them into a Vec.
    ///
    /// O(1) memory — processes entries leaf-by-leaf via callback.
    /// Supports lazy mode: if the index is lazy, opens the file and loads
    /// child nodes on-demand (same pattern as `walk_leaves_desc`).
    ///
    /// # Arguments
    /// * `callback` - Called for each (key, doc_id) pair in ascending key order.
    ///
    /// # Example
    /// ```ignore
    /// let mut count = 0u64;
    /// btree.for_each_entry(&mut |_key, _doc_id| {
    ///     count += 1;
    /// });
    /// ```
    pub fn for_each_entry<F: FnMut(&IndexKey, &DocumentId)>(&self, callback: &mut F) {
        // LAZY MODE: open file for on-demand child loading
        if self.lazy_mode {
            if let Some(ref path) = self.source_path {
                match File::open(path) {
                    Ok(mut file) => {
                        Self::for_each_entry_with_file(&self.root, callback, &mut file);
                        return;
                    }
                    Err(e) => {
                        tracing::warn!(
                            index = %self.metadata.name,
                            path = %path.display(),
                            error = %e,
                            "for_each_entry: failed to open lazy index file, falling back to in-memory"
                        );
                        // Fall through to in-memory path
                    }
                }
            }
        }

        // IN-MEMORY PATH: walk leaves without any allocation
        Self::for_each_entry_in_memory(&self.root, callback);
    }

    /// In-memory ascending leaf walk — calls callback for each (key, doc_id).
    fn for_each_entry_in_memory<F: FnMut(&IndexKey, &DocumentId)>(
        node: &BTreeNode,
        callback: &mut F,
    ) {
        match node {
            BTreeNode::Leaf(leaf) => {
                for (key, doc_id) in leaf.keys.iter().zip(leaf.document_ids.iter()) {
                    callback(key, doc_id);
                }
            }
            BTreeNode::Internal(internal) => {
                for child in &internal.children {
                    Self::for_each_entry_in_memory(child, callback);
                }
            }
        }
    }

    /// Lazy-file ascending leaf walk — loads child nodes from disk on-demand.
    fn for_each_entry_with_file<F: FnMut(&IndexKey, &DocumentId)>(
        node: &BTreeNode,
        callback: &mut F,
        file: &mut File,
    ) {
        match node {
            BTreeNode::Leaf(leaf) => {
                for (key, doc_id) in leaf.keys.iter().zip(leaf.document_ids.iter()) {
                    callback(key, doc_id);
                }
            }
            BTreeNode::Internal(internal) => {
                if !internal.children.is_empty() {
                    // In-memory children available
                    for child in &internal.children {
                        Self::for_each_entry_with_file(child, callback, file);
                    }
                } else {
                    // Lazy load children from file (LEFT to RIGHT for ascending order)
                    for &child_offset in &internal.children_offsets {
                        if child_offset > 0 {
                            match Self::load_node(file, child_offset) {
                                Ok(child_node) => {
                                    Self::for_each_entry_with_file(&child_node, callback, file);
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        offset = child_offset,
                                        error = %e,
                                        "for_each_entry_with_file: failed to load lazy child node"
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Apply batch updates without full rebuild.
    ///
    /// Performs a two-phase update:
    /// 1. Delete all old entries.
    /// 2. Insert all new entries.
    ///
    /// This avoids rebuilding the entire tree and keeps memory usage low.
    ///
    /// # Arguments
    /// * `updates` - Vec of (old_key, old_doc_id, new_key, new_doc_id) tuples
    pub fn apply_batch_updates(
        &mut self,
        updates: Vec<(IndexKey, DocumentId, IndexKey, DocumentId)>,
    ) -> Result<()> {
        if updates.is_empty() {
            return Ok(());
        }

        let mut deletes: Vec<(IndexKey, DocumentId)> = Vec::with_capacity(updates.len());
        let mut inserts: Vec<(IndexKey, DocumentId)> = Vec::with_capacity(updates.len());

        for (old_key, old_doc_id, new_key, new_doc_id) in updates {
            if old_key == new_key && old_doc_id == new_doc_id {
                continue;
            }
            deletes.push((old_key, old_doc_id));
            inserts.push((new_key, new_doc_id));
        }

        deletes.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        inserts.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));

        for (key, doc_id) in deletes {
            self.delete(&key, &doc_id)?;
        }
        for (key, doc_id) in inserts {
            self.insert(key, doc_id)?;
        }

        Ok(())
    }

    /// Find child index for key in internal node
    /// B+ tree convention: left child has keys < separator, right child has keys >= separator
    fn find_child_index(&self, keys: &[IndexKey], key: &IndexKey) -> usize {
        match keys.binary_search(key) {
            Ok(pos) => pos + 1, // Key equals separator: go to right child (keys >= separator)
            Err(pos) => pos,    // Key less than separator: go to left child (keys < separator)
        }
    }

    // ========================================================================
    // Unified Range Query API
    // ========================================================================

    /// Unified range query - single entry point for all range operations
    ///
    /// This is the recommended API for all range-based index operations.
    /// It provides O(1) memory for counts and O(limit) memory for scans.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// // Count entries in range (O(1) memory)
    /// let count = tree.range_query(start, end, true, true, RangeQueryMode::Count);
    ///
    /// // Scan with limit, ascending (O(limit) memory)
    /// let docs = tree.range_query(start, end, true, true,
    ///     RangeQueryMode::Scan { skip: 0, limit: Some(10), order: ScanOrder::Asc });
    ///
    /// // Scan with limit, descending (O(limit) memory)
    /// let docs = tree.range_query(start, end, true, true,
    ///     RangeQueryMode::Scan { skip: 0, limit: Some(10), order: ScanOrder::Desc });
    /// ```
    pub fn range_query(
        &self,
        start: &IndexKey,
        end: &IndexKey,
        inclusive_start: bool,
        inclusive_end: bool,
        mode: RangeQueryMode,
    ) -> RangeQueryResult {
        // LAZY MODE: open file for on-demand child loading (same pattern as search_in_node)
        let mut lazy_file = if self.lazy_mode {
            self.source_path.as_ref().and_then(|path| {
                File::open(path)
                    .map_err(|e| {
                        tracing::warn!(
                            index = %self.metadata.name,
                            path = %path.display(),
                            error = %e,
                            "range_query: failed to open lazy index file"
                        );
                        e
                    })
                    .ok()
            })
        } else {
            None
        };

        match mode {
            RangeQueryMode::Count => {
                let count = self.count_range_internal(
                    start,
                    end,
                    inclusive_start,
                    inclusive_end,
                    &mut lazy_file,
                );
                RangeQueryResult::Count(count)
            }
            RangeQueryMode::Scan { skip, limit, order } => {
                let docs = match order {
                    ScanOrder::Asc => self.scan_asc_internal(
                        start,
                        end,
                        inclusive_start,
                        inclusive_end,
                        skip,
                        limit,
                        &mut lazy_file,
                    ),
                    ScanOrder::Desc => self.scan_desc_internal(
                        start,
                        end,
                        inclusive_start,
                        inclusive_end,
                        skip,
                        limit,
                        &mut lazy_file,
                    ),
                };
                RangeQueryResult::Docs(docs)
            }
        }
    }

    /// Internal: Count entries in range without materializing.
    /// Supports lazy mode via optional file handle for on-demand child loading.
    fn count_range_internal(
        &self,
        start: &IndexKey,
        end: &IndexKey,
        inclusive_start: bool,
        inclusive_end: bool,
        lazy_file: &mut Option<File>,
    ) -> usize {
        fn count_range(
            node: &BTreeNode,
            start: &IndexKey,
            end: &IndexKey,
            inclusive_start: bool,
            inclusive_end: bool,
            lazy_file: &mut Option<File>,
        ) -> usize {
            match node {
                BTreeNode::Leaf(leaf) => {
                    let start_idx = if inclusive_start {
                        leaf.keys.partition_point(|k| k < start)
                    } else {
                        leaf.keys.partition_point(|k| k <= start)
                    };
                    let end_idx = if inclusive_end {
                        leaf.keys.partition_point(|k| k <= end)
                    } else {
                        leaf.keys.partition_point(|k| k < end)
                    };
                    end_idx.saturating_sub(start_idx)
                }
                BTreeNode::Internal(internal) => {
                    let mut count = 0;
                    let child_count = if !internal.children.is_empty() {
                        internal.children.len()
                    } else {
                        internal.children_offsets.len()
                    };
                    for i in 0..child_count {
                        let child_min_might_match = i == 0 || &internal.keys[i - 1] <= end;
                        let child_max_might_match =
                            i >= internal.keys.len() || &internal.keys[i] >= start;
                        if child_min_might_match && child_max_might_match {
                            if i < internal.children.len() {
                                count += count_range(
                                    &internal.children[i],
                                    start,
                                    end,
                                    inclusive_start,
                                    inclusive_end,
                                    lazy_file,
                                );
                            } else if lazy_file.is_some() {
                                // Lazy load: take file, load node, put file back
                                let offset = internal.children_offsets[i];
                                let Some(mut f) = lazy_file.take() else {
                                    break;
                                };
                                match BPlusTree::load_node(&mut f, offset) {
                                    Ok(child_node) => {
                                        *lazy_file = Some(f);
                                        count += count_range(
                                            &child_node,
                                            start,
                                            end,
                                            inclusive_start,
                                            inclusive_end,
                                            lazy_file,
                                        );
                                    }
                                    Err(e) => {
                                        *lazy_file = Some(f);
                                        tracing::warn!(
                                            offset,
                                            error = %e,
                                            "count_range: failed to load lazy child node"
                                        );
                                    }
                                }
                            }
                        }
                    }
                    count
                }
            }
        }
        count_range(
            &self.root,
            start,
            end,
            inclusive_start,
            inclusive_end,
            lazy_file,
        )
    }

    /// Count entries in range, validating each entry against the document catalog.
    ///
    /// Same traversal as `count_range_internal()`, but instead of `end_idx - start_idx`
    /// it checks every leaf entry's `doc_id` against the catalog HashMap.
    /// This filters out stale index entries (from dirty-marking bugs or hard restarts).
    ///
    /// Memory: O(1) — only a counter; the catalog reference is already in memory.
    /// Time: O(N) — single traversal, per-entry O(1) HashMap lookup.
    pub fn count_range_validated(
        &self,
        start: &IndexKey,
        end: &IndexKey,
        inclusive_start: bool,
        inclusive_end: bool,
        catalog: &std::collections::HashMap<crate::DocumentId, u64>,
    ) -> usize {
        // LAZY MODE: open file for on-demand child loading (same pattern as range_query)
        let mut lazy_file = if self.lazy_mode {
            self.source_path.as_ref().and_then(|path| {
                File::open(path)
                    .map_err(|e| {
                        tracing::warn!(
                            index = %self.metadata.name,
                            path = %path.display(),
                            error = %e,
                            "count_range_validated: failed to open lazy index file"
                        );
                        e
                    })
                    .ok()
            })
        } else {
            None
        };

        fn count_validated(
            node: &BTreeNode,
            start: &IndexKey,
            end: &IndexKey,
            inclusive_start: bool,
            inclusive_end: bool,
            catalog: &std::collections::HashMap<crate::DocumentId, u64>,
            lazy_file: &mut Option<File>,
        ) -> usize {
            match node {
                BTreeNode::Leaf(leaf) => {
                    let start_idx = if inclusive_start {
                        leaf.keys.partition_point(|k| k < start)
                    } else {
                        leaf.keys.partition_point(|k| k <= start)
                    };
                    let end_idx = if inclusive_end {
                        leaf.keys.partition_point(|k| k <= end)
                    } else {
                        leaf.keys.partition_point(|k| k < end)
                    };
                    leaf.document_ids[start_idx..end_idx]
                        .iter()
                        .filter(|doc_id| catalog.contains_key(doc_id))
                        .count()
                }
                BTreeNode::Internal(internal) => {
                    let mut count = 0;
                    let child_count = if !internal.children.is_empty() {
                        internal.children.len()
                    } else {
                        internal.children_offsets.len()
                    };
                    for i in 0..child_count {
                        let child_min_might_match = i == 0 || &internal.keys[i - 1] <= end;
                        let child_max_might_match =
                            i >= internal.keys.len() || &internal.keys[i] >= start;
                        if child_min_might_match && child_max_might_match {
                            if i < internal.children.len() {
                                count += count_validated(
                                    &internal.children[i],
                                    start,
                                    end,
                                    inclusive_start,
                                    inclusive_end,
                                    catalog,
                                    lazy_file,
                                );
                            } else if lazy_file.is_some() {
                                let offset = internal.children_offsets[i];
                                let Some(mut f) = lazy_file.take() else {
                                    break;
                                };
                                match BPlusTree::load_node(&mut f, offset) {
                                    Ok(child_node) => {
                                        *lazy_file = Some(f);
                                        count += count_validated(
                                            &child_node,
                                            start,
                                            end,
                                            inclusive_start,
                                            inclusive_end,
                                            catalog,
                                            lazy_file,
                                        );
                                    }
                                    Err(e) => {
                                        *lazy_file = Some(f);
                                        tracing::warn!(
                                            offset,
                                            error = %e,
                                            "count_range_validated: failed to load lazy child node"
                                        );
                                    }
                                }
                            }
                        }
                    }
                    count
                }
            }
        }
        count_validated(
            &self.root,
            start,
            end,
            inclusive_start,
            inclusive_end,
            catalog,
            &mut lazy_file,
        )
    }

    /// Internal: Scan ascending with skip/limit and early termination.
    /// Supports lazy mode via optional file handle for on-demand child loading.
    fn scan_asc_internal(
        &self,
        start: &IndexKey,
        end: &IndexKey,
        inclusive_start: bool,
        inclusive_end: bool,
        skip: usize,
        limit: Option<usize>,
        lazy_file: &mut Option<File>,
    ) -> Vec<DocumentId> {
        let mut results = Vec::new();
        let mut skipped = 0usize;
        let limit_count = limit.unwrap_or(usize::MAX);

        fn scan_asc(
            node: &BTreeNode,
            start: &IndexKey,
            end: &IndexKey,
            inclusive_start: bool,
            inclusive_end: bool,
            results: &mut Vec<DocumentId>,
            skipped: &mut usize,
            skip: usize,
            limit_count: usize,
            lazy_file: &mut Option<File>,
        ) -> bool {
            // returns true if limit reached
            match node {
                BTreeNode::Leaf(leaf) => {
                    let start_idx = if inclusive_start {
                        leaf.keys.partition_point(|k| k < start)
                    } else {
                        leaf.keys.partition_point(|k| k <= start)
                    };
                    let end_idx = if inclusive_end {
                        leaf.keys.partition_point(|k| k <= end)
                    } else {
                        leaf.keys.partition_point(|k| k < end)
                    };

                    for idx in start_idx..end_idx {
                        if *skipped < skip {
                            *skipped += 1;
                            continue;
                        }
                        if results.len() >= limit_count {
                            return true; // Early termination!
                        }
                        if idx < leaf.document_ids.len() {
                            results.push(leaf.document_ids[idx].clone());
                        }
                    }
                    false
                }
                BTreeNode::Internal(internal) => {
                    let child_count = if !internal.children.is_empty() {
                        internal.children.len()
                    } else {
                        internal.children_offsets.len()
                    };
                    for i in 0..child_count {
                        let child_min_might_match = i == 0 || &internal.keys[i - 1] <= end;
                        let child_max_might_match =
                            i >= internal.keys.len() || &internal.keys[i] >= start;
                        if child_min_might_match && child_max_might_match {
                            let reached_limit = if i < internal.children.len() {
                                scan_asc(
                                    &internal.children[i],
                                    start,
                                    end,
                                    inclusive_start,
                                    inclusive_end,
                                    results,
                                    skipped,
                                    skip,
                                    limit_count,
                                    lazy_file,
                                )
                            } else if lazy_file.is_some() {
                                let offset = internal.children_offsets[i];
                                let Some(mut f) = lazy_file.take() else {
                                    break;
                                };
                                let result = match BPlusTree::load_node(&mut f, offset) {
                                    Ok(child_node) => {
                                        *lazy_file = Some(f);
                                        scan_asc(
                                            &child_node,
                                            start,
                                            end,
                                            inclusive_start,
                                            inclusive_end,
                                            results,
                                            skipped,
                                            skip,
                                            limit_count,
                                            lazy_file,
                                        )
                                    }
                                    Err(e) => {
                                        *lazy_file = Some(f);
                                        tracing::warn!(
                                            offset,
                                            error = %e,
                                            "scan_asc: failed to load lazy child node"
                                        );
                                        false
                                    }
                                };
                                result
                            } else {
                                false
                            };
                            if reached_limit {
                                return true;
                            }
                        }
                    }
                    false
                }
            }
        }

        scan_asc(
            &self.root,
            start,
            end,
            inclusive_start,
            inclusive_end,
            &mut results,
            &mut skipped,
            skip,
            limit_count,
            lazy_file,
        );
        results
    }

    /// Internal: Scan ascending with key+doc_id pairs and early termination
    fn scan_asc_pairs_internal(
        &self,
        start: &IndexKey,
        end: &IndexKey,
        inclusive_start: bool,
        inclusive_end: bool,
        skip: usize,
        limit: Option<usize>,
        lazy_file: &mut Option<File>,
    ) -> Vec<(IndexKey, DocumentId)> {
        let mut results = Vec::new();
        let mut skipped = 0usize;
        let limit_count = limit.unwrap_or(usize::MAX);

        fn scan_asc_pairs(
            node: &BTreeNode,
            start: &IndexKey,
            end: &IndexKey,
            inclusive_start: bool,
            inclusive_end: bool,
            results: &mut Vec<(IndexKey, DocumentId)>,
            skipped: &mut usize,
            skip: usize,
            limit_count: usize,
            lazy_file: &mut Option<File>,
        ) -> bool {
            match node {
                BTreeNode::Leaf(leaf) => {
                    let start_idx = if inclusive_start {
                        leaf.keys.partition_point(|k| k < start)
                    } else {
                        leaf.keys.partition_point(|k| k <= start)
                    };
                    let end_idx = if inclusive_end {
                        leaf.keys.partition_point(|k| k <= end)
                    } else {
                        leaf.keys.partition_point(|k| k < end)
                    };

                    for idx in start_idx..end_idx {
                        if *skipped < skip {
                            *skipped += 1;
                            continue;
                        }
                        if results.len() >= limit_count {
                            return true;
                        }
                        if idx < leaf.document_ids.len() {
                            results.push((leaf.keys[idx].clone(), leaf.document_ids[idx].clone()));
                        }
                    }
                    false
                }
                BTreeNode::Internal(internal) => {
                    let child_count = if !internal.children.is_empty() {
                        internal.children.len()
                    } else {
                        internal.children_offsets.len()
                    };
                    for i in 0..child_count {
                        let child_min_might_match = i == 0 || &internal.keys[i - 1] <= end;
                        let child_max_might_match =
                            i >= internal.keys.len() || &internal.keys[i] >= start;
                        if child_min_might_match && child_max_might_match {
                            let reached_limit = if i < internal.children.len() {
                                scan_asc_pairs(
                                    &internal.children[i],
                                    start,
                                    end,
                                    inclusive_start,
                                    inclusive_end,
                                    results,
                                    skipped,
                                    skip,
                                    limit_count,
                                    lazy_file,
                                )
                            } else if lazy_file.is_some() {
                                let offset = internal.children_offsets[i];
                                let Some(mut f) = lazy_file.take() else {
                                    break;
                                };
                                let result = match BPlusTree::load_node(&mut f, offset) {
                                    Ok(child_node) => {
                                        *lazy_file = Some(f);
                                        scan_asc_pairs(
                                            &child_node,
                                            start,
                                            end,
                                            inclusive_start,
                                            inclusive_end,
                                            results,
                                            skipped,
                                            skip,
                                            limit_count,
                                            lazy_file,
                                        )
                                    }
                                    Err(e) => {
                                        *lazy_file = Some(f);
                                        tracing::warn!(
                                            offset,
                                            error = %e,
                                            "scan_asc_pairs: failed to load lazy child node"
                                        );
                                        false
                                    }
                                };
                                result
                            } else {
                                false
                            };
                            if reached_limit {
                                return true;
                            }
                        }
                    }
                    false
                }
            }
        }

        scan_asc_pairs(
            &self.root,
            start,
            end,
            inclusive_start,
            inclusive_end,
            &mut results,
            &mut skipped,
            skip,
            limit_count,
            lazy_file,
        );
        results
    }

    /// Walk leaf nodes in DESCENDING order (right-to-left traversal) with child pruning.
    /// Callback returns `true` to continue, `false` to stop early.
    /// Children whose key range is entirely outside [start, end] are skipped (pruned).
    fn walk_leaves_desc<F>(
        &self,
        callback: &mut F,
        start: &IndexKey,
        end: &IndexKey,
        lazy_file: &mut Option<File>,
    ) -> bool
    where
        F: FnMut(&LeafNode) -> bool,
    {
        Self::walk_leaves_desc_node(&self.root, callback, start, end, lazy_file)
    }

    fn walk_leaves_desc_node<F>(
        node: &BTreeNode,
        callback: &mut F,
        start: &IndexKey,
        end: &IndexKey,
        lazy_file: &mut Option<File>,
    ) -> bool
    where
        F: FnMut(&LeafNode) -> bool,
    {
        match node {
            BTreeNode::Leaf(leaf) => callback(leaf),
            BTreeNode::Internal(internal) => {
                let child_count = if !internal.children.is_empty() {
                    internal.children.len()
                } else {
                    internal.children_offsets.len()
                };

                // Traverse RIGHT to LEFT (descending), with child pruning.
                // B+ tree convention: child[i] covers keys in range
                //   [keys[i-1], keys[i]) where keys[-1] = -inf, keys[len] = +inf.
                // A child might match if its range overlaps [start, end].
                // Same pruning logic as scan_asc Internal branch, but reversed iteration.
                for i in (0..child_count).rev() {
                    let child_min_might_match = i == 0 || &internal.keys[i - 1] <= end;
                    let child_max_might_match =
                        i >= internal.keys.len() || &internal.keys[i] >= start;
                    if !(child_min_might_match && child_max_might_match) {
                        continue; // This child's range doesn't overlap [start, end]
                    }

                    let should_continue = if i < internal.children.len() {
                        Self::walk_leaves_desc_node(
                            &internal.children[i],
                            callback,
                            start,
                            end,
                            lazy_file,
                        )
                    } else if lazy_file.is_some() {
                        let offset = internal.children_offsets[i];
                        let Some(mut f) = lazy_file.take() else {
                            break;
                        };
                        match BPlusTree::load_node(&mut f, offset) {
                            Ok(child_node) => {
                                *lazy_file = Some(f);
                                Self::walk_leaves_desc_node(
                                    &child_node,
                                    callback,
                                    start,
                                    end,
                                    lazy_file,
                                )
                            }
                            Err(e) => {
                                *lazy_file = Some(f);
                                tracing::warn!(
                                    offset,
                                    error = %e,
                                    "walk_leaves_desc: failed to load lazy child node"
                                );
                                true // Continue despite error
                            }
                        }
                    } else {
                        true
                    };

                    if !should_continue {
                        return false; // Early termination from callback
                    }
                }
                true
            }
        }
    }

    /// Internal: Scan descending with skip/limit and early termination.
    /// O(k) memory where k = limit, NOT O(n) like before.
    fn scan_desc_internal(
        &self,
        start: &IndexKey,
        end: &IndexKey,
        inclusive_start: bool,
        inclusive_end: bool,
        skip: usize,
        limit: Option<usize>,
        lazy_file: &mut Option<File>,
    ) -> Vec<DocumentId> {
        let mut results = Vec::new();
        let mut skipped = 0usize;
        let limit_count = limit.unwrap_or(usize::MAX);

        // Pre-allocate for expected results (bounded by limit)
        if limit_count < 10_000 {
            let _ = results.try_reserve(limit_count);
        }

        self.walk_leaves_desc(
            &mut |leaf| {
                // Descending: iterate keys right-to-left within each leaf
                for idx in (0..leaf.keys.len()).rev() {
                    let key = &leaf.keys[idx];

                    // Check if key is in range, respecting inclusive/exclusive boundaries
                    // (mirrors scan_asc_internal logic)
                    let below_start = if inclusive_start {
                        key < start
                    } else {
                        key <= start
                    };
                    let above_end = if inclusive_end { key > end } else { key >= end };

                    if below_start || above_end {
                        // Descending order: if key < start, all remaining keys in this
                        // leaf (and all leaves to the left) are also < start → stop early
                        if below_start {
                            return false;
                        }
                        continue;
                    }

                    // Apply skip
                    if skipped < skip {
                        skipped += 1;
                        continue;
                    }

                    // Collect result
                    if idx < leaf.document_ids.len() {
                        results.push(leaf.document_ids[idx].clone());
                    }

                    // Check limit (early termination!)
                    if results.len() >= limit_count {
                        return false; // Stop walking
                    }
                }
                true // Continue to next leaf
            },
            start,
            end,
            lazy_file,
        );

        results
    }

    /// Internal: Scan descending with key+doc_id pairs and early termination.
    /// O(k) memory where k = limit, NOT O(n) like before.
    fn scan_desc_pairs_internal(
        &self,
        start: &IndexKey,
        end: &IndexKey,
        inclusive_start: bool,
        inclusive_end: bool,
        skip: usize,
        limit: Option<usize>,
        lazy_file: &mut Option<File>,
    ) -> Vec<(IndexKey, DocumentId)> {
        let mut results = Vec::new();
        let mut skipped = 0usize;
        let limit_count = limit.unwrap_or(usize::MAX);

        // Pre-allocate for expected results (bounded by limit)
        if limit_count < 10_000 {
            let _ = results.try_reserve(limit_count);
        }

        self.walk_leaves_desc(
            &mut |leaf| {
                for idx in (0..leaf.keys.len()).rev() {
                    let key = &leaf.keys[idx];

                    // Check if key is in range, respecting inclusive/exclusive boundaries
                    let below_start = if inclusive_start {
                        key < start
                    } else {
                        key <= start
                    };
                    let above_end = if inclusive_end { key > end } else { key >= end };

                    if below_start || above_end {
                        if below_start {
                            return false;
                        }
                        continue;
                    }

                    if skipped < skip {
                        skipped += 1;
                        continue;
                    }
                    if idx < leaf.document_ids.len() {
                        results.push((key.clone(), leaf.document_ids[idx].clone()));
                    }
                    if results.len() >= limit_count {
                        return false; // Stop walking
                    }
                }
                true // Continue to next leaf
            },
            start,
            end,
            lazy_file,
        );

        results
    }

    /// Range scan that returns (key, doc_id) pairs for resumable streaming.
    pub fn range_query_pairs(
        &self,
        start: &IndexKey,
        end: &IndexKey,
        inclusive_start: bool,
        inclusive_end: bool,
        skip: usize,
        limit: Option<usize>,
        order: ScanOrder,
    ) -> Vec<(IndexKey, DocumentId)> {
        // LAZY MODE: open file for on-demand child loading
        let mut file = if self.lazy_mode {
            self.source_path.as_ref().and_then(|path| {
                File::open(path)
                    .map_err(|e| {
                        tracing::warn!(
                            index = %self.metadata.name,
                            path = %path.display(),
                            error = %e,
                            "range_query_pairs: failed to open lazy index file"
                        );
                        e
                    })
                    .ok()
            })
        } else {
            None
        };

        match order {
            ScanOrder::Asc => self.scan_asc_pairs_internal(
                start,
                end,
                inclusive_start,
                inclusive_end,
                skip,
                limit,
                &mut file,
            ),
            ScanOrder::Desc => self.scan_desc_pairs_internal(
                start,
                end,
                inclusive_start,
                inclusive_end,
                skip,
                limit,
                &mut file,
            ),
        }
    }

    /// Build range bounds for compound index prefix query
    ///
    /// For a compound index on (country, city) with query `{"country": "US"}`:
    /// - Returns start: `Compound([String("US"), Null])`
    /// - Returns end: `Compound([String("US"), MaxKey])`
    ///
    /// This allows range_scan to find all entries where the first field matches.
    ///
    /// # Arguments
    /// * `prefix_value` - The value for the first field (e.g., `IndexKey::String("US")`)
    ///
    /// # Returns
    /// A tuple of (start_key, end_key) for use with `range_scan()`
    pub fn build_prefix_range(&self, prefix_value: IndexKey) -> (IndexKey, IndexKey) {
        let num_fields = self.metadata.fields.len();

        if num_fields <= 1 {
            // Single-field index: just return the prefix value as both bounds
            return (prefix_value.clone(), prefix_value);
        }

        // Build start key: prefix + Nulls for remaining fields
        let mut start_parts = vec![prefix_value.clone()];
        for _ in 1..num_fields {
            start_parts.push(IndexKey::Null);
        }

        // Build end key: prefix + MaxKeys for remaining fields
        let mut end_parts = vec![prefix_value];
        for _ in 1..num_fields {
            end_parts.push(IndexKey::MaxKey);
        }

        (
            IndexKey::Compound(start_parts),
            IndexKey::Compound(end_parts),
        )
    }

    /// Get index size (number of keys)
    pub fn size(&self) -> u64 {
        self.metadata.num_keys
    }

    /// Count actual keys in the tree by traversing all leaf nodes
    /// Used after loading from file to fix potentially stale metadata
    fn count_actual_keys(&self) -> u64 {
        Self::count_keys_in_node(&self.root)
    }

    /// Recursively count keys in a node
    fn count_keys_in_node(node: &BTreeNode) -> u64 {
        match node {
            BTreeNode::Leaf(leaf) => leaf.keys.len() as u64,
            BTreeNode::Internal(internal) => internal
                .children
                .iter()
                .map(|c| Self::count_keys_in_node(c))
                .sum(),
        }
    }

    /// Sync metadata.num_keys with actual tree content
    /// Call this after loading from file to ensure consistency
    pub fn sync_num_keys(&mut self) {
        self.metadata.num_keys = self.count_actual_keys();
    }

    /// Refresh index statistics by scanning all keys in leaf nodes.
    ///
    /// This method traverses all leaf nodes (which are already sorted in B+ tree order)
    /// and counts distinct keys and null values in a single O(n) pass.
    ///
    /// Memory usage: O(k) where k is the number of leaf nodes (only pointers stored).
    ///
    /// # Example
    /// ```ignore
    /// let mut tree = BPlusTree::new("idx".into(), "field".into(), false, false);
    /// tree.insert(IndexKey::String("a".into()), 1).unwrap();
    /// tree.insert(IndexKey::String("b".into()), 2).unwrap();
    /// tree.refresh_stats();
    /// assert_eq!(tree.metadata.stats.distinct_count, 2);
    /// ```
    /// Refresh index statistics by streaming through all keys.
    ///
    /// For indexes with >= 100,000 entries, also builds an equi-depth histogram
    /// for better range query selectivity estimation.
    ///
    /// - Small indexes (<100k): O(n) time, O(1) memory
    /// - Large indexes (>=100k): O(n log n) time, O(n) memory (for histogram sort)
    pub fn refresh_stats(&mut self) {
        use std::time::{SystemTime, UNIX_EPOCH};

        // First pass: count total keys to decide if histogram is needed
        let total_keys = self.metadata.num_keys as usize;
        let build_histogram = total_keys >= Histogram::MIN_ENTRIES_FOR_HISTOGRAM
            && !self.metadata.is_compound()
            && !self.metadata.multikey;

        if build_histogram {
            // Large index: collect all values for histogram (O(n) memory)
            self.refresh_stats_with_histogram();
        } else {
            // Small index: streaming stats only (O(1) memory)
            self.refresh_stats_streaming();
        }

        // Update timestamp
        self.metadata.stats.last_analyzed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.metadata.stats.sample_rate = 1.0;

        // Multikey ratio heuristic
        if self.metadata.multikey {
            self.metadata.stats.multikey_ratio = 0.25;
        } else {
            self.metadata.stats.multikey_ratio = 0.0;
        }

        // Reset staleness counters after refresh
        self.counters.mark_refreshed(self.metadata.num_keys);

        // Build MCV for equality selectivity on non-compound indexes
        if total_keys >= MostCommonValues::MIN_ENTRIES_FOR_MCV && !self.metadata.is_compound() {
            self.metadata.stats.mcv = self.build_mcv(MostCommonValues::DEFAULT_MCV_SIZE);
        } else {
            self.metadata.stats.mcv = None;
        }
    }

    /// Streaming stats refresh - O(1) memory, no histogram
    fn refresh_stats_streaming(&mut self) {
        let mut distinct_count: u64 = 0;
        let mut null_count: u64 = 0;
        let mut last_key: Option<IndexKey> = None;

        Self::walk_leaves_asc(&self.root, &mut |leaf| {
            for key in &leaf.keys {
                if last_key.as_ref() != Some(key) {
                    distinct_count += 1;
                    last_key = Some(key.clone());
                }
                if matches!(key, IndexKey::Null) {
                    null_count += 1;
                }
            }
        });

        self.metadata.stats.distinct_count = distinct_count;
        self.metadata.stats.null_count = null_count;
        self.metadata.stats.histogram = None;
    }

    /// Stats refresh with histogram building - O(n) memory
    fn refresh_stats_with_histogram(&mut self) {
        let estimated_size = self.metadata.num_keys as usize;

        let mut all_keys: Vec<IndexKey> = Vec::new();
        let mut null_count: u64 = 0;

        // Try to pre-allocate, fall back to streaming if OOM
        if all_keys.try_reserve(estimated_size).is_err() {
            self.refresh_stats_streaming();
            return;
        }

        // Collect all keys (O(n) memory)
        Self::walk_leaves_asc(&self.root, &mut |leaf| {
            for key in &leaf.keys {
                if matches!(key, IndexKey::Null) {
                    null_count += 1;
                } else {
                    all_keys.push(key.clone());
                }
            }
        });

        // Count distinct values (keys are already sorted in B+ tree leaves)
        let distinct_count = {
            let mut count = 0u64;
            let mut last_key: Option<&IndexKey> = None;
            for key in &all_keys {
                if last_key != Some(key) {
                    count += 1;
                    last_key = Some(key);
                }
            }
            count
        };

        // Build equi-depth histogram with 64 buckets
        let histogram = if all_keys.len() >= Histogram::MIN_ENTRIES_FOR_HISTOGRAM {
            // Keys from B+ tree leaves are already sorted!
            // No need to sort again - this is a key optimization
            let bucket_count = 64u32;
            let bucket_size = all_keys.len() / bucket_count as usize;

            // Collect bucket boundaries (63 boundaries for 64 buckets)
            let boundaries: Vec<IndexKey> = (1..bucket_count as usize)
                .map(|i| all_keys[i * bucket_size].clone())
                .collect();

            Some(Histogram {
                boundaries,
                min_value: all_keys.first().cloned(),
                max_value: all_keys.last().cloned(),
                bucket_count,
            })
        } else {
            None
        };

        self.metadata.stats.distinct_count = distinct_count;
        self.metadata.stats.null_count = null_count;
        self.metadata.stats.histogram = histogram;
    }

    /// Build Most Common Values (MCV) for equality selectivity estimation
    ///
    /// Uses streaming approach with bounded memory:
    /// - Counts frequency of each distinct value
    /// - Returns top `mcv_size` values sorted by frequency
    /// - Uses sampling if distinct count exceeds MAX_TRACKED_DISTINCT
    ///
    /// # Memory Efficiency
    ///
    /// Memory usage is bounded by O(min(distinct_count, MAX_TRACKED_DISTINCT))
    fn build_mcv(&self, mcv_size: usize) -> Option<MostCommonValues> {
        use std::collections::HashMap;

        let num_keys = self.metadata.num_keys as usize;
        if num_keys < MostCommonValues::MIN_ENTRIES_FOR_MCV {
            return None;
        }

        // Use HashMap to count frequencies
        // B+ tree leaves are sorted, so consecutive equal keys are together
        // We can use run-length encoding for efficiency
        let mut freq_map: HashMap<IndexKey, u64> = HashMap::new();

        // Try to reserve capacity (fallback if OOM)
        let estimated_distinct = self.metadata.stats.distinct_count.max(1) as usize;
        let capacity = estimated_distinct.min(MostCommonValues::MAX_TRACKED_DISTINCT);
        if freq_map.try_reserve(capacity).is_err() {
            // OOM - skip MCV building
            return None;
        }

        let mut total_counted: u64 = 0;
        let mut last_key: Option<IndexKey> = None;
        let mut current_count: u64 = 0;

        // Single pass through leaves using run-length encoding
        Self::walk_leaves_asc(&self.root, &mut |leaf| {
            for key in &leaf.keys {
                if last_key.as_ref() == Some(key) {
                    // Same key - increment count
                    current_count += 1;
                } else {
                    // New key - flush previous
                    if let Some(prev_key) = last_key.take() {
                        // Check if we're tracking too many distinct values
                        if freq_map.len() < MostCommonValues::MAX_TRACKED_DISTINCT {
                            *freq_map.entry(prev_key).or_insert(0) += current_count;
                        }
                    }
                    last_key = Some(key.clone());
                    current_count = 1;
                }
                total_counted += 1;
            }
        });

        // Flush last key
        if let Some(prev_key) = last_key {
            if freq_map.len() < MostCommonValues::MAX_TRACKED_DISTINCT {
                *freq_map.entry(prev_key).or_insert(0) += current_count;
            }
        }

        if freq_map.is_empty() {
            return None;
        }

        // Extract top N by frequency
        let mut entries: Vec<(IndexKey, u64)> = freq_map.into_iter().collect();
        entries.sort_by(|a, b| b.1.cmp(&a.1)); // Sort by frequency DESC
        entries.truncate(mcv_size);

        let mcv_frequency_sum: u64 = entries.iter().map(|(_, f)| *f).sum();

        Some(MostCommonValues {
            values: entries,
            total_keys: total_counted,
            mcv_frequency_sum,
        })
    }

    /// Walk all leaf nodes in ASCENDING order (left-to-right traversal).
    /// O(1) memory - doesn't collect leaves into Vec.
    fn walk_leaves_asc<F>(node: &BTreeNode, callback: &mut F)
    where
        F: FnMut(&LeafNode),
    {
        match node {
            BTreeNode::Leaf(leaf) => callback(leaf),
            BTreeNode::Internal(internal) => {
                // Traverse children LEFT to RIGHT for ascending order
                for child in &internal.children {
                    Self::walk_leaves_asc(child, callback);
                }
            }
        }
    }

    // ===== FILE-BASED PERSISTENCE =====

    /// Save a single node to file and return its offset
    pub(crate) fn save_node(file: &mut File, node: &BTreeNode) -> Result<u64> {
        // Get current file position (where this node will be written)
        let offset = file.seek(SeekFrom::End(0))?;

        // Serialize node to JSON (more compatible than bincode with untagged enums)
        let node_json = serde_json::to_string(node).map_err(|e| {
            IronBaseError::Serialization(format!("Failed to serialize node: {}", e))
        })?;
        let node_bytes = node_json.as_bytes();

        // Ensure node fits in a page (4KB)
        if node_bytes.len() > NODE_PAGE_SIZE - 5 {
            return Err(IronBaseError::IndexError(format!(
                "Node size {} exceeds page size {}",
                node_bytes.len(),
                NODE_PAGE_SIZE - 5
            )));
        }

        // Create page buffer (4KB) and write node data
        let mut page = vec![0u8; NODE_PAGE_SIZE];

        // Write node type (1 byte)
        page[0] = match node {
            BTreeNode::Internal(_) => NODE_TYPE_INTERNAL,
            BTreeNode::Leaf(_) => NODE_TYPE_LEAF,
        };

        // Write data length (4 bytes, u32)
        let len_bytes = (node_bytes.len() as u32).to_le_bytes();
        page[1..5].copy_from_slice(&len_bytes);

        // Write node data
        page[5..(5 + node_bytes.len())].copy_from_slice(node_bytes);

        // Write page to file
        file.write_all(&page)?;
        file.flush()?;

        Ok(offset)
    }

    /// Load a node from file given its offset
    pub(crate) fn load_node(file: &mut File, offset: u64) -> Result<BTreeNode> {
        // Seek to node offset
        file.seek(SeekFrom::Start(offset))?;

        // Read page (4KB)
        let mut page = vec![0u8; NODE_PAGE_SIZE];
        file.read_exact(&mut page)?;

        // Read node type
        let node_type = page[0];

        // Read data length
        let len_bytes: [u8; 4] = page[1..5].try_into().unwrap();
        let data_len = u32::from_le_bytes(len_bytes) as usize;

        // Read node data
        let node_bytes = &page[5..(5 + data_len)];

        // Deserialize node from JSON
        let node_json = std::str::from_utf8(node_bytes).map_err(|e| {
            IronBaseError::Serialization(format!("Invalid UTF-8 in node data: {}", e))
        })?;
        let node: BTreeNode = serde_json::from_str(node_json).map_err(|e| {
            IronBaseError::Serialization(format!("Failed to deserialize node: {}", e))
        })?;

        // Verify node type matches
        match (&node, node_type) {
            (BTreeNode::Internal(_), NODE_TYPE_INTERNAL) => Ok(node),
            (BTreeNode::Leaf(_), NODE_TYPE_LEAF) => Ok(node),
            _ => Err(IronBaseError::Corruption(format!(
                "Node type mismatch at offset {}",
                offset
            ))),
        }
    }

    /// Save entire tree to file (recursive)
    ///
    /// File format:
    /// - First 8 bytes: root_offset (u64 little-endian)
    /// - Remaining bytes: serialized nodes
    pub fn save_to_file(&mut self, file: &mut File) -> Result<u64> {
        use std::io::{Seek, SeekFrom, Write};

        // Reserve first 8 bytes for root offset (will be written at the end)
        file.seek(SeekFrom::Start(0))?;
        file.write_all(&[0u8; 8])?;

        // Clone root to avoid borrowing issues
        let root_clone = self.root.clone();
        let root_offset = self.save_node_recursive(file, &root_clone)?;
        self.metadata.root_offset = root_offset;

        // Write root offset at the beginning of the file
        file.seek(SeekFrom::Start(0))?;
        file.write_all(&root_offset.to_le_bytes())?;

        Ok(root_offset)
    }

    /// Save node and children recursively
    /// Saves in-memory children first, then saves the parent with updated offsets
    fn save_node_recursive(&mut self, file: &mut File, node: &BTreeNode) -> Result<u64> {
        match node {
            BTreeNode::Internal(internal) => {
                // First, recursively save all in-memory children and collect their offsets
                let mut saved_offsets = Vec::new();

                if !internal.children.is_empty() {
                    // Save in-memory children
                    for child in &internal.children {
                        let child_offset = self.save_node_recursive(file, child)?;
                        saved_offsets.push(child_offset);
                    }
                } else {
                    // No in-memory children - preserve existing offsets
                    saved_offsets = internal.children_offsets.clone();
                }

                // Create new internal node with updated offsets
                let updated_node = BTreeNode::Internal(InternalNode {
                    keys: internal.keys.clone(),
                    children: Vec::new(), // Children not serialized (serde skip)
                    children_offsets: saved_offsets,
                });

                // Save this internal node
                Self::save_node(file, &updated_node)
            }
            BTreeNode::Leaf(_) => {
                // Leaf nodes can be saved directly
                Self::save_node(file, node)
            }
        }
    }

    /// Load tree from file (EAGER - loads all nodes)
    ///
    /// File format:
    /// - First 8 bytes: root_offset (u64 little-endian)
    /// - Remaining bytes: serialized nodes
    ///
    /// Recursively loads all children into memory for full tree traversal support.
    /// For lazy loading (memory efficient), use `load_from_path` instead.
    pub fn load_from_file(file: &mut File, mut metadata: IndexMetadata) -> Result<Self> {
        use std::io::{Read, Seek, SeekFrom};

        // Read root offset from the file header (first 8 bytes)
        file.seek(SeekFrom::Start(0))?;
        let mut offset_bytes = [0u8; 8];
        file.read_exact(&mut offset_bytes)?;
        let root_offset = u64::from_le_bytes(offset_bytes);
        metadata.root_offset = root_offset;

        // Load root node recursively (includes all children)
        let root = Box::new(Self::load_node_recursive(file, root_offset)?);

        let mut tree = BPlusTree {
            root,
            metadata,
            counters: StatisticsCounters::default(),
            source_path: None,
            lazy_mode: false,
            persisted_size: 0,
        };
        // Sync num_keys with actual tree content
        // (metadata from CollectionData may be stale if not synced on every insert/delete)
        tree.sync_num_keys();

        // Validate statistics against actual index size
        tree.metadata.stats.validate_and_fix(tree.metadata.num_keys);

        Ok(tree)
    }

    /// Load tree from path with optional lazy loading
    ///
    /// Loads the tree from the specified .idx file path.
    /// Uses **lazy loading** if file size exceeds the RAM-based threshold
    /// (calculated by `calculate_lazy_threshold()`).
    ///
    /// # Lazy Loading Behavior
    ///
    /// - **Below threshold**: Eager loading (all nodes in memory immediately)
    /// - **Above threshold**: Only root node loaded, children loaded on-demand
    ///   via `ensure_fully_loaded()` before search/insert/delete operations
    ///
    /// # Arguments
    /// * `path` - Path to the .idx file
    /// * `metadata` - Index metadata from collection catalog
    pub fn load_from_path(path: PathBuf, mut metadata: IndexMetadata) -> Result<Self> {
        use super::traits::calculate_lazy_threshold;
        use std::io::{Read, Seek, SeekFrom};

        let mut file = File::open(&path)?;

        // Get file size for threshold comparison
        let file_size = file.seek(SeekFrom::End(0))?;
        let lazy_threshold = calculate_lazy_threshold();
        let use_lazy = file_size > lazy_threshold;

        // Read root offset from the file header (first 8 bytes)
        file.seek(SeekFrom::Start(0))?;
        let mut offset_bytes = [0u8; 8];
        file.read_exact(&mut offset_bytes)?;
        let root_offset = u64::from_le_bytes(offset_bytes);
        metadata.root_offset = root_offset;

        if use_lazy {
            // LAZY LOADING: Only load root node, children loaded on-demand
            let root = Box::new(Self::load_node(&mut file, root_offset)?);

            tracing::debug!(
                index = %metadata.name,
                file_size_mb = file_size / (1024 * 1024),
                threshold_mb = lazy_threshold / (1024 * 1024),
                "B+ tree using lazy loading"
            );

            Ok(BPlusTree {
                root,
                metadata,
                counters: StatisticsCounters::default(),
                source_path: Some(path),
                lazy_mode: true,
                persisted_size: file_size,
            })
        } else {
            // EAGER LOADING: Load all nodes immediately
            let root = Box::new(Self::load_node_recursive(&mut file, root_offset)?);

            let mut tree = BPlusTree {
                root,
                metadata,
                counters: StatisticsCounters::default(),
                source_path: None,
                lazy_mode: false,
                persisted_size: file_size,
            };

            // Sync num_keys with actual tree content
            tree.sync_num_keys();

            // Validate statistics against actual index size
            tree.metadata.stats.validate_and_fix(tree.metadata.num_keys);

            Ok(tree)
        }
    }

    /// Ensure all children of an internal node are loaded into memory
    ///
    /// For lazy-loaded trees, this loads children from disk using their offsets.
    /// For in-memory trees, this is a no-op.
    #[allow(dead_code)]
    fn ensure_children_loaded(&self, node: &mut InternalNode) -> Result<()> {
        // If children are already loaded, nothing to do
        if !node.children.is_empty() {
            return Ok(());
        }

        // If no offsets, nothing to load (empty internal node)
        if node.children_offsets.is_empty() {
            return Ok(());
        }

        // Need source path to load from disk
        let path = match &self.source_path {
            Some(p) => p,
            None => {
                // This shouldn't happen - if we have offsets but no children,
                // we should have a source path
                return Err(IronBaseError::Corruption(
                    "Cannot load children: index has offsets but no source path".to_string(),
                ));
            }
        };

        // Open file and load each child
        let mut file = File::open(path)?;
        let mut children = Vec::with_capacity(node.children_offsets.len());
        for &child_offset in &node.children_offsets {
            let child = Self::load_node(&mut file, child_offset)?;
            children.push(Box::new(child));
        }
        node.children = children;

        Ok(())
    }

    /// Recursively ensure all nodes in the tree are loaded (converts lazy to eager)
    ///
    /// Use this before operations that need full tree access (like sync_num_keys).
    /// After loading, syncs num_keys with actual tree content.
    ///
    /// This method is idempotent - calling it multiple times is safe and fast
    /// (early return if already loaded).
    pub fn ensure_fully_loaded(&mut self) -> Result<()> {
        // If no source path, already fully in-memory
        let path = match &self.source_path {
            Some(p) => p.clone(),
            None => return Ok(()),
        };

        // Open file once for all loading
        let mut file = File::open(&path)?;

        // Load all children recursively
        Self::load_children_recursive(&mut self.root, &mut file)?;

        // After full load, we don't need source path anymore
        self.source_path = None;
        self.lazy_mode = false;

        // Sync num_keys with actual tree content (catalog metadata may be stale)
        self.sync_num_keys();

        // Validate statistics against actual index size
        self.metadata.stats.validate_and_fix(self.metadata.num_keys);

        tracing::debug!(
            index = %self.metadata.name,
            num_keys = self.metadata.num_keys,
            "B+ tree fully loaded from lazy mode"
        );

        Ok(())
    }

    /// Helper: recursively load all children from file
    fn load_children_recursive(node: &mut BTreeNode, file: &mut File) -> Result<()> {
        if let BTreeNode::Internal(ref mut internal) = node {
            // Load children if not loaded
            if internal.children.is_empty() && !internal.children_offsets.is_empty() {
                for &child_offset in &internal.children_offsets {
                    let child = Self::load_node(file, child_offset)?;
                    internal.children.push(Box::new(child));
                }
            }
            // Recursively load grandchildren
            for child in &mut internal.children {
                Self::load_children_recursive(child, file)?;
            }
        }
        Ok(())
    }

    /// Load a node and all its children recursively
    fn load_node_recursive(file: &mut File, offset: u64) -> Result<BTreeNode> {
        let mut node = Self::load_node(file, offset)?;

        // If internal node, recursively load all children
        if let BTreeNode::Internal(ref mut internal) = node {
            let mut children = Vec::new();
            for &child_offset in &internal.children_offsets {
                let child = Self::load_node_recursive(file, child_offset)?;
                children.push(Box::new(child));
            }
            internal.children = children;
        }

        Ok(node)
    }

    /// Two-Phase Commit: Phase 1 - Prepare changes to a temporary file
    /// Creates a .tmp file with the current index state
    /// Returns the path to the temporary file
    pub fn prepare_changes(&mut self, base_path: &PathBuf) -> Result<PathBuf> {
        use std::fs::OpenOptions;

        // Create temp file path: {base_path}.tmp
        let temp_path = base_path.with_extension("idx.tmp");

        // Open/create temp file (truncate if exists)
        let mut temp_file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&temp_path)
            .map_err(IronBaseError::Io)?;

        // Save current tree state to temp file
        self.save_to_file(&mut temp_file)?;

        // Ensure data is written to disk
        temp_file.sync_all().map_err(IronBaseError::Io)?;

        Ok(temp_path)
    }

    /// Two-Phase Commit: Phase 2 - Commit prepared changes atomically
    /// Performs atomic rename from temp file to final file
    /// If final_path doesn't exist yet, creates parent directories
    pub fn commit_prepared_changes(temp_path: &PathBuf, final_path: &PathBuf) -> Result<()> {
        use std::fs;

        // Ensure parent directory exists
        if let Some(parent) = final_path.parent() {
            fs::create_dir_all(parent).map_err(IronBaseError::Io)?;
        }

        // Atomic rename: temp → final
        fs::rename(temp_path, final_path).map_err(IronBaseError::Io)?;

        Ok(())
    }

    /// Rollback prepared changes by deleting the temp file
    pub fn rollback_prepared_changes(temp_path: &PathBuf) -> Result<()> {
        use std::fs;

        if temp_path.exists() {
            fs::remove_file(temp_path).map_err(IronBaseError::Io)?;
        }

        Ok(())
    }
}

// ============================================================================
// LazyLoadable Trait Implementation
// ============================================================================

use super::traits::{IndexTrait, LazyLoadable};

impl IndexTrait for BPlusTree {
    fn name(&self) -> &str {
        &self.metadata.name
    }

    fn fields(&self) -> Vec<&str> {
        self.metadata.fields.iter().map(|s| s.as_str()).collect()
    }

    fn entry_count(&self) -> usize {
        self.metadata.num_keys as usize
    }

    fn memory_usage_bytes(&self) -> usize {
        if self.lazy_mode {
            // Lazy mode: only root node and metadata in memory
            std::mem::size_of::<Self>() + 1024 // Approximate root node size
        } else {
            // Estimate based on num_keys: each entry ~100 bytes average
            std::mem::size_of::<Self>() + (self.metadata.num_keys as usize) * 100
        }
    }

    fn is_disk_backed(&self) -> bool {
        self.source_path.is_some() || self.persisted_size > 0
    }

    fn flush(&mut self) -> Result<()> {
        // B+ trees are written via save_to_file(), not incremental flush
        Ok(())
    }
}

impl LazyLoadable for BPlusTree {
    fn is_lazy_mode(&self) -> bool {
        self.lazy_mode
    }

    fn ensure_fully_loaded(&mut self) -> Result<()> {
        // Delegate to inherent method (BPlusTree::ensure_fully_loaded)
        BPlusTree::ensure_fully_loaded(self)
    }

    fn persisted_size_bytes(&self) -> Option<u64> {
        if self.persisted_size > 0 {
            Some(self.persisted_size)
        } else {
            None
        }
    }

    fn hot_ratio(&self) -> f32 {
        if self.lazy_mode {
            0.0
        } else {
            1.0
        }
    }
}
