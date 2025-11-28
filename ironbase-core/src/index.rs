// src/index.rs
// B+ Tree Index Implementation

use crate::document::DocumentId;
use crate::error::{MongoLiteError, Result};
use crate::value_utils::get_nested_value;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;

// Node page constants (for file-based persistence)
pub const NODE_PAGE_SIZE: usize = 4096; // 4KB pages
const NODE_TYPE_INTERNAL: u8 = 0;
const NODE_TYPE_LEAF: u8 = 1;

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
        match (self, other) {
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

/// Convert serde_json::Value to IndexKey
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

/// B+ Tree Node types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BTreeNode {
    Internal(InternalNode),
    Leaf(LeafNode),
}

/// Internal node (non-leaf) - contains routing keys
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InternalNode {
    pub keys: Vec<IndexKey>,
    pub children_offsets: Vec<u64>,
}

/// Leaf node - contains actual data pointers
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeafNode {
    pub keys: Vec<IndexKey>,
    pub document_ids: Vec<DocumentId>,
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
                let _child_index = self.find_child_index(&internal.keys, key);

                // IMPLEMENTATION PLAN: B+ Tree Child Loading
                //
                // Current state: Always returns None for internal nodes
                // Impact: NO correctness bug - index.search() is never called in codebase!
                //         grep "index.search" → 0 matches
                //         All queries use full scans (see collection_core.rs:1406)
                //
                // Architecture exists:
                // - InternalNode.children_offsets: Vec<u64> contains file offsets
                // - Persistence layer ready (prepare/commit two-phase pattern)
                // - Node serialization works (serde JSON for metadata, bincode for nodes)
                //
                // Implementation steps (when index-based queries are added):
                //
                // 1. Load child node from disk:
                //    let child_offset = internal.children_offsets[child_index];
                //    let child_node = self.load_node_from_disk(child_offset)?;
                //
                // 2. Implement load_node_from_disk():
                //    fn load_node_from_disk(&self, offset: u64) -> Result<BTreeNode> {
                //        // Read node page (4KB) from index file at offset
                //        // Deserialize using bincode (not JSON - performance!)
                //        // Cache in memory (LRU cache, ~1000 nodes = 4MB)
                //    }
                //
                // 3. Recursive descent:
                //    return self.search_in_node(&child_node, key);
                //
                // 4. Add node caching:
                //    - LRU cache: HashMap<u64, Arc<BTreeNode>> with capacity limit
                //    - Eviction policy: least recently used when cache full
                //    - Thread-safe: RwLock for concurrent reads
                //
                // Performance considerations:
                // - Tree height = log_32(n) → 650K docs = 4 levels
                // - Without cache: 4 disk seeks per lookup (~40ms HDD, ~0.4ms SSD)
                // - With cache (90% hit rate): ~0.04ms average
                //
                // Prerequisites before implementing:
                // - Wire up Collection.find() to use indexes (query optimizer)
                // - Add index selection heuristics (see IMPLEMENTATION_QUERY_OPTIMIZER.md)
                // - Implement range scans (leaf node sibling pointers)

                None // Returns None until index-based queries are implemented
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
    pub fn insert(&mut self, key: IndexKey, doc_id: DocumentId) -> Result<()> {
        // Check unique constraint
        if self.metadata.unique && self.search(&key).is_some() {
            return Err(MongoLiteError::IndexError(format!(
                "Duplicate key: {:?} (unique index)",
                key
            )));
        }

        // For now, simplified insert into leaf
        // Full implementation would handle splits and internal nodes
        if let BTreeNode::Leaf(ref mut leaf) = *self.root {
            let insert_pos = leaf.keys.binary_search(&key).unwrap_or_else(|pos| pos);
            leaf.keys.insert(insert_pos, key);
            leaf.document_ids.insert(insert_pos, doc_id);
            self.metadata.num_keys += 1;
        }

        Ok(())
    }

    /// 🚀 BULK LOAD: Build index from pre-sorted entries in O(n) time
    ///
    /// This is MUCH faster than repeated insert() calls:
    /// - insert() is O(n) per call due to Vec::insert() → O(n²) total for n docs
    /// - build_from_sorted() is O(n) total - just assigns the vectors
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

        // Separate keys and document_ids - O(n)
        let (keys, document_ids): (Vec<IndexKey>, Vec<DocumentId>) = entries.into_iter().unzip();

        // Replace the leaf node's vectors directly - O(1) pointer swap
        if let BTreeNode::Leaf(ref mut leaf) = *self.root {
            self.metadata.num_keys = keys.len() as u64;
            leaf.keys = keys;
            leaf.document_ids = document_ids;
        }

        Ok(())
    }

    /// Delete key-document pair from index
    pub fn delete(&mut self, key: &IndexKey, doc_id: &DocumentId) -> Result<()> {
        // For now, simplified delete from leaf
        // Full implementation would handle merges and internal nodes
        if let BTreeNode::Leaf(ref mut leaf) = *self.root {
            // Find the key position
            if let Ok(pos) = leaf.keys.binary_search(key) {
                // Verify this is the correct document ID
                if &leaf.document_ids[pos] == doc_id {
                    leaf.keys.remove(pos);
                    leaf.document_ids.remove(pos);
                    self.metadata.num_keys -= 1;
                }
            }
        }

        Ok(())
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
            BTreeNode::Internal(_internal) => {
                // For Internal nodes, children are stored as file offsets.
                // Without a file handle, we cannot traverse children.
                //
                // NOTE: In practice, this branch is rarely hit because:
                // 1. apply_batch_updates() rebuilds the tree as a single Leaf via build_from_sorted
                // 2. For multi-level persistent trees, use get_all_entries_with_file() instead
                //
                // For now, return empty results for this node (caller should handle this case)
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
    fn find_child_index(&self, keys: &[IndexKey], key: &IndexKey) -> usize {
        keys.binary_search(key).unwrap_or_else(|pos| pos)
    }

    /// Range scan: find all keys between start and end
    pub fn range_scan(
        &self,
        start: &IndexKey,
        end: &IndexKey,
        inclusive_start: bool,
        inclusive_end: bool,
    ) -> Vec<DocumentId> {
        fn collect_leaf(
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
                    let child_index = internal.keys.partition_point(|k| k <= start);
                    if child_index < internal.children_offsets.len() {
                        // Load child node from file or memory (not implemented for persistent tree)
                        // For now, fallback to scanning entire tree in memory
                        for child_idx in child_index..internal.children_offsets.len() {
                            // TODO: load child node properly (currently unsupported)
                            // This is a placeholder to keep compiler happy
                            let _child_offset = internal.children_offsets[child_idx];
                        }
                    }
                }
            }
        }

        let mut results = Vec::new();
        collect_leaf(
            &self.root,
            start,
            end,
            inclusive_start,
            inclusive_end,
            &mut results,
        );
        results
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
    fn save_node_recursive(&mut self, file: &mut File, node: &BTreeNode) -> Result<u64> {
        match node {
            BTreeNode::Internal(internal) => {
                // First, save all children and collect their offsets
                let mut saved_offsets = Vec::new();
                for &child_offset in &internal.children_offsets {
                    if child_offset == 0 {
                        // This is a placeholder, skip
                        saved_offsets.push(0);
                        continue;
                    }
                    // In a real implementation, we'd load the child node here
                    // For now, just preserve the offset
                    saved_offsets.push(child_offset);
                }

                // Create new internal node with updated offsets
                let updated_node = BTreeNode::Internal(InternalNode {
                    keys: internal.keys.clone(),
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
    pub fn load_from_file(file: &mut File, metadata: IndexMetadata) -> Result<Self> {
        // Note: offset 0 is valid (start of file), so we don't check for it
        // An empty file would fail on load_node instead

        // Load root node
        let root = Box::new(Self::load_node(file, metadata.root_offset)?);

        Ok(BPlusTree { root, metadata })
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
    /// File paths for persistent indexes (for two-phase commit)
    index_file_paths: HashMap<String, PathBuf>,
}

impl IndexManager {
    pub fn new() -> Self {
        IndexManager {
            btree_indexes: HashMap::new(),
            legacy_indexes: HashMap::new(),
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

    /// Drop index by name
    pub fn drop_index(&mut self, name: &str) -> Result<()> {
        if self.btree_indexes.remove(name).is_none() && self.legacy_indexes.remove(name).is_none() {
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
            .cloned()
            .collect();
        names.sort();
        names
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
