// src/index.rs
// B+ Tree Index Implementation

use std::collections::HashMap;
use std::io::{Read, Write, Seek, SeekFrom};
use std::fs::File;
use serde::{Serialize, Deserialize};
use crate::document::DocumentId;
use crate::error::{Result, MongoLiteError};

// B+ Tree Configuration
#[allow(dead_code)]
const BTREE_ORDER: usize = 32;
#[allow(dead_code)]
const MAX_KEYS: usize = BTREE_ORDER - 1;  // 31
#[allow(dead_code)]
const MIN_KEYS: usize = BTREE_ORDER / 2;   // 16

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
            (false, false) => self.0.partial_cmp(&other.0).unwrap_or(std::cmp::Ordering::Equal),
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
    pub next_leaf_offset: u64,  // File offset to next leaf node (0 = none)
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
    pub field: String,
    pub unique: bool,
    pub sparse: bool,
    pub num_keys: u64,
    pub tree_height: u32,
    #[serde(default)]
    pub root_offset: u64,  // File offset to root node (0 = in-memory only)
}

impl BPlusTree {
    /// Create new B+ tree index
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
                field,
                unique,
                sparse: false,
                num_keys: 0,
                tree_height: 1,
                root_offset: 0,
            },
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
                // In real implementation, would load child from disk
                // For now, simplified in-memory version
                None // TODO: implement child loading
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
            return Err(MongoLiteError::IndexError(
                format!("Duplicate key: {:?} (unique index)", key)
            ));
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
        let mut results = Vec::new();

        if let BTreeNode::Leaf(leaf) = &*self.root {
            for (i, key) in leaf.keys.iter().enumerate() {
                // Check start bound
                if *key < *start || (!inclusive_start && *key == *start) {
                    continue;
                }

                // Check end bound
                if *key > *end || (!inclusive_end && *key == *end) {
                    break;
                }

                results.push(leaf.document_ids[i].clone());
            }
        }

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
        let node_json = serde_json::to_string(node)
            .map_err(|e| MongoLiteError::Serialization(format!("Failed to serialize node: {}", e)))?;
        let node_bytes = node_json.as_bytes();

        // Ensure node fits in a page (4KB)
        if node_bytes.len() > NODE_PAGE_SIZE - 5 {
            return Err(MongoLiteError::IndexError(
                format!("Node size {} exceeds page size {}", node_bytes.len(), NODE_PAGE_SIZE - 5)
            ));
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
        page[5..(5 + node_bytes.len())].copy_from_slice(&node_bytes);

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
        let node_json = std::str::from_utf8(node_bytes)
            .map_err(|e| MongoLiteError::Serialization(format!("Invalid UTF-8 in node data: {}", e)))?;
        let node: BTreeNode = serde_json::from_str(node_json)
            .map_err(|e| MongoLiteError::Serialization(format!("Failed to deserialize node: {}", e)))?;

        // Verify node type matches
        match (&node, node_type) {
            (BTreeNode::Internal(_), NODE_TYPE_INTERNAL) => Ok(node),
            (BTreeNode::Leaf(_), NODE_TYPE_LEAF) => Ok(node),
            _ => Err(MongoLiteError::Corruption(
                format!("Node type mismatch at offset {}", offset)
            )),
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

        Ok(BPlusTree {
            root,
            metadata,
        })
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
            return Err(MongoLiteError::IndexError(
                format!("Duplicate key: {} (unique index)", key)
            ));
        }

        self.entries.entry(key)
            .or_insert_with(Vec::new)
            .push(doc_id);

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
}

impl IndexManager {
    pub fn new() -> Self {
        IndexManager {
            btree_indexes: HashMap::new(),
            legacy_indexes: HashMap::new(),
        }
    }

    /// Create B+ tree index
    pub fn create_btree_index(&mut self, name: String, field: String, unique: bool) -> Result<()> {
        if self.btree_indexes.contains_key(&name) {
            return Err(MongoLiteError::IndexError(
                format!("Index already exists: {}", name)
            ));
        }

        let tree = BPlusTree::new(name.clone(), field, unique);
        self.btree_indexes.insert(name, tree);
        Ok(())
    }

    /// Create legacy HashMap index
    pub fn create_index(&mut self, definition: IndexDefinition) -> Result<()> {
        let name = definition.name.clone();

        if self.legacy_indexes.contains_key(&name) {
            return Err(MongoLiteError::IndexError(
                format!("Index already exists: {}", name)
            ));
        }

        self.legacy_indexes.insert(name, Index::new(definition));
        Ok(())
    }

    /// Drop index by name
    pub fn drop_index(&mut self, name: &str) -> Result<()> {
        if self.btree_indexes.remove(name).is_none() && self.legacy_indexes.remove(name).is_none() {
            return Err(MongoLiteError::IndexError(
                format!("Index not found: {}", name)
            ));
        }
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
        let mut names: Vec<String> = self.btree_indexes.keys()
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

        tree.insert(IndexKey::String("test@example.com".to_string()), DocumentId::Int(1)).unwrap();

        let result = tree.insert(IndexKey::String("test@example.com".to_string()), DocumentId::Int(2));
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

        assert_eq!(results.len(), 10);  // 10..19
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
            tree.insert(IndexKey::Int(i * 10), DocumentId::Int(i)).unwrap();
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
        assert!(root_offset > 0 || root_offset == 0); // Valid offset
        assert_eq!(tree.metadata.root_offset, root_offset);

        // Load tree from file
        let metadata_clone = tree.metadata.clone();
        let loaded_tree = BPlusTree::load_from_file(&mut file, metadata_clone).unwrap();

        // Verify search still works
        assert_eq!(loaded_tree.search(&IndexKey::Int(0)), Some(DocumentId::Int(0)));
        assert_eq!(loaded_tree.search(&IndexKey::Int(50)), Some(DocumentId::Int(5)));
        assert_eq!(loaded_tree.search(&IndexKey::Int(90)), Some(DocumentId::Int(9)));
        assert_eq!(loaded_tree.search(&IndexKey::Int(99)), None);

        // Cleanup
        std::fs::remove_file(temp_path).ok();
    }
}
