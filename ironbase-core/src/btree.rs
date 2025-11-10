// src/btree.rs
// Full B+ Tree Implementation with proper split support

use crate::index::{IndexKey, IndexMetadata};
use crate::document::DocumentId;
use crate::error::{Result, MongoLiteError};
use serde::{Serialize, Deserialize};

// B+ Tree Configuration
const BTREE_ORDER: usize = 32;
const MAX_KEYS: usize = BTREE_ORDER - 1;  // 31

/// B+ Tree Node (in-memory, simplified)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Node {
    Internal {
        keys: Vec<IndexKey>,
        children: Vec<Box<Node>>,  // In-memory children
    },
    Leaf {
        keys: Vec<IndexKey>,
        values: Vec<DocumentId>,
    },
}

/// Split result when node overflows
#[derive(Debug)]
struct SplitResult {
    key: IndexKey,        // Key to push up
    right: Box<Node>,     // New right node
}

/// Full B+ Tree with complete split support
#[derive(Debug, Clone)]
pub struct BPlusTreeFull {
    root: Box<Node>,
    pub metadata: IndexMetadata,
}

impl BPlusTreeFull {
    /// Create new B+ tree
    pub fn new(name: String, field: String, unique: bool) -> Self {
        let root = Box::new(Node::Leaf {
            keys: Vec::new(),
            values: Vec::new(),
        });

        BPlusTreeFull {
            root,
            metadata: IndexMetadata {
                name,
                field,
                unique,
                sparse: false,
                num_keys: 0,
                tree_height: 1,
            },
        }
    }

    /// Search for a key
    pub fn search(&self, key: &IndexKey) -> Option<DocumentId> {
        Self::search_in_node(&self.root, key)
    }

    fn search_in_node(node: &Node, key: &IndexKey) -> Option<DocumentId> {
        match node {
            Node::Internal { keys, children } => {
                // Find child: keys[i] is separator
                // child[i] has keys < keys[i]
                // child[i+1] has keys >= keys[i]
                let idx = match keys.binary_search(key) {
                    Ok(pos) => pos + 1,  // Found exact match in separator -> go right
                    Err(pos) => pos,     // Not found -> pos is correct child
                };
                Self::search_in_node(&children[idx], key)
            }
            Node::Leaf { keys, values } => {
                keys.binary_search(key).ok().map(|idx| values[idx].clone())
            }
        }
    }

    /// Insert key-value pair with full split support
    pub fn insert(&mut self, key: IndexKey, doc_id: DocumentId) -> Result<()> {
        // Unique constraint check
        if self.metadata.unique && self.search(&key).is_some() {
            return Err(MongoLiteError::IndexError(
                format!("Duplicate key: {:?}", key)
            ));
        }

        // Take root ownership for mutation
        let old_root = std::mem::replace(
            &mut self.root,
            Box::new(Node::Leaf { keys: Vec::new(), values: Vec::new() })
        );

        // Insert and handle potential split
        match Self::insert_into_node(old_root, key, doc_id)? {
            (new_node, None) => {
                // No split, just update root
                self.root = new_node;
            }
            (left_node, Some(split)) => {
                // Root split - create new root
                self.root = Box::new(Node::Internal {
                    keys: vec![split.key],
                    children: vec![left_node, split.right],
                });
                self.metadata.tree_height += 1;
            }
        }

        self.metadata.num_keys += 1;
        Ok(())
    }

    /// Insert into node with split propagation
    fn insert_into_node(
        mut node: Box<Node>,
        key: IndexKey,
        value: DocumentId,
    ) -> Result<(Box<Node>, Option<SplitResult>)> {
        match *node {
            Node::Leaf { ref mut keys, ref mut values } => {
                // Find insert position
                let pos = keys.binary_search(&key).unwrap_or_else(|p| p);

                // Insert
                keys.insert(pos, key);
                values.insert(pos, value);

                // Check overflow
                if keys.len() <= MAX_KEYS {
                    return Ok((node, None));
                }

                // Split leaf
                let mid = keys.len() / 2;
                let right_keys = keys.split_off(mid);
                let right_values = values.split_off(mid);

                let split_key = right_keys[0].clone();

                let right_node = Box::new(Node::Leaf {
                    keys: right_keys,
                    values: right_values,
                });

                Ok((node, Some(SplitResult {
                    key: split_key,
                    right: right_node,
                })))
            }
            Node::Internal { ref mut keys, ref mut children } => {
                // Find child to insert into (same logic as search)
                let idx = match keys.binary_search(&key) {
                    Ok(pos) => pos + 1,  // Exact match -> go right
                    Err(pos) => pos,     // Not found -> pos is correct child
                };

                // Remove child, insert into it
                let child = children.remove(idx);
                let (new_child, split_opt) = Self::insert_into_node(child, key, value)?;

                // Put child back
                children.insert(idx, new_child);

                // Handle child split
                if let Some(split) = split_opt {
                    // Insert split key and new child
                    keys.insert(idx, split.key.clone());
                    children.insert(idx + 1, split.right);

                    // Check if internal node overflows
                    if keys.len() <= MAX_KEYS {
                        return Ok((node, None));
                    }

                    // Split internal node
                    let mid = keys.len() / 2;

                    // Split children first (mid+1 because we have n+1 children for n keys)
                    let right_children = children.split_off(mid + 1);

                    // Remove mid key (this goes up to parent)
                    let mid_key = keys.remove(mid);

                    // Split remaining keys
                    let right_keys = keys.split_off(mid);

                    let right_node = Box::new(Node::Internal {
                        keys: right_keys,
                        children: right_children,
                    });

                    return Ok((node, Some(SplitResult {
                        key: mid_key,
                        right: right_node,
                    })));
                }

                Ok((node, None))
            }
        }
    }

