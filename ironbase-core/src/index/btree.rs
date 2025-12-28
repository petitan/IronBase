// B+ Tree Index Implementation

use crate::document::DocumentId;
use crate::error::{IronBaseError, Result};
use crate::value_utils::get_nested_value;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;

use super::key::IndexKey;

// Node page constants (for file-based persistence)
pub const NODE_PAGE_SIZE: usize = 16384; // 16KB pages - supports long keys
const NODE_TYPE_INTERNAL: u8 = 0;
const NODE_TYPE_LEAF: u8 = 1;

/// Maximum keys per node before split is triggered
/// With 16KB pages, we can handle more keys per node
const MAX_KEYS_PER_NODE: usize = 128;

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
        // Check unique constraint if required - O(n) scan for adjacent duplicates
        if check_unique && entries.len() > 1 {
            for i in 0..entries.len() - 1 {
                if entries[i].0 == entries[i + 1].0 {
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
            }
        }
    }

    /// Get all entries from the index as a Vec
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

    /// Apply batch updates efficiently using HashMap + rebuild
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

        // Step 4: Clear existing tree and rebuild - O(n)
        // FIX: Must clear before rebuild to prevent duplicate entries!
        // Without this, build_from_sorted would ADD to existing entries.
        self.clear();
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
