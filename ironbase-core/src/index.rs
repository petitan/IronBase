// src/index.rs
// B+ Tree Index Implementation + Fuzzy Text Index

use crate::document::DocumentId;
use crate::error::{MongoLiteError, Result};
use crate::value_utils::get_nested_value;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use strsim::{damerau_levenshtein, jaro_winkler, normalized_levenshtein};

// Node page constants (for file-based persistence)
pub const NODE_PAGE_SIZE: usize = 16384; // 16KB pages - supports long keys
const NODE_TYPE_INTERNAL: u8 = 0;
const NODE_TYPE_LEAF: u8 = 1;

/// Maximum keys per node before split is triggered
/// With 16KB pages, we can handle more keys per node
const MAX_KEYS_PER_NODE: usize = 128;

/// Index key - supported types for indexing
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum IndexKey {
    Null,
    Bool(bool),
    Int(i64),
    Float(OrderedFloat),
    String(String),
    /// Compound key for multi-field indexes (e.g., ["country", "city"])
    Compound(Vec<IndexKey>),
    /// Sentinel value for "greater than everything" - used for range scan upper bounds
    MaxKey,
}

/// OrderedFloat wrapper for f64 to enable Ord
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct OrderedFloat(pub f64);

impl PartialEq for OrderedFloat {
    fn eq(&self, other: &Self) -> bool {
        self.0.to_bits() == other.0.to_bits()
    }
}

impl Eq for OrderedFloat {}

impl PartialOrd for OrderedFloat {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for OrderedFloat {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match (self.0.is_nan(), other.0.is_nan()) {
            (true, true) => std::cmp::Ordering::Equal,
            (true, false) => std::cmp::Ordering::Greater,
            (false, true) => std::cmp::Ordering::Less,
            (false, false) => self
                .0
                .partial_cmp(&other.0)
                .unwrap_or(std::cmp::Ordering::Equal),
        }
    }
}

/// Implement Ord for IndexKey - defines ordering for B+ tree
impl PartialOrd for IndexKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for IndexKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        use IndexKey::*;
        // Ordering: Null < Bool < Int < Float < String < Compound < MaxKey
        match (self, other) {
            // MaxKey is greater than everything (except itself)
            (MaxKey, MaxKey) => std::cmp::Ordering::Equal,
            (MaxKey, _) => std::cmp::Ordering::Greater,
            (_, MaxKey) => std::cmp::Ordering::Less,

            (Null, Null) => std::cmp::Ordering::Equal,
            (Null, _) => std::cmp::Ordering::Less,
            (_, Null) => std::cmp::Ordering::Greater,

            (Bool(a), Bool(b)) => a.cmp(b),
            (Bool(_), _) => std::cmp::Ordering::Less,
            (_, Bool(_)) => std::cmp::Ordering::Greater,

            (Int(a), Int(b)) => a.cmp(b),
            (Int(_), _) => std::cmp::Ordering::Less,
            (_, Int(_)) => std::cmp::Ordering::Greater,

            (Float(a), Float(b)) => a.cmp(b),
            (Float(_), _) => std::cmp::Ordering::Less,
            (_, Float(_)) => std::cmp::Ordering::Greater,

            (String(a), String(b)) => a.cmp(b),
            (String(_), Compound(_)) => std::cmp::Ordering::Less,

            // Compound keys - compare element by element (lexicographic order)
            (Compound(a), Compound(b)) => a.cmp(b),
            (Compound(_), _) => std::cmp::Ordering::Greater,
        }
    }
}

/// Convert serde_json::Value reference to IndexKey (borrows, must clone strings)
impl From<&serde_json::Value> for IndexKey {
    fn from(value: &serde_json::Value) -> Self {
        match value {
            serde_json::Value::Null => IndexKey::Null,
            serde_json::Value::Bool(b) => IndexKey::Bool(*b),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    IndexKey::Int(i)
                } else if let Some(f) = n.as_f64() {
                    IndexKey::Float(OrderedFloat(f))
                } else {
                    IndexKey::Null
                }
            }
            serde_json::Value::String(s) => IndexKey::String(s.clone()),
            _ => IndexKey::Null, // Arrays and objects -> Null for simple index
        }
    }
}

/// Convert owned serde_json::Value to IndexKey (takes ownership, zero-copy for strings)
impl From<serde_json::Value> for IndexKey {
    fn from(value: serde_json::Value) -> Self {
        match value {
            serde_json::Value::Null => IndexKey::Null,
            serde_json::Value::Bool(b) => IndexKey::Bool(b),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    IndexKey::Int(i)
                } else if let Some(f) = n.as_f64() {
                    IndexKey::Float(OrderedFloat(f))
                } else {
                    IndexKey::Null
                }
            }
            serde_json::Value::String(s) => IndexKey::String(s), // Zero-copy: takes ownership
            _ => IndexKey::Null, // Arrays and objects -> Null for simple index
        }
    }
}

/// Index prefix information for QueryPlanner (compound index aware)
#[derive(Debug, Clone)]
pub struct IndexPrefixInfo {
    /// Index name
    pub index_name: String,
    /// First (prefix) field name - used for matching queries
    pub prefix_field: String,
    /// Whether this is a compound index
    pub is_compound: bool,
    /// Total number of fields in the index
    pub num_fields: usize,
}

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
    pub num_keys: u64,
    pub tree_height: u32,
    #[serde(default)]
    pub root_offset: u64, // File offset to root node (0 = in-memory only)
}

impl IndexMetadata {
    /// Check if this is a compound index (multiple fields)
    pub fn is_compound(&self) -> bool {
        self.fields.len() > 1
    }
}