    /// Range scan
    pub fn range_scan(
        &self,
        start: &IndexKey,
        end: &IndexKey,
        inclusive_start: bool,
        inclusive_end: bool,
    ) -> Vec<DocumentId> {
        let mut results = Vec::new();
        Self::range_scan_node(&self.root, start, end, inclusive_start, inclusive_end, &mut results);
        results
    }

    fn range_scan_node(
        node: &Node,
        start: &IndexKey,
        end: &IndexKey,
        inclusive_start: bool,
        inclusive_end: bool,
        results: &mut Vec<DocumentId>,
    ) {
        match node {
            Node::Internal { keys, children } => {
                // Find starting child (same separator logic as search)
                let start_idx = match keys.binary_search(start) {
                    Ok(pos) => pos + 1,  // Start key equals separator -> start from right child
                    Err(pos) => pos,     // Start key between separators
                };

                // Scan all potentially relevant children
                for i in start_idx..children.len() {
                    // Check if we can stop early
                    // If we've passed the end key, no need to continue
                    if i > 0 && keys.get(i - 1).map(|k| k > end).unwrap_or(false) {
                        break;
                    }
                    Self::range_scan_node(&children[i], start, end, inclusive_start, inclusive_end, results);
                }
            }
            Node::Leaf { keys, values } => {
                for (i, key) in keys.iter().enumerate() {
                    if *key < *start || (!inclusive_start && *key == *start) {
                        continue;
                    }
                    if *key > *end || (!inclusive_end && *key == *end) {
                        break;
                    }
                    results.push(values[i].clone());
                }
            }
        }
    }

    /// Delete a key (lazy delete - no merge)
    pub fn delete(&mut self, key: &IndexKey) -> Result<bool> {
        let deleted = Self::delete_from_node(&mut self.root, key);
        if deleted {
            self.metadata.num_keys -= 1;
        }
        Ok(deleted)
    }

    fn delete_from_node(node: &mut Node, key: &IndexKey) -> bool {
        match node {
            Node::Leaf { keys, values } => {
                if let Ok(idx) = keys.binary_search(key) {
                    keys.remove(idx);
                    values.remove(idx);
                    true
                } else {
                    false
                }
            }
            Node::Internal { keys, children } => {
                let idx = keys.binary_search(key).unwrap_or_else(|p| p);
                Self::delete_from_node(&mut children[idx], key)
            }
        }
    }

    /// Get tree size
    pub fn size(&self) -> u64 {
        self.metadata.num_keys
    }

