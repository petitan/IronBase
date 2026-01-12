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
    /// Unwrap as count, panics if not a Count variant
    pub fn unwrap_count(self) -> usize {
        match self {
            RangeQueryResult::Count(c) => c,
            RangeQueryResult::Docs(_) => panic!("Expected Count, got Docs"),
        }
    }

    /// Unwrap as docs, panics if not a Docs variant
    pub fn unwrap_docs(self) -> Vec<DocumentId> {
        match self {
            RangeQueryResult::Docs(d) => d,
            RangeQueryResult::Count(_) => panic!("Expected Docs, got Count"),
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
                } else {
                    // No in-memory children - would need to load from disk
                    // This path is for file-based persistence (not yet implemented)
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
        Ok(())
    }

    /// Estimate serialized size of a node (approximate, for early split detection)
    fn estimate_node_size(node: &BTreeNode) -> usize {
        match node {
            BTreeNode::Leaf(leaf) => {
                let key_sizes: usize = leaf.keys.iter().map(Self::estimate_key_size).sum();
                let doc_id_overhead = leaf.document_ids.len() * 20;
                50 + key_sizes + doc_id_overhead
            }
            BTreeNode::Internal(internal) => {
                let key_sizes: usize = internal.keys.iter().map(Self::estimate_key_size).sum();
                50 + key_sizes + internal.children.len() * 100
            }
        }
    }

    /// Estimate serialized size of a key
    fn estimate_key_size(key: &IndexKey) -> usize {
        match key {
            IndexKey::Null => 4,
            IndexKey::Bool(_) => 5,
            IndexKey::Int(_) => 20,
            IndexKey::Float(_) => 20,
            IndexKey::String(s) => s.len() + 10,
            IndexKey::Compound(keys) => {
                keys.iter().map(Self::estimate_key_size).sum::<usize>() + 10
            }
            IndexKey::MaxKey => 10,
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
        let deleted = Self::delete_from_node(&mut self.root, key, doc_id);
        if deleted {
            self.metadata.num_keys -= 1;
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
    /// OOM Protection: Pre-allocates based on known num_keys.
    pub fn get_all_entries(&self) -> Vec<(IndexKey, DocumentId)> {
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
        match mode {
            RangeQueryMode::Count => {
                let count = self.count_range_internal(start, end, inclusive_start, inclusive_end);
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
                    ),
                    ScanOrder::Desc => self.scan_desc_internal(start, end, skip, limit),
                };
                RangeQueryResult::Docs(docs)
            }
        }
    }

    /// Internal: Count entries in range without materializing
    fn count_range_internal(
        &self,
        start: &IndexKey,
        end: &IndexKey,
        inclusive_start: bool,
        inclusive_end: bool,
    ) -> usize {
        fn count_range(
            node: &BTreeNode,
            start: &IndexKey,
            end: &IndexKey,
            inclusive_start: bool,
            inclusive_end: bool,
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
                    for (i, child) in internal.children.iter().enumerate() {
                        let child_min_might_match = i == 0 || &internal.keys[i - 1] <= end;
                        let child_max_might_match =
                            i >= internal.keys.len() || &internal.keys[i] >= start;
                        if child_min_might_match && child_max_might_match {
                            count += count_range(child, start, end, inclusive_start, inclusive_end);
                        }
                    }
                    count
                }
            }
        }
        count_range(&self.root, start, end, inclusive_start, inclusive_end)
    }

    /// Internal: Scan ascending with skip/limit and early termination
    fn scan_asc_internal(
        &self,
        start: &IndexKey,
        end: &IndexKey,
        inclusive_start: bool,
        inclusive_end: bool,
        skip: usize,
        limit: Option<usize>,
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
                    for (i, child) in internal.children.iter().enumerate() {
                        let child_min_might_match = i == 0 || &internal.keys[i - 1] <= end;
                        let child_max_might_match =
                            i >= internal.keys.len() || &internal.keys[i] >= start;
                        if child_min_might_match
                            && child_max_might_match
                            && scan_asc(
                                child,
                                start,
                                end,
                                inclusive_start,
                                inclusive_end,
                                results,
                                skipped,
                                skip,
                                limit_count,
                            )
                        {
                            return true; // Propagate early termination
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
                    for (i, child) in internal.children.iter().enumerate() {
                        let child_min_might_match = i == 0 || &internal.keys[i - 1] <= end;
                        let child_max_might_match =
                            i >= internal.keys.len() || &internal.keys[i] >= start;
                        if child_min_might_match
                            && child_max_might_match
                            && scan_asc_pairs(
                                child,
                                start,
                                end,
                                inclusive_start,
                                inclusive_end,
                                results,
                                skipped,
                                skip,
                                limit_count,
                            )
                        {
                            return true;
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
        );
        results
    }

    /// Walk all leaf nodes in DESCENDING order (right-to-left traversal).
    /// Callback returns `true` to continue, `false` to stop early.
    /// This avoids collecting all leaves into a Vec for LIMIT queries.
    fn walk_leaves_desc<F>(&self, callback: &mut F) -> bool
    where
        F: FnMut(&LeafNode) -> bool,
    {
        Self::walk_leaves_desc_node(&self.root, callback)
    }

    fn walk_leaves_desc_node<F>(node: &BTreeNode, callback: &mut F) -> bool
    where
        F: FnMut(&LeafNode) -> bool,
    {
        match node {
            BTreeNode::Leaf(leaf) => callback(leaf),
            BTreeNode::Internal(internal) => {
                // Traverse children RIGHT to LEFT for descending order
                for child in internal.children.iter().rev() {
                    if !Self::walk_leaves_desc_node(child, callback) {
                        return false; // Early termination
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
        skip: usize,
        limit: Option<usize>,
    ) -> Vec<DocumentId> {
        let mut results = Vec::new();
        let mut skipped = 0usize;
        let limit_count = limit.unwrap_or(usize::MAX);

        // Pre-allocate for expected results (bounded by limit)
        if limit_count < 10_000 {
            let _ = results.try_reserve(limit_count);
        }

        self.walk_leaves_desc(&mut |leaf| {
            for idx in (0..leaf.keys.len()).rev() {
                let key = &leaf.keys[idx];

                // Check if key is in range [start, end]
                if key < start || key > end {
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
        });

        results
    }

    /// Internal: Scan descending with key+doc_id pairs and early termination.
    /// O(k) memory where k = limit, NOT O(n) like before.
    fn scan_desc_pairs_internal(
        &self,
        start: &IndexKey,
        end: &IndexKey,
        skip: usize,
        limit: Option<usize>,
    ) -> Vec<(IndexKey, DocumentId)> {
        let mut results = Vec::new();
        let mut skipped = 0usize;
        let limit_count = limit.unwrap_or(usize::MAX);

        // Pre-allocate for expected results (bounded by limit)
        if limit_count < 10_000 {
            let _ = results.try_reserve(limit_count);
        }

        self.walk_leaves_desc(&mut |leaf| {
            for idx in (0..leaf.keys.len()).rev() {
                let key = &leaf.keys[idx];
                if key < start || key > end {
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
        });

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
        match order {
            ScanOrder::Asc => self.scan_asc_pairs_internal(
                start,
                end,
                inclusive_start,
                inclusive_end,
                skip,
                limit,
            ),
            ScanOrder::Desc => self.scan_desc_pairs_internal(start, end, skip, limit),
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

    /// Load tree from file
    ///
    /// File format:
    /// - First 8 bytes: root_offset (u64 little-endian)
    /// - Remaining bytes: serialized nodes
    ///
    /// Recursively loads all children into memory for full tree traversal support
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

        let mut tree = BPlusTree { root, metadata };
        // Sync num_keys with actual tree content
        // (metadata from CollectionData may be stale if not synced on every insert/delete)
        tree.sync_num_keys();

        // Validate statistics against actual index size
        tree.metadata.stats.validate_and_fix(tree.metadata.num_keys);

        Ok(tree)
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