impl BPlusTree {
    /// Create new B+ tree index (single field)
    pub fn new(name: String, field: String, unique: bool) -> Self {
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
                sparse: false,
                num_keys: 0,
                tree_height: 1,
                root_offset: 0,
            },
        }
    }

    /// Create new compound B+ tree index (multiple fields)
    ///
    /// # Arguments
    /// * `name` - Index name
    /// * `fields` - List of fields in order (e.g., ["country", "city"])
    /// * `unique` - Whether the compound key must be unique
    ///
    /// # Example
    /// ```rust,ignore
    /// let index = BPlusTree::new_compound(
    ///     "users_location".to_string(),
    ///     vec!["country".to_string(), "city".to_string()],
    ///     false
    /// );
    /// ```
    pub fn new_compound(name: String, fields: Vec<String>, unique: bool) -> Self {
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
                sparse: false,
                num_keys: 0,
                tree_height: 1,
                root_offset: 0,
            },
        }
    }

    /// Extract compound key from a document
    ///
    /// For compound indexes, creates an IndexKey::Compound from multiple fields
    /// For single-field indexes, returns a simple IndexKey
    pub fn extract_key(&self, doc: &serde_json::Value) -> IndexKey {
        if self.metadata.is_compound() {
            let keys: Vec<IndexKey> = self
                .metadata
                .fields
                .iter()
                .map(|field| {
                    get_nested_value(doc, field)
                        .map(IndexKey::from)
                        .unwrap_or(IndexKey::Null)
                })
                .collect();
            IndexKey::Compound(keys)
        } else {
            get_nested_value(doc, &self.metadata.field)
                .map(IndexKey::from)
                .unwrap_or(IndexKey::Null)
        }
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
            return Err(MongoLiteError::IndexError(format!(
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

            // Create new root with the split result
            self.root = Box::new(BTreeNode::Internal(InternalNode {
                keys: vec![separator],
                children: vec![old_root, right_child],
                children_offsets: Vec::new(),
            }));
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
                    return Err(MongoLiteError::IndexError(
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
        // Check unique constraint if required - O(n) scan for adjacent duplicates
        if check_unique && entries.len() > 1 {
            for i in 0..entries.len() - 1 {
                if entries[i].0 == entries[i + 1].0 {
                    return Err(MongoLiteError::IndexError(format!(
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

        Ok(())
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
    fn delete_from_node(node: &mut Box<BTreeNode>, key: &IndexKey, doc_id: &DocumentId) -> bool {
        match **node {
            BTreeNode::Leaf(ref mut leaf) => {
                // Find the key position in leaf
                if let Ok(pos) = leaf.keys.binary_search(key) {
                    // Verify this is the correct document ID
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
                // Note: Full B+ tree implementation would handle underflow and merges here
            }
        }
    }

    /// 🚀 BATCH OPTIMIZATION: Get all entries from the index as a Vec
    /// This allows O(n) extraction for batch rebuild operations
    ///
    /// NOTE: This method now supports multi-level B+ trees through recursive traversal.
    /// For Internal nodes, it recursively collects entries from all children.
    pub fn get_all_entries(&self) -> Vec<(IndexKey, DocumentId)> {
        let mut results = Vec::new();
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
    pub fn get_all_entries_with_file(
        &self,
        file: &mut File,
    ) -> Result<Vec<(IndexKey, DocumentId)>> {
        let mut results = Vec::new();
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

    /// 🚀 BATCH OPTIMIZATION: Apply batch updates efficiently using HashMap + rebuild
    ///
    /// Instead of O(n) per update (Vec::insert), this does:
    /// 1. Extract all entries to HashMap: O(n) - now supports multi-level trees!
    /// 2. Apply all updates to HashMap: O(k)
    /// 3. Rebuild index from sorted entries: O(n log n) for sort + O(n) for rebuild
    /// Total: O(n log n + k) instead of O(n * k)
    ///
    /// NOTE: This method now supports multi-level B+ trees through the improved
    /// get_all_entries() which recursively collects from all nodes.
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

        // Step 1: Extract all current entries into a BTreeMap (key -> doc_ids)
        // Use BTreeMap because IndexKey doesn't implement Hash (due to OrderedFloat)
        // but it does implement Ord. BTreeMap also maintains sorted order.
        //
        // NOTE: Now uses get_all_entries() which supports multi-level trees!
        use std::collections::BTreeMap;
        let mut entries_map: BTreeMap<IndexKey, Vec<DocumentId>> = BTreeMap::new();
        for (key, doc_id) in self.get_all_entries() {
            entries_map.entry(key).or_default().push(doc_id);
        }

        // Step 2: Apply all updates to the HashMap
        for (old_key, old_doc_id, new_key, new_doc_id) in updates {
            // Remove old entry
            if let Some(doc_ids) = entries_map.get_mut(&old_key) {
                doc_ids.retain(|id| id != &old_doc_id);
                if doc_ids.is_empty() {
                    entries_map.remove(&old_key);
                }
            }

            // Add new entry
            entries_map.entry(new_key).or_default().push(new_doc_id);
        }

        // Step 3: Convert back to sorted Vec for rebuild
        let mut entries: Vec<(IndexKey, DocumentId)> =
            Vec::with_capacity(entries_map.values().map(|v| v.len()).sum());
        for (key, doc_ids) in entries_map {
            for doc_id in doc_ids {
                entries.push((key.clone(), doc_id));
            }
        }

        // Sort by key - O(n log n)
        entries.sort_by(|a, b| a.0.cmp(&b.0));

        // Step 4: Rebuild index - O(n)
        self.build_from_sorted(entries, false)?;

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

    /// Range scan: find all keys between start and end
    /// Supports multi-level B+ trees by recursively traversing internal nodes
    pub fn range_scan(
        &self,
        start: &IndexKey,
        end: &IndexKey,
        inclusive_start: bool,
        inclusive_end: bool,
    ) -> Vec<DocumentId> {
        fn collect_range(
            node: &BTreeNode,
            start: &IndexKey,
            end: &IndexKey,
            inclusive_start: bool,
            inclusive_end: bool,
            results: &mut Vec<DocumentId>,
        ) {
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
                        if idx < leaf.document_ids.len() {
                            results.push(leaf.document_ids[idx].clone());
                        }
                    }
                }
                BTreeNode::Internal(internal) => {
                    // Find which children might contain keys in the range
                    // We need to check all children whose key range overlaps [start, end]
                    for (i, child) in internal.children.iter().enumerate() {
                        // Determine if this child could contain keys in range
                        let child_min_might_match = i == 0 || &internal.keys[i - 1] <= end;
                        let child_max_might_match =
                            i >= internal.keys.len() || &internal.keys[i] >= start;

                        if child_min_might_match && child_max_might_match {
                            collect_range(
                                child,
                                start,
                                end,
                                inclusive_start,
                                inclusive_end,
                                results,
                            );
                        }
                    }
                }
            }
        }

        let mut results = Vec::new();
        collect_range(
            &self.root,
            start,
            end,
            inclusive_start,
            inclusive_end,
            &mut results,
        );
        results
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

    // ===== FILE-BASED PERSISTENCE =====

    /// Save a single node to file and return its offset
    fn save_node(file: &mut File, node: &BTreeNode) -> Result<u64> {
        // Get current file position (where this node will be written)
        let offset = file.seek(SeekFrom::End(0))?;

        // Serialize node to JSON (more compatible than bincode with untagged enums)
        let node_json = serde_json::to_string(node).map_err(|e| {
            MongoLiteError::Serialization(format!("Failed to serialize node: {}", e))
        })?;
        let node_bytes = node_json.as_bytes();

        // Ensure node fits in a page (4KB)
        if node_bytes.len() > NODE_PAGE_SIZE - 5 {
            return Err(MongoLiteError::IndexError(format!(
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
    fn load_node(file: &mut File, offset: u64) -> Result<BTreeNode> {
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
            MongoLiteError::Serialization(format!("Invalid UTF-8 in node data: {}", e))
        })?;
        let node: BTreeNode = serde_json::from_str(node_json).map_err(|e| {
            MongoLiteError::Serialization(format!("Failed to deserialize node: {}", e))
        })?;

        // Verify node type matches
        match (&node, node_type) {
            (BTreeNode::Internal(_), NODE_TYPE_INTERNAL) => Ok(node),
            (BTreeNode::Leaf(_), NODE_TYPE_LEAF) => Ok(node),
            _ => Err(MongoLiteError::Corruption(format!(
                "Node type mismatch at offset {}",
                offset
            ))),
        }
    }

    /// Save entire tree to file (recursive)
    pub fn save_to_file(&mut self, file: &mut File) -> Result<u64> {
        // Clone root to avoid borrowing issues
        let root_clone = self.root.clone();
        let root_offset = self.save_node_recursive(file, &root_clone)?;
        self.metadata.root_offset = root_offset;
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

    /// Load tree from file given root offset
    /// Recursively loads all children into memory for full tree traversal support
    pub fn load_from_file(file: &mut File, metadata: IndexMetadata) -> Result<Self> {
        // Note: offset 0 is valid (start of file), so we don't check for it
        // An empty file would fail on load_node instead

        // Load root node recursively (includes all children)
        let root = Box::new(Self::load_node_recursive(file, metadata.root_offset)?);

        Ok(BPlusTree { root, metadata })
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
            .map_err(MongoLiteError::Io)?;

        // Save current tree state to temp file
        self.save_to_file(&mut temp_file)?;

        // Ensure data is written to disk
        temp_file.sync_all().map_err(MongoLiteError::Io)?;

        Ok(temp_path)
    }

    /// Two-Phase Commit: Phase 2 - Commit prepared changes atomically
    /// Performs atomic rename from temp file to final file
    /// If final_path doesn't exist yet, creates parent directories
    pub fn commit_prepared_changes(temp_path: &PathBuf, final_path: &PathBuf) -> Result<()> {
        use std::fs;

        // Ensure parent directory exists
        if let Some(parent) = final_path.parent() {
            fs::create_dir_all(parent).map_err(MongoLiteError::Io)?;
        }

        // Atomic rename: temp → final
        fs::rename(temp_path, final_path).map_err(MongoLiteError::Io)?;

        Ok(())
    }

    /// Rollback prepared changes by deleting the temp file
    pub fn rollback_prepared_changes(temp_path: &PathBuf) -> Result<()> {
        use std::fs;

        if temp_path.exists() {
            fs::remove_file(temp_path).map_err(MongoLiteError::Io)?;
        }

        Ok(())
    }
}

// ===== Legacy HashMap-based Index (for compatibility) =====

/// Index types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IndexType {
    Regular,
    Unique,
    Text,
    Geo2d,
}

// ===== Fuzzy Text Index =====

/// Fuzzy matching algorithm
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum FuzzyAlgorithm {
    /// Jaro-Winkler similarity (default) - fast, good for names
    #[default]
    JaroWinkler,
    /// Normalized Levenshtein distance - accurate edit distance
    Levenshtein,
    /// Damerau-Levenshtein - includes transpositions
    DamerauLevenshtein,
}

impl FuzzyAlgorithm {
    /// Calculate similarity between two strings (0.0 to 1.0)
    pub fn similarity(&self, a: &str, b: &str) -> f64 {
        let a_lower = a.to_lowercase();
        let b_lower = b.to_lowercase();
        match self {
            FuzzyAlgorithm::JaroWinkler => jaro_winkler(&a_lower, &b_lower),
            FuzzyAlgorithm::Levenshtein => normalized_levenshtein(&a_lower, &b_lower),
            FuzzyAlgorithm::DamerauLevenshtein => {
                let max_len = a_lower.len().max(b_lower.len());
                if max_len == 0 {
                    return 1.0;
                }
                let distance = damerau_levenshtein(&a_lower, &b_lower);
                1.0 - (distance as f64 / max_len as f64)
            }
        }
    }

    /// Parse algorithm name from string
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "jaro_winkler" | "jarowinkler" => Some(FuzzyAlgorithm::JaroWinkler),
            "levenshtein" => Some(FuzzyAlgorithm::Levenshtein),
            "damerau_levenshtein" | "dameraulevenshtein" => {
                Some(FuzzyAlgorithm::DamerauLevenshtein)
            }
            _ => None,
        }
    }
}

/// Fuzzy index metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FuzzyIndexMetadata {
    pub name: String,
    pub field: String,
    pub algorithm: FuzzyAlgorithm,
    pub threshold: f64,
    pub num_entries: usize,
}

/// Fuzzy text index - stores string values for similarity search
///
/// Unlike B+ tree indexes which support exact matching and range queries,
/// fuzzy indexes support similarity-based search using algorithms like
/// Jaro-Winkler or Levenshtein distance.
///
/// # Performance Characteristics
/// - Insert: O(1)
/// - Search: O(n) where n = number of indexed values
/// - Storage: ~40-60% overhead per indexed field
///
/// # Example
/// ```rust,ignore
/// let mut index = FuzzyIndex::new("name_fuzzy", "name", FuzzyAlgorithm::JaroWinkler, 0.8);
/// index.insert("John Smith", doc_id);
/// let matches = index.search("Jon Smyth", None); // Returns similar matches
/// ```
#[derive(Debug, Clone)]
pub struct FuzzyIndex {
    pub metadata: FuzzyIndexMetadata,
    /// Indexed entries: (lowercase_value, original_value, document_id)
    entries: Vec<(String, String, DocumentId)>,
}

impl FuzzyIndex {
    /// Create a new fuzzy index
    pub fn new(name: &str, field: &str, algorithm: FuzzyAlgorithm, threshold: f64) -> Self {
        FuzzyIndex {
            metadata: FuzzyIndexMetadata {
                name: name.to_string(),
                field: field.to_string(),
                algorithm,
                threshold: threshold.clamp(0.0, 1.0),
                num_entries: 0,
            },
            entries: Vec::new(),
        }
    }

    /// Insert a value into the fuzzy index
    pub fn insert(&mut self, value: &str, doc_id: DocumentId) {
        let lower = value.to_lowercase();
        self.entries.push((lower, value.to_string(), doc_id));
        self.metadata.num_entries = self.entries.len();
    }

    /// Remove a document from the fuzzy index
    pub fn remove(&mut self, doc_id: &DocumentId) {
        self.entries.retain(|(_, _, id)| id != doc_id);
        self.metadata.num_entries = self.entries.len();
    }

    /// Remove a specific value-document pair
    pub fn remove_value(&mut self, value: &str, doc_id: &DocumentId) {
        let lower = value.to_lowercase();
        self.entries
            .retain(|(l, _, id)| !(l == &lower && id == doc_id));
        self.metadata.num_entries = self.entries.len();
    }

    /// Search for similar values
    ///
    /// Returns document IDs where the indexed value has similarity >= threshold
    /// Optionally override the default threshold for this search
    pub fn search(&self, query: &str, threshold_override: Option<f64>) -> Vec<(DocumentId, f64)> {
        let threshold = threshold_override.unwrap_or(self.metadata.threshold);
        let query_lower = query.to_lowercase();
        let mut results = Vec::new();

        for (lower_value, _original, doc_id) in &self.entries {
            let similarity = self
                .metadata
                .algorithm
                .similarity(&query_lower, lower_value);
            if similarity >= threshold {
                results.push((doc_id.clone(), similarity));
            }
        }

        // Sort by similarity descending
        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        results
    }

    /// Search with algorithm override
    pub fn search_with_algorithm(
        &self,
        query: &str,
        algorithm: FuzzyAlgorithm,
        threshold: f64,
    ) -> Vec<(DocumentId, f64)> {
        let query_lower = query.to_lowercase();
        let mut results = Vec::new();

        for (lower_value, _original, doc_id) in &self.entries {
            let similarity = algorithm.similarity(&query_lower, lower_value);
            if similarity >= threshold {
                results.push((doc_id.clone(), similarity));
            }
        }

        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        results
    }

    /// Get index size
    pub fn size(&self) -> usize {
        self.metadata.num_entries
    }

    /// Clear the index
    pub fn clear(&mut self) {
        self.entries.clear();
        self.metadata.num_entries = 0;
    }

    /// Rebuild index from documents
    ///
    /// Clears existing entries and rebuilds from the provided documents
    pub fn rebuild<'a, I>(&mut self, documents: I)
    where
        I: Iterator<Item = (&'a serde_json::Value, &'a DocumentId)>,
    {
        self.entries.clear();

        for (doc, doc_id) in documents {
            if let Some(value) = get_nested_value(doc, &self.metadata.field) {
                if let Some(s) = value.as_str() {
                    self.insert(s, doc_id.clone());
                }
            }
        }
    }
}

/// Index definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexDefinition {
    pub name: String,
    pub field: String,
    pub index_type: IndexType,
    pub unique: bool,
}

/// Simple HashMap-based index (legacy)
pub struct Index {
    definition: IndexDefinition,
    entries: HashMap<String, Vec<DocumentId>>,
}

impl Index {
    pub fn new(definition: IndexDefinition) -> Self {
        Index {
            definition,
            entries: HashMap::new(),
        }
    }

    pub fn insert(&mut self, key: String, doc_id: DocumentId) -> Result<()> {
        if self.definition.unique && self.entries.contains_key(&key) {
            return Err(MongoLiteError::IndexError(format!(
                "Duplicate key: {} (unique index)",
                key
            )));
        }

        self.entries.entry(key).or_default().push(doc_id);

        Ok(())
    }

    pub fn find(&self, key: &str) -> Option<&Vec<DocumentId>> {
        self.entries.get(key)
    }

    pub fn remove(&mut self, key: &str, doc_id: &DocumentId) {
        if let Some(ids) = self.entries.get_mut(key) {
            ids.retain(|id| id != doc_id);
            if ids.is_empty() {
                self.entries.remove(key);
            }
        }
    }

    pub fn size(&self) -> usize {
        self.entries.len()
    }
}

/// Index Manager - manages all indexes for a collection
pub struct IndexManager {
    btree_indexes: HashMap<String, BPlusTree>,
    legacy_indexes: HashMap<String, Index>,
    /// Fuzzy text indexes for similarity search
    fuzzy_indexes: HashMap<String, FuzzyIndex>,
    /// File paths for persistent indexes (for two-phase commit)
    index_file_paths: HashMap<String, PathBuf>,
}

impl IndexManager {
    pub fn new() -> Self {
        IndexManager {
            btree_indexes: HashMap::new(),
            legacy_indexes: HashMap::new(),
            fuzzy_indexes: HashMap::new(),
            index_file_paths: HashMap::new(),
        }
    }

    /// Set file path for an index (required for two-phase commit)
    pub fn set_index_path(&mut self, index_name: &str, path: PathBuf) {
        self.index_file_paths.insert(index_name.to_string(), path);
    }

    /// Get file path for an index
    pub fn get_index_path(&self, index_name: &str) -> Option<&PathBuf> {
        self.index_file_paths.get(index_name)
    }

    /// Create B+ tree index (single field)
    pub fn create_btree_index(&mut self, name: String, field: String, unique: bool) -> Result<()> {
        if self.btree_indexes.contains_key(&name) {
            return Err(MongoLiteError::IndexError(format!(
                "Index already exists: {}",
                name
            )));
        }

        let tree = BPlusTree::new(name.clone(), field, unique);
        self.btree_indexes.insert(name, tree);
        Ok(())
    }

    /// Create compound B+ tree index (multiple fields)
    ///
    /// # Arguments
    /// * `name` - Index name
    /// * `fields` - Ordered list of fields (e.g., ["country", "city"])
    /// * `unique` - Whether the compound key must be unique
    ///
    /// # Example
    /// ```rust,ignore
    /// manager.create_compound_index(
    ///     "users_location".to_string(),
    ///     vec!["country".to_string(), "city".to_string()],
    ///     false
    /// )?;
    /// ```
    pub fn create_compound_index(
        &mut self,
        name: String,
        fields: Vec<String>,
        unique: bool,
    ) -> Result<()> {
        if self.btree_indexes.contains_key(&name) {
            return Err(MongoLiteError::IndexError(format!(
                "Index already exists: {}",
                name
            )));
        }

        if fields.is_empty() {
            return Err(MongoLiteError::IndexError(
                "Compound index must have at least one field".to_string(),
            ));
        }

        let tree = BPlusTree::new_compound(name.clone(), fields, unique);
        self.btree_indexes.insert(name, tree);
        Ok(())
    }

    /// Create legacy HashMap index
    pub fn create_index(&mut self, definition: IndexDefinition) -> Result<()> {
        let name = definition.name.clone();

        if self.legacy_indexes.contains_key(&name) {
            return Err(MongoLiteError::IndexError(format!(
                "Index already exists: {}",
                name
            )));
        }

        self.legacy_indexes.insert(name, Index::new(definition));
        Ok(())
    }

    /// Drop index by name (supports B+ tree, legacy, and fuzzy indexes)
    pub fn drop_index(&mut self, name: &str) -> Result<()> {
        let removed = self.btree_indexes.remove(name).is_some()
            || self.legacy_indexes.remove(name).is_some()
            || self.fuzzy_indexes.remove(name).is_some();

        if !removed {
            return Err(MongoLiteError::IndexError(format!(
                "Index not found: {}",
                name
            )));
        }
        // Also remove file path if it exists
        self.index_file_paths.remove(name);
        Ok(())
    }

    /// Get B+ tree index
    pub fn get_btree_index(&self, name: &str) -> Option<&BPlusTree> {
        self.btree_indexes.get(name)
    }

    /// Get B+ tree index (mutable)
    pub fn get_btree_index_mut(&mut self, name: &str) -> Option<&mut BPlusTree> {
        self.btree_indexes.get_mut(name)
    }

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
    ///
    /// Compound indexes can be used for prefix queries via range scans.
    pub fn list_indexes_with_compound_info(&self) -> Vec<IndexPrefixInfo> {
        let mut result: Vec<IndexPrefixInfo> = Vec::new();

        for (name, index) in &self.btree_indexes {
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
            });
        }

        // Legacy indexes are single-field only
        for (name, index) in &self.legacy_indexes {
            result.push(IndexPrefixInfo {
                index_name: name.clone(),
                prefix_field: index.definition.field.clone(),
                is_compound: false,
                num_fields: 1,
            });
        }

        result.sort_by(|a, b| a.index_name.cmp(&b.index_name));
        result
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
        // Check if any index with this name already exists
        if self.btree_indexes.contains_key(&name)
            || self.legacy_indexes.contains_key(&name)
            || self.fuzzy_indexes.contains_key(&name)
        {
            return Err(MongoLiteError::IndexError(format!(
                "Index already exists: {}",
                name
            )));
        }

        let index = FuzzyIndex::new(&name, &field, algorithm, threshold);
        self.fuzzy_indexes.insert(name, index);
        Ok(())
    }

    /// Get fuzzy index
    pub fn get_fuzzy_index(&self, name: &str) -> Option<&FuzzyIndex> {
        self.fuzzy_indexes.get(name)
    }

    /// Get fuzzy index (mutable)
    pub fn get_fuzzy_index_mut(&mut self, name: &str) -> Option<&mut FuzzyIndex> {
        self.fuzzy_indexes.get_mut(name)
    }

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
                let index_key = index.extract_key(doc);
                let is_null = Self::is_key_all_null(&index_key);

                // For unique indexes: include null keys (null is a value, enforce uniqueness)
                // For non-unique indexes: skip null keys (no query benefit)
                if !is_null || index.metadata.unique {
                    index.insert(index_key, doc_id.clone())?;
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
                let index_key = index.extract_key(doc);
                let is_null = Self::is_key_all_null(&index_key);

                // For unique indexes: remove null keys (they were inserted)
                // For non-unique indexes: skip null keys (they weren't inserted)
                if !is_null || index.metadata.unique {
                    index.delete(&index_key, doc_id)?;
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

            let index_key = index.extract_key(doc);

            // MongoDB behavior: null IS a value for unique constraint purposes
            // Do NOT skip null keys - duplicate nulls should be rejected

            // Check if key already exists
            if let Some(existing_id) = index.search(&index_key) {
                // Allow update to same document (exclude_doc_id matches)
                if exclude_doc_id != Some(&existing_id) {
                    // Format field names for error message
                    let fields_str = if index.metadata.is_compound() {
                        index.metadata.fields.join(", ")
                    } else {
                        index.metadata.field.clone()
                    };
                    return Err(MongoLiteError::IndexError(format!(
                        "Duplicate key: {:?} in field(s) '{}' (unique index)",
                        index_key, fields_str
                    )));
                }
            }
        }

        Ok(())
    }

    /// Helper: Check if an IndexKey is all Null
    fn is_key_all_null(key: &IndexKey) -> bool {
        match key {
            IndexKey::Null => true,
            IndexKey::Compound(keys) => keys.iter().all(|k| matches!(k, IndexKey::Null)),
            _ => false,
        }
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

    #[test]
    fn test_index_key_ordering() {
        assert!(IndexKey::Null < IndexKey::Bool(false));
        assert!(IndexKey::Bool(false) < IndexKey::Bool(true));
        assert!(IndexKey::Bool(true) < IndexKey::Int(0));
        assert!(IndexKey::Int(5) < IndexKey::Int(10));
        assert!(IndexKey::Int(10) < IndexKey::Float(OrderedFloat(10.5)));
        assert!(IndexKey::Float(OrderedFloat(10.5)) < IndexKey::String("a".to_string()));
        assert!(IndexKey::String("a".to_string()) < IndexKey::String("b".to_string()));
    }

    #[test]
    fn test_btree_insert_search() {
        let mut tree = BPlusTree::new("test_idx".to_string(), "age".to_string(), false);

        tree.insert(IndexKey::Int(25), DocumentId::Int(1)).unwrap();
        tree.insert(IndexKey::Int(30), DocumentId::Int(2)).unwrap();
        tree.insert(IndexKey::Int(20), DocumentId::Int(3)).unwrap();

        assert_eq!(tree.search(&IndexKey::Int(25)), Some(DocumentId::Int(1)));
        assert_eq!(tree.search(&IndexKey::Int(30)), Some(DocumentId::Int(2)));
        assert_eq!(tree.search(&IndexKey::Int(20)), Some(DocumentId::Int(3)));
        assert_eq!(tree.search(&IndexKey::Int(99)), None);
    }

    #[test]
    fn test_btree_unique_constraint() {
        let mut tree = BPlusTree::new("email_idx".to_string(), "email".to_string(), true);

        tree.insert(
            IndexKey::String("test@example.com".to_string()),
            DocumentId::Int(1),
        )
        .unwrap();

        let result = tree.insert(
            IndexKey::String("test@example.com".to_string()),
            DocumentId::Int(2),
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_btree_range_scan() {
        let mut tree = BPlusTree::new("age_idx".to_string(), "age".to_string(), false);

        for i in 0..100 {
            tree.insert(IndexKey::Int(i), DocumentId::Int(i)).unwrap();
        }

        let results = tree.range_scan(
            &IndexKey::Int(10),
            &IndexKey::Int(20),
            true,  // inclusive start
            false, // exclusive end
        );

        assert_eq!(results.len(), 10); // 10..19
    }

    #[test]
    fn test_node_save_load() {
        use std::fs::OpenOptions;

        // Create temporary file
        let temp_path = "test_node_io.tmp";
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(temp_path)
            .unwrap();

        // Create a leaf node
        let leaf = BTreeNode::Leaf(LeafNode {
            keys: vec![IndexKey::Int(10), IndexKey::Int(20), IndexKey::Int(30)],
            document_ids: vec![DocumentId::Int(1), DocumentId::Int(2), DocumentId::Int(3)],
            next_leaf_offset: 0,
        });

        // Save node
        let offset = BPlusTree::save_node(&mut file, &leaf).unwrap();
        assert_eq!(offset, 0); // First node at offset 0

        // Load node back
        let loaded = BPlusTree::load_node(&mut file, offset).unwrap();

        // Verify
        match (leaf, loaded) {
            (BTreeNode::Leaf(original), BTreeNode::Leaf(restored)) => {
                assert_eq!(original.keys, restored.keys);
                assert_eq!(original.document_ids, restored.document_ids);
                assert_eq!(original.next_leaf_offset, restored.next_leaf_offset);
            }
            _ => panic!("Expected leaf nodes"),
        }

        // Cleanup
        std::fs::remove_file(temp_path).ok();
    }

    #[test]
    fn test_tree_persistence() {
        use std::fs::OpenOptions;

        let temp_path = "test_tree_persist.tmp";

        // Create and populate tree
        let mut tree = BPlusTree::new("test_idx".to_string(), "age".to_string(), false);

        for i in 0..10 {
            tree.insert(IndexKey::Int(i * 10), DocumentId::Int(i))
                .unwrap();
        }

        // Save tree to file
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(temp_path)
            .unwrap();

        let root_offset = tree.save_to_file(&mut file).unwrap();
        // root_offset is u64, always >= 0, just verify it was set correctly
        assert_eq!(tree.metadata.root_offset, root_offset);

        // Load tree from file
        let metadata_clone = tree.metadata.clone();
        let loaded_tree = BPlusTree::load_from_file(&mut file, metadata_clone).unwrap();

        // Verify search still works
        assert_eq!(
            loaded_tree.search(&IndexKey::Int(0)),
            Some(DocumentId::Int(0))
        );
        assert_eq!(
            loaded_tree.search(&IndexKey::Int(50)),
            Some(DocumentId::Int(5))
        );
        assert_eq!(
            loaded_tree.search(&IndexKey::Int(90)),
            Some(DocumentId::Int(9))
        );
        assert_eq!(loaded_tree.search(&IndexKey::Int(99)), None);

        // Cleanup
        std::fs::remove_file(temp_path).ok();
    }

    #[test]
    fn test_compound_index_key_ordering() {
        // Test that compound keys are ordered lexicographically
        let key1 = IndexKey::Compound(vec![
            IndexKey::String("US".to_string()),
            IndexKey::String("NYC".to_string()),
        ]);
        let key2 = IndexKey::Compound(vec![
            IndexKey::String("US".to_string()),
            IndexKey::String("LA".to_string()),
        ]);
        let key3 = IndexKey::Compound(vec![
            IndexKey::String("CA".to_string()),
            IndexKey::String("Toronto".to_string()),
        ]);

        // CA < US, so key3 < key1
        assert!(key3 < key1);
        assert!(key3 < key2);

        // LA < NYC, so key2 < key1
        assert!(key2 < key1);
    }

    #[test]
    fn test_compound_index_create() {
        let tree = BPlusTree::new_compound(
            "users_location".to_string(),
            vec!["country".to_string(), "city".to_string()],
            false,
        );

        assert_eq!(tree.metadata.name, "users_location");
        assert_eq!(tree.metadata.field, "country"); // Primary field
        assert_eq!(
            tree.metadata.fields,
            vec!["country".to_string(), "city".to_string()]
        );
        assert!(tree.metadata.is_compound());
    }

    #[test]
    fn test_compound_index_extract_key() {
        let tree = BPlusTree::new_compound(
            "users_location".to_string(),
            vec!["country".to_string(), "city".to_string()],
            false,
        );

        let doc = serde_json::json!({
            "_id": 1,
            "name": "Alice",
            "country": "US",
            "city": "NYC"
        });

        let key = tree.extract_key(&doc);
        let expected = IndexKey::Compound(vec![
            IndexKey::String("US".to_string()),
            IndexKey::String("NYC".to_string()),
        ]);
        assert_eq!(key, expected);
    }

    #[test]
    fn test_compound_index_insert_search() {
        let mut tree = BPlusTree::new_compound(
            "users_location".to_string(),
            vec!["country".to_string(), "city".to_string()],
            false,
        );

        // Insert compound keys
        let key1 = IndexKey::Compound(vec![
            IndexKey::String("US".to_string()),
            IndexKey::String("NYC".to_string()),
        ]);
        let key2 = IndexKey::Compound(vec![
            IndexKey::String("US".to_string()),
            IndexKey::String("LA".to_string()),
        ]);
        let key3 = IndexKey::Compound(vec![
            IndexKey::String("CA".to_string()),
            IndexKey::String("Toronto".to_string()),
        ]);

        tree.insert(key1.clone(), DocumentId::Int(1)).unwrap();
        tree.insert(key2.clone(), DocumentId::Int(2)).unwrap();
        tree.insert(key3.clone(), DocumentId::Int(3)).unwrap();

        // Search should work
        assert_eq!(tree.search(&key1), Some(DocumentId::Int(1)));
        assert_eq!(tree.search(&key2), Some(DocumentId::Int(2)));
        assert_eq!(tree.search(&key3), Some(DocumentId::Int(3)));

        // Non-existent key
        let key_missing = IndexKey::Compound(vec![
            IndexKey::String("UK".to_string()),
            IndexKey::String("London".to_string()),
        ]);
        assert_eq!(tree.search(&key_missing), None);
    }

    #[test]
    fn test_compound_index_range_scan() {
        let mut tree = BPlusTree::new_compound(
            "users_location".to_string(),
            vec!["country".to_string(), "city".to_string()],
            false,
        );

        // Insert several compound keys
        let keys = vec![
            (vec!["CA", "Montreal"], 1),
            (vec!["CA", "Toronto"], 2),
            (vec!["CA", "Vancouver"], 3),
            (vec!["US", "Chicago"], 4),
            (vec!["US", "LA"], 5),
            (vec!["US", "NYC"], 6),
        ];

        for (fields, id) in &keys {
            let key = IndexKey::Compound(vec![
                IndexKey::String(fields[0].to_string()),
                IndexKey::String(fields[1].to_string()),
            ]);
            tree.insert(key, DocumentId::Int(*id)).unwrap();
        }

        // Range scan for all US cities
        let start = IndexKey::Compound(vec![
            IndexKey::String("US".to_string()),
            IndexKey::String("".to_string()), // Empty string sorts before any city
        ]);
        let end = IndexKey::Compound(vec![
            IndexKey::String("US".to_string()),
            IndexKey::String("\u{10ffff}".to_string()), // Max unicode sorts after any city
        ]);

        let results = tree.range_scan(&start, &end, true, true);
        assert_eq!(results.len(), 3); // Chicago, LA, NYC
    }

    #[test]
    fn test_compound_index_unique() {
        let mut tree = BPlusTree::new_compound(
            "users_location".to_string(),
            vec!["country".to_string(), "city".to_string()],
            true, // unique
        );

        let key = IndexKey::Compound(vec![
            IndexKey::String("US".to_string()),
            IndexKey::String("NYC".to_string()),
        ]);

        // First insert should succeed
        tree.insert(key.clone(), DocumentId::Int(1)).unwrap();

        // Second insert with same compound key should fail
        let result = tree.insert(key, DocumentId::Int(2));
        assert!(result.is_err());
    }

    #[test]
    fn test_index_manager_compound() {
        let mut manager = IndexManager::new();

        // Create compound index
        manager
            .create_compound_index(
                "users_country_city".to_string(),
                vec!["country".to_string(), "city".to_string()],
                false,
            )
            .unwrap();

        // Verify it exists
        let index = manager.get_btree_index("users_country_city").unwrap();
        assert!(index.metadata.is_compound());
        assert_eq!(index.metadata.fields.len(), 2);

        // Duplicate should fail
        let result = manager.create_compound_index(
            "users_country_city".to_string(),
            vec!["country".to_string(), "city".to_string()],
            false,
        );
        assert!(result.is_err());
    }
}

#[cfg(test)]
mod debug_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn debug_compound_index_search() {
        let mut manager = IndexManager::new();

        // Create compound index on (country, city)
        manager
            .create_compound_index(
                "loc_idx".to_string(),
                vec!["country".to_string(), "city".to_string()],
                true,
            )
            .unwrap();

        // Insert doc1: USA, NYC
        let doc1 = json!({"country": "USA", "city": "NYC"});
        manager
            .add_document_to_indexes(&doc1, &DocumentId::Int(1), None)
            .unwrap();

        // Insert doc2: USA, LA
        let doc2 = json!({"country": "USA", "city": "LA"});
        manager
            .add_document_to_indexes(&doc2, &DocumentId::Int(2), None)
            .unwrap();

        // Now check if doc with USA, NYC would violate unique constraint (excluding doc2)
        let doc_new = json!({"country": "USA", "city": "NYC"});
        let result = manager.check_unique_constraints(&doc_new, Some(&DocumentId::Int(2)), None);

        println!("Result: {:?}", result);
        assert!(result.is_err(), "Should find duplicate compound key!");
    }
}

#[cfg(test)]
mod fuzzy_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_fuzzy_algorithm_similarity() {
        let jw = FuzzyAlgorithm::JaroWinkler;
        let lev = FuzzyAlgorithm::Levenshtein;
        let dl = FuzzyAlgorithm::DamerauLevenshtein;

        // Exact match should be 1.0
        assert!((jw.similarity("john", "john") - 1.0).abs() < 0.001);
        assert!((lev.similarity("john", "john") - 1.0).abs() < 0.001);
        assert!((dl.similarity("john", "john") - 1.0).abs() < 0.001);

        // Similar strings should have high similarity
        assert!(jw.similarity("john", "jon") > 0.8);
        assert!(lev.similarity("john", "jon") > 0.7);

        // Transposition - DL should handle better
        assert!(dl.similarity("the", "teh") > 0.6);

        // Case insensitive
        assert!((jw.similarity("JOHN", "john") - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_fuzzy_algorithm_from_str() {
        assert_eq!(
            FuzzyAlgorithm::from_str("jaro_winkler"),
            Some(FuzzyAlgorithm::JaroWinkler)
        );
        assert_eq!(
            FuzzyAlgorithm::from_str("levenshtein"),
            Some(FuzzyAlgorithm::Levenshtein)
        );
        assert_eq!(
            FuzzyAlgorithm::from_str("damerau_levenshtein"),
            Some(FuzzyAlgorithm::DamerauLevenshtein)
        );
        assert_eq!(FuzzyAlgorithm::from_str("unknown"), None);
    }

    #[test]
    fn test_fuzzy_index_create() {
        let index = FuzzyIndex::new("name_idx", "name", FuzzyAlgorithm::JaroWinkler, 0.8);

        assert_eq!(index.metadata.name, "name_idx");
        assert_eq!(index.metadata.field, "name");
        assert_eq!(index.metadata.algorithm, FuzzyAlgorithm::JaroWinkler);
        assert!((index.metadata.threshold - 0.8).abs() < 0.001);
        assert_eq!(index.size(), 0);
    }

    #[test]
    fn test_fuzzy_index_insert_search() {
        let mut index = FuzzyIndex::new("name_idx", "name", FuzzyAlgorithm::JaroWinkler, 0.8);

        index.insert("John Smith", DocumentId::Int(1));
        index.insert("Jane Doe", DocumentId::Int(2));
        index.insert("Jon Snow", DocumentId::Int(3));

        assert_eq!(index.size(), 3);

        // Exact match
        let results = index.search("John Smith", None);
        assert!(!results.is_empty());
        assert_eq!(results[0].0, DocumentId::Int(1));

        // Fuzzy match - "Jon" should match "John" with JaroWinkler
        let results = index.search("Jon", None);
        // Should find "Jon Snow" (exact partial) and possibly "John Smith" (similar)
        assert!(!results.is_empty());
    }

    #[test]
    fn test_fuzzy_index_remove() {
        let mut index = FuzzyIndex::new("name_idx", "name", FuzzyAlgorithm::JaroWinkler, 0.8);

        index.insert("John", DocumentId::Int(1));
        index.insert("Jane", DocumentId::Int(2));
        assert_eq!(index.size(), 2);

        index.remove(&DocumentId::Int(1));
        assert_eq!(index.size(), 1);

        // Search should not find removed document
        let results = index.search("John", None);
        for (doc_id, _) in &results {
            assert_ne!(*doc_id, DocumentId::Int(1));
        }
    }

    #[test]
    fn test_fuzzy_index_threshold_override() {
        let mut index = FuzzyIndex::new("name_idx", "name", FuzzyAlgorithm::JaroWinkler, 0.9);

        index.insert("John", DocumentId::Int(1));
        index.insert("Jane", DocumentId::Int(2));

        // High threshold (default 0.9) - "Jon" might not match "John"
        let results_high = index.search("Jon", None);

        // Lower threshold - should find more matches
        let results_low = index.search("Jon", Some(0.7));

        // Lower threshold should return at least as many results
        assert!(results_low.len() >= results_high.len());
    }

    #[test]
    fn test_fuzzy_index_algorithm_override() {
        let mut index = FuzzyIndex::new("name_idx", "name", FuzzyAlgorithm::JaroWinkler, 0.8);

        index.insert("the", DocumentId::Int(1));
        index.insert("teh", DocumentId::Int(2)); // transposition

        // Search with Damerau-Levenshtein (good for transpositions)
        let results = index.search_with_algorithm("teh", FuzzyAlgorithm::DamerauLevenshtein, 0.6);

        assert!(!results.is_empty());
    }

    #[test]
    fn test_index_manager_fuzzy() {
        let mut manager = IndexManager::new();

        // Create fuzzy index
        manager
            .create_fuzzy_index(
                "name_fuzzy".to_string(),
                "name".to_string(),
                FuzzyAlgorithm::JaroWinkler,
                0.8,
            )
            .unwrap();

        // Verify it exists
        let index = manager.get_fuzzy_index("name_fuzzy").unwrap();
        assert_eq!(index.metadata.field, "name");

        // Duplicate should fail
        let result = manager.create_fuzzy_index(
            "name_fuzzy".to_string(),
            "name".to_string(),
            FuzzyAlgorithm::JaroWinkler,
            0.8,
        );
        assert!(result.is_err());

        // List should include fuzzy index
        let indexes = manager.list_indexes();
        assert!(indexes.contains(&"name_fuzzy".to_string()));

        // Drop should work
        manager.drop_index("name_fuzzy").unwrap();
        assert!(manager.get_fuzzy_index("name_fuzzy").is_none());
    }

    #[test]
    fn test_index_manager_fuzzy_with_documents() {
        let mut manager = IndexManager::new();

        // Create fuzzy index
        manager
            .create_fuzzy_index(
                "name_fuzzy".to_string(),
                "name".to_string(),
                FuzzyAlgorithm::JaroWinkler,
                0.8,
            )
            .unwrap();

        // Add documents
        let doc1 = json!({"name": "John Smith", "age": 30});
        let doc2 = json!({"name": "Jane Doe", "age": 25});

        manager
            .add_document_to_indexes(&doc1, &DocumentId::Int(1), None)
            .unwrap();
        manager
            .add_document_to_indexes(&doc2, &DocumentId::Int(2), None)
            .unwrap();

        // Check fuzzy index has entries
        let index = manager.get_fuzzy_index("name_fuzzy").unwrap();
        assert_eq!(index.size(), 2);

        // Search should work
        let results = index.search("John", None);
        assert!(!results.is_empty());

        // Remove document
        manager
            .remove_document_from_indexes(&doc1, &DocumentId::Int(1), None)
            .unwrap();

        let index = manager.get_fuzzy_index("name_fuzzy").unwrap();
        assert_eq!(index.size(), 1);
    }

    #[test]
    fn test_fuzzy_index_results_sorted_by_similarity() {
        let mut index = FuzzyIndex::new("name_idx", "name", FuzzyAlgorithm::JaroWinkler, 0.5);

        index.insert("John", DocumentId::Int(1));
        index.insert("Johnny", DocumentId::Int(2));
        index.insert("Jonathan", DocumentId::Int(3));
        index.insert("Jane", DocumentId::Int(4));

        let results = index.search("John", Some(0.5));

        // Results should be sorted by similarity (descending)
        for i in 0..results.len() - 1 {
            assert!(
                results[i].1 >= results[i + 1].1,
                "Results should be sorted by similarity descending"
            );
        }

        // "John" should be first (exact match = 1.0)
        if !results.is_empty() {
            assert_eq!(results[0].0, DocumentId::Int(1));
        }
    }
}

#[cfg(test)]
mod split_tests {
    use super::*;

    /// Test that inserting more than MAX_KEYS_PER_NODE triggers a split
    /// and increases tree height
    #[test]
    fn test_btree_insert_triggers_split() {
        let mut tree = BPlusTree::new("test_idx".to_string(), "field".to_string(), false);

        // Initial height should be 1 (just root leaf)
        assert_eq!(tree.metadata.tree_height, 1);

        // Insert MAX_KEYS_PER_NODE + 1 elements to trigger split
        for i in 0..=MAX_KEYS_PER_NODE {
            tree.insert(IndexKey::Int(i as i64), DocumentId::Int(i as i64))
                .unwrap();
        }

        // After split, tree height should be 2
        assert_eq!(
            tree.metadata.tree_height, 2,
            "Tree height should increase to 2 after split"
        );

        // Verify all elements are still searchable
        for i in 0..=MAX_KEYS_PER_NODE {
            assert_eq!(
                tree.search(&IndexKey::Int(i as i64)),
                Some(DocumentId::Int(i as i64)),
                "Element {} should be found after split",
                i
            );
        }
    }

    /// Test bulk loading a large dataset (10,000 entries)
    #[test]
    fn test_btree_bulk_load_large_dataset() {
        let mut tree = BPlusTree::new("large_idx".to_string(), "id".to_string(), false);

        let entries: Vec<_> = (0..10000)
            .map(|i| (IndexKey::Int(i), DocumentId::Int(i)))
            .collect();

        tree.build_from_sorted(entries, false).unwrap();

        // Verify count
        assert_eq!(tree.metadata.num_keys, 10000);

        // Verify tree has multiple levels
        assert!(
            tree.metadata.tree_height > 1,
            "Large dataset should create multi-level tree"
        );

        // Verify random samples are searchable
        assert_eq!(
            tree.search(&IndexKey::Int(0)),
            Some(DocumentId::Int(0)),
            "First element should be found"
        );
        assert_eq!(
            tree.search(&IndexKey::Int(5000)),
            Some(DocumentId::Int(5000)),
            "Middle element should be found"
        );
        assert_eq!(
            tree.search(&IndexKey::Int(9999)),
            Some(DocumentId::Int(9999)),
            "Last element should be found"
        );

        // Verify range scan works on multi-level tree
        let results = tree.range_scan(&IndexKey::Int(100), &IndexKey::Int(200), true, false);
        assert_eq!(results.len(), 100, "Range scan should return 100 elements");
    }

    /// Test that multiple splits create correct tree structure
    #[test]
    fn test_btree_multiple_splits() {
        let mut tree = BPlusTree::new("multi_split_idx".to_string(), "x".to_string(), false);

        // Insert enough elements to cause multiple levels of splits
        // With MAX_KEYS_PER_NODE = 128, we need > 128^2 = 16384 for 3 levels
        for i in 0..20000 {
            tree.insert(IndexKey::Int(i), DocumentId::Int(i)).unwrap();
        }

        // Verify tree height is at least 3
        assert!(
            tree.metadata.tree_height >= 3,
            "20000 elements should create tree with height >= 3, got {}",
            tree.metadata.tree_height
        );

        // Verify all elements are searchable
        for i in (0..20000).step_by(100) {
            assert_eq!(
                tree.search(&IndexKey::Int(i)),
                Some(DocumentId::Int(i)),
                "Element {} should be found in multi-level tree",
                i
            );
        }
    }

    /// Test persistence of multi-level tree
    #[test]
    fn test_btree_multilevel_persistence() {
        use std::fs::OpenOptions;

        let temp_path = "test_multilevel_persist.tmp";

        // Create and populate multi-level tree
        let mut tree = BPlusTree::new("persist_idx".to_string(), "id".to_string(), false);

        for i in 0..500 {
            tree.insert(IndexKey::Int(i), DocumentId::Int(i)).unwrap();
        }

        let original_height = tree.metadata.tree_height;
        assert!(original_height > 1, "Should have multi-level tree");

        // Save tree to file
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(temp_path)
            .unwrap();

        tree.save_to_file(&mut file).unwrap();

        // Load tree from file
        let metadata_clone = tree.metadata.clone();
        let loaded_tree = BPlusTree::load_from_file(&mut file, metadata_clone).unwrap();

        // Verify tree structure preserved
        assert_eq!(
            loaded_tree.metadata.tree_height, original_height,
            "Tree height should be preserved after load"
        );
        assert_eq!(
            loaded_tree.metadata.num_keys, 500,
            "Key count should be preserved"
        );

        // Verify search still works on loaded tree
        assert_eq!(
            loaded_tree.search(&IndexKey::Int(0)),
            Some(DocumentId::Int(0))
        );
        assert_eq!(
            loaded_tree.search(&IndexKey::Int(250)),
            Some(DocumentId::Int(250))
        );
        assert_eq!(
            loaded_tree.search(&IndexKey::Int(499)),
            Some(DocumentId::Int(499))
        );

        // Cleanup
        std::fs::remove_file(temp_path).ok();
    }
}

#[cfg(test)]
mod compound_prefix_tests {
    use super::*;

    /// Test MaxKey ordering - it should be greater than everything
    #[test]
    fn test_maxkey_ordering() {
        assert!(IndexKey::MaxKey > IndexKey::Null);
        assert!(IndexKey::MaxKey > IndexKey::Bool(true));
        assert!(IndexKey::MaxKey > IndexKey::Int(i64::MAX));
        assert!(IndexKey::MaxKey > IndexKey::Float(OrderedFloat(f64::MAX)));
        assert!(IndexKey::MaxKey > IndexKey::String("zzzzz".to_string()));
        assert!(IndexKey::MaxKey > IndexKey::Compound(vec![IndexKey::String("z".to_string())]));
        assert_eq!(IndexKey::MaxKey, IndexKey::MaxKey);
    }

    /// Test build_prefix_range for compound indexes
    #[test]
    fn test_build_prefix_range() {
        // Create compound index on (country, city)
        let tree = BPlusTree::new_compound(
            "users_country_city".to_string(),
            vec!["country".to_string(), "city".to_string()],
            false,
        );

        // Build prefix range for country = "US"
        let prefix = IndexKey::String("US".to_string());
        let (start, end) = tree.build_prefix_range(prefix);

        // Verify start: Compound(["US", Null])
        if let IndexKey::Compound(ref parts) = start {
            assert_eq!(parts.len(), 2);
            assert_eq!(parts[0], IndexKey::String("US".to_string()));
            assert_eq!(parts[1], IndexKey::Null);
        } else {
            panic!("Expected Compound key for start");
        }

        // Verify end: Compound(["US", MaxKey])
        if let IndexKey::Compound(ref parts) = end {
            assert_eq!(parts.len(), 2);
            assert_eq!(parts[0], IndexKey::String("US".to_string()));
            assert_eq!(parts[1], IndexKey::MaxKey);
        } else {
            panic!("Expected Compound key for end");
        }
    }

    /// Test compound index prefix query via range scan
    #[test]
    fn test_compound_prefix_query_range_scan() {
        let mut tree = BPlusTree::new_compound(
            "users_country_city".to_string(),
            vec!["country".to_string(), "city".to_string()],
            false,
        );

        // Insert data: US cities and HU cities
        let data = vec![
            ("HU", "Budapest", 1),
            ("HU", "Debrecen", 2),
            ("US", "LA", 3),
            ("US", "NYC", 4),
            ("US", "SF", 5),
        ];

        for (country, city, id) in &data {
            let key = IndexKey::Compound(vec![
                IndexKey::String(country.to_string()),
                IndexKey::String(city.to_string()),
            ]);
            tree.insert(key, DocumentId::Int(*id)).unwrap();
        }

        // Query: prefix = "US" (should find LA, NYC, SF)
        let prefix = IndexKey::String("US".to_string());
        let (start, end) = tree.build_prefix_range(prefix);
        let results = tree.range_scan(&start, &end, true, true);

        assert_eq!(results.len(), 3, "Should find 3 US cities");
        assert!(results.contains(&DocumentId::Int(3)));
        assert!(results.contains(&DocumentId::Int(4)));
        assert!(results.contains(&DocumentId::Int(5)));

        // Query: prefix = "HU" (should find Budapest, Debrecen)
        let prefix = IndexKey::String("HU".to_string());
        let (start, end) = tree.build_prefix_range(prefix);
        let results = tree.range_scan(&start, &end, true, true);

        assert_eq!(results.len(), 2, "Should find 2 HU cities");
        assert!(results.contains(&DocumentId::Int(1)));
        assert!(results.contains(&DocumentId::Int(2)));

        // Query: prefix = "DE" (should find nothing)
        let prefix = IndexKey::String("DE".to_string());
        let (start, end) = tree.build_prefix_range(prefix);
        let results = tree.range_scan(&start, &end, true, true);

        assert_eq!(results.len(), 0, "Should find no DE cities");
    }

    /// Test IndexPrefixInfo for compound indexes
    #[test]
    fn test_index_prefix_info_compound() {
        let mut manager = IndexManager::new();

        // Create single-field index
        manager
            .create_btree_index("users_age".to_string(), "age".to_string(), false)
            .unwrap();

        // Create compound index
        manager
            .create_compound_index(
                "users_country_city".to_string(),
                vec!["country".to_string(), "city".to_string()],
                false,
            )
            .unwrap();

        let infos = manager.list_indexes_with_compound_info();

        // Find single-field index
        let age_info = infos.iter().find(|i| i.index_name == "users_age").unwrap();
        assert_eq!(age_info.prefix_field, "age");
        assert!(!age_info.is_compound);
        assert_eq!(age_info.num_fields, 1);

        // Find compound index
        let compound_info = infos
            .iter()
            .find(|i| i.index_name == "users_country_city")
            .unwrap();
        assert_eq!(compound_info.prefix_field, "country");
        assert!(compound_info.is_compound);
        assert_eq!(compound_info.num_fields, 2);
    }
}