    /// Get tree height
    pub fn height(&self) -> u32 {
        self.metadata.tree_height
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_btree_basic_insert_search() {
        let mut tree = BPlusTreeFull::new("test".to_string(), "age".to_string(), false);

        tree.insert(IndexKey::Int(10), DocumentId::Int(100)).unwrap();
        tree.insert(IndexKey::Int(20), DocumentId::Int(200)).unwrap();
        tree.insert(IndexKey::Int(5), DocumentId::Int(50)).unwrap();

        assert_eq!(tree.search(&IndexKey::Int(10)), Some(DocumentId::Int(100)));
        assert_eq!(tree.search(&IndexKey::Int(20)), Some(DocumentId::Int(200)));
        assert_eq!(tree.search(&IndexKey::Int(5)), Some(DocumentId::Int(50)));
        assert_eq!(tree.search(&IndexKey::Int(99)), None);
    }

    #[test]
    fn test_btree_split() {
        let mut tree = BPlusTreeFull::new("test".to_string(), "age".to_string(), false);

        // Insert enough to force splits (MAX_KEYS = 31, so 32 will force first split)
        for i in 0..100 {
            tree.insert(IndexKey::Int(i), DocumentId::Int(i)).unwrap();
        }

        // All keys should still be searchable
        for i in 0..100 {
            assert_eq!(tree.search(&IndexKey::Int(i)), Some(DocumentId::Int(i)),
                "Failed to find key {}", i);
        }

        assert_eq!(tree.size(), 100);
        // Tree height should increase with splits
        assert!(tree.height() > 1, "Tree should have split and increased height");
    }

    #[test]
    fn test_btree_unique_constraint() {
        let mut tree = BPlusTreeFull::new("test".to_string(), "email".to_string(), true);

        tree.insert(IndexKey::String("test@example.com".to_string()), DocumentId::Int(1)).unwrap();

        let result = tree.insert(IndexKey::String("test@example.com".to_string()), DocumentId::Int(2));
        assert!(result.is_err());
    }

    #[test]
    fn test_btree_range_scan() {
        let mut tree = BPlusTreeFull::new("test".to_string(), "age".to_string(), false);

        for i in 0..100 {
            tree.insert(IndexKey::Int(i), DocumentId::Int(i)).unwrap();
        }

        let results = tree.range_scan(
            &IndexKey::Int(10),
            &IndexKey::Int(20),
            true,
            false,
        );

        assert_eq!(results.len(), 10);
        assert_eq!(results[0], DocumentId::Int(10));
        assert_eq!(results[9], DocumentId::Int(19));
    }

    #[test]
    fn test_btree_delete() {
        let mut tree = BPlusTreeFull::new("test".to_string(), "age".to_string(), false);

        tree.insert(IndexKey::Int(10), DocumentId::Int(100)).unwrap();
        tree.insert(IndexKey::Int(20), DocumentId::Int(200)).unwrap();
        tree.insert(IndexKey::Int(30), DocumentId::Int(300)).unwrap();

        assert_eq!(tree.size(), 3);

        let deleted = tree.delete(&IndexKey::Int(20)).unwrap();
        assert!(deleted);
        assert_eq!(tree.size(), 2);
        assert_eq!(tree.search(&IndexKey::Int(20)), None);
    }

    #[test]
    fn test_btree_large_insert() {
        let mut tree = BPlusTreeFull::new("test".to_string(), "id".to_string(), false);

        // Insert 1000 keys
        for i in 0..1000 {
            tree.insert(IndexKey::Int(i), DocumentId::Int(i)).unwrap();
        }

        assert_eq!(tree.size(), 1000);

        // Verify all are searchable
        for i in 0..1000 {
            assert_eq!(tree.search(&IndexKey::Int(i)), Some(DocumentId::Int(i)));
        }

        println!("Tree height for 1000 keys: {}", tree.height());
    }

    #[test]
    fn test_btree_random_order() {
        let mut tree = BPlusTreeFull::new("test".to_string(), "random".to_string(), false);

        // Insert in non-sequential order
        let keys = vec![50, 25, 75, 10, 30, 60, 90, 5, 15, 20];
        for &k in &keys {
            tree.insert(IndexKey::Int(k), DocumentId::Int(k as i64)).unwrap();
        }

        // All should be searchable
        for &k in &keys {
            assert_eq!(tree.search(&IndexKey::Int(k)), Some(DocumentId::Int(k as i64)));
        }
    }

    #[test]
    #[ignore]  // Slow test - run with: cargo test -- --ignored
    fn test_btree_performance_1m_keys() {
        use std::time::Instant;

        let mut tree = BPlusTreeFull::new("perf".to_string(), "id".to_string(), false);

        // Insert 1M keys
        let start = Instant::now();
        for i in 0..1_000_000 {
            tree.insert(IndexKey::Int(i), DocumentId::Int(i)).unwrap();
        }
        let insert_duration = start.elapsed();

        println!("1M inserts took: {:?} ({:.2} ops/sec)",
            insert_duration,
            1_000_000.0 / insert_duration.as_secs_f64()
        );
        println!("Tree height: {}", tree.height());
        println!("Tree size: {}", tree.size());

        // Random search test (1000 searches)
        let start = Instant::now();
        for i in (0..1_000_000).step_by(1000) {
            assert_eq!(tree.search(&IndexKey::Int(i)), Some(DocumentId::Int(i)));
        }
        let search_duration = start.elapsed();

        println!("1000 searches took: {:?} ({:.2} µs/search)",
            search_duration,
            search_duration.as_micros() as f64 / 1000.0
        );

        // Range scan test
        let start = Instant::now();
        let results = tree.range_scan(
            &IndexKey::Int(500_000),
            &IndexKey::Int(500_100),
            true,
            false,
        );
        let range_duration = start.elapsed();

        assert_eq!(results.len(), 100);
        println!("Range scan (100 keys) took: {:?}", range_duration);

        // Verify final size
        assert_eq!(tree.size(), 1_000_000);
    }
}
