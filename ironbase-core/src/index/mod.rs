//! Index Module - B+ Tree, Fuzzy, and Full-Text Search Indexes
//!
//! This module provides all indexing functionality for IronBase, enabling
//! fast lookups, range queries, similarity search, and full-text search.
//!
//! # Index Types
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │                        IndexManager                                  │
//! │  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐  ┌───────────┐  │
//! │  │  BPlusTree  │  │ FuzzyIndex  │  │FulltextIdx │  │  Legacy   │  │
//! │  │  (.idx)     │  │  (.fzidx)   │  │  (.ftidx)  │  │ (HashMap) │  │
//! │  └─────────────┘  └─────────────┘  └─────────────┘  └───────────┘  │
//! └─────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! | Type | Query Support | File | Use Case |
//! |------|---------------|------|----------|
//! | [`BPlusTree`] | $eq, $gt, $lt, $gte, $lte, $in | `.idx` | Primary/foreign keys, sorted fields |
//! | [`FuzzyIndex`] | $fuzzy (similarity) | `.fzidx` | Name matching, typo tolerance |
//! | `FulltextIndex` | TF-IDF text search | `.ftidx` | Document content, articles |
//! | [`Index`] | $eq only | N/A | Legacy, deprecated |
//!
//! # Key Ordering
//!
//! [`IndexKey`] implements a total ordering compatible with MongoDB:
//!
//! ```text
//! Null < Bool(false) < Bool(true) < Int < Float < String
//! ```
//!
//! This enables correct range scans across mixed types.
//!
//! # Compound Indexes
//!
//! B+ trees support multi-field compound indexes:
//!
//! ```rust,ignore
//! // Index on (country, city, zipcode)
//! collection.create_compound_index(
//!     vec!["country".to_string(), "city".to_string(), "zipcode".to_string()],
//!     false
//! )?;
//!
//! // Query uses index prefix: {country: "US", city: "NYC"}
//! // Query uses full index: {country: "US", city: "NYC", zipcode: "10001"}
//! // Query CANNOT use index: {city: "NYC"} (missing prefix)
//! ```
//!
//! # Submodules
//!
//! - [`btree`] - B+ tree implementation with persistence
//! - [`fuzzy`] - Jaro-Winkler, Levenshtein, Damerau-Levenshtein similarity
//! - [`key`] - IndexKey type with ordering and serialization
//! - [`manager`] - Per-collection index lifecycle management
//! - [`legacy`] - Deprecated HashMap-based indexes
//! - [`traits`] - Common index traits (IndexTrait, LazyLoadable)

pub mod btree;
pub mod fuzzy;
pub mod key;
pub mod legacy;
pub mod manager;
pub mod traits;

// Re-export main types for convenience
pub use btree::{
    BPlusTree, IndexMetadata, IndexStats, RangeQueryMode, RangeQueryResult, ScanOrder,
    NODE_PAGE_SIZE,
};
pub use fuzzy::{FuzzyAlgorithm, FuzzyIndex, FuzzyIndexMetadata};
pub use key::{IndexKey, IndexPrefixInfo, OrderedFloat};
pub use legacy::{Index, IndexDefinition, IndexType};
pub use manager::IndexManager;
pub use traits::{IndexTrait, LazyLoadable};

// Constants for tests
#[cfg(test)]
const MAX_KEYS_PER_NODE: usize = 128;

#[cfg(test)]
mod tests {
    use super::btree::{BTreeNode, LeafNode};
    use super::*;
    use crate::document::DocumentId;
    use serde_json::json;
    use std::collections::HashSet;

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
        let mut tree = BPlusTree::new("test_idx".to_string(), "age".to_string(), false, false);

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
        let mut tree = BPlusTree::new("email_idx".to_string(), "email".to_string(), true, false);

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
    fn test_multikey_extract_keys_array_field() {
        let tree = BPlusTree::new("tags_idx".to_string(), "tags".to_string(), false, false);
        let doc = json!({"tags": ["urgent", "important"]});

        let keys: HashSet<IndexKey> = tree.extract_keys(&doc).into_iter().collect();
        let expected: HashSet<IndexKey> = [
            IndexKey::String("urgent".to_string()),
            IndexKey::String("important".to_string()),
        ]
        .into_iter()
        .collect();

        assert_eq!(keys, expected);
    }

    #[test]
    fn test_compound_multikey_single_array_field() {
        let tree = BPlusTree::new_compound(
            "country_tags_idx".to_string(),
            vec!["country".to_string(), "tags".to_string()],
            false,
            false,
        );
        let doc = json!({"country": "US", "tags": ["a", "b"]});

        let keys: HashSet<IndexKey> = tree.extract_keys(&doc).into_iter().collect();
        let expected: HashSet<IndexKey> = [
            IndexKey::Compound(vec![
                IndexKey::String("US".to_string()),
                IndexKey::String("a".to_string()),
            ]),
            IndexKey::Compound(vec![
                IndexKey::String("US".to_string()),
                IndexKey::String("b".to_string()),
            ]),
        ]
        .into_iter()
        .collect();

        assert_eq!(keys, expected);
    }

    #[test]
    fn test_compound_multikey_multiple_arrays_rejected() {
        let mut manager = IndexManager::new();
        manager
            .create_compound_index(
                "country_tags_idx".to_string(),
                vec!["country".to_string(), "tags".to_string()],
                false,
                false,
            )
            .unwrap();

        let doc = json!({"country": ["US", "CA"], "tags": ["a", "b"]});
        let result = manager.add_document_to_indexes(&doc, &DocumentId::Int(1), None);

        assert!(result.is_err());
    }

    #[test]
    fn test_btree_range_query() {
        let mut tree = BPlusTree::new("age_idx".to_string(), "age".to_string(), false, false);

        for i in 0..100 {
            tree.insert(IndexKey::Int(i), DocumentId::Int(i)).unwrap();
        }

        let mode = RangeQueryMode::Scan {
            skip: 0,
            limit: None,
            order: ScanOrder::Asc,
        };
        let results = tree
            .range_query(&IndexKey::Int(10), &IndexKey::Int(20), true, false, mode)
            .unwrap_docs();

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
        let mut tree = BPlusTree::new("test_idx".to_string(), "age".to_string(), false, false);

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
    fn test_compound_index_range_query() {
        let mut tree = BPlusTree::new_compound(
            "users_location".to_string(),
            vec!["country".to_string(), "city".to_string()],
            false,
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

        // Range query for all US cities
        let start = IndexKey::Compound(vec![
            IndexKey::String("US".to_string()),
            IndexKey::String("".to_string()), // Empty string sorts before any city
        ]);
        let end = IndexKey::Compound(vec![
            IndexKey::String("US".to_string()),
            IndexKey::String("\u{10ffff}".to_string()), // Max unicode sorts after any city
        ]);

        let mode = RangeQueryMode::Scan {
            skip: 0,
            limit: None,
            order: ScanOrder::Asc,
        };
        let results = tree
            .range_query(&start, &end, true, true, mode)
            .unwrap_docs();
        assert_eq!(results.len(), 3); // Chicago, LA, NYC
    }

    #[test]
    fn test_compound_index_unique() {
        let mut tree = BPlusTree::new_compound(
            "users_location".to_string(),
            vec!["country".to_string(), "city".to_string()],
            true,  // unique
            false, // sparse
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
            false,
        );
        assert!(result.is_err());
    }

    /// Regression test for BUG #3: apply_batch_updates was causing duplicate entries
    /// because build_from_sorted was adding to the existing tree instead of replacing it.
    /// Fix: Added clear() call before build_from_sorted in apply_batch_updates.
    #[test]
    fn test_apply_batch_updates_no_duplicates() {
        let mut tree = BPlusTree::new("test_idx".to_string(), "name".to_string(), false, false);

        // Insert 3 entries
        tree.insert(
            IndexKey::String("alice".to_string()),
            DocumentId::String("doc1".to_string()),
        )
        .unwrap();
        tree.insert(
            IndexKey::String("bob".to_string()),
            DocumentId::String("doc2".to_string()),
        )
        .unwrap();
        tree.insert(
            IndexKey::String("carol".to_string()),
            DocumentId::String("doc3".to_string()),
        )
        .unwrap();

        assert_eq!(tree.metadata.num_keys, 3);

        // Apply batch updates (simulating update_many that doesn't change keys)
        let updates = vec![
            (
                IndexKey::String("alice".to_string()),
                DocumentId::String("doc1".to_string()),
                IndexKey::String("alice".to_string()),
                DocumentId::String("doc1".to_string()),
            ),
            (
                IndexKey::String("bob".to_string()),
                DocumentId::String("doc2".to_string()),
                IndexKey::String("bob".to_string()),
                DocumentId::String("doc2".to_string()),
            ),
            (
                IndexKey::String("carol".to_string()),
                DocumentId::String("doc3".to_string()),
                IndexKey::String("carol".to_string()),
                DocumentId::String("doc3".to_string()),
            ),
        ];

        tree.apply_batch_updates(updates).unwrap();

        // CRITICAL: After batch update, should still have exactly 3 entries, not 6!
        assert_eq!(
            tree.metadata.num_keys, 3,
            "BUG #3 regression: apply_batch_updates created duplicates!"
        );

        // Verify each key has exactly one entry
        let mode = RangeQueryMode::Scan {
            skip: 0,
            limit: None,
            order: ScanOrder::Asc,
        };
        let alice_key = IndexKey::String("alice".to_string());
        let alice_results = tree
            .range_query(&alice_key, &alice_key, true, true, mode.clone())
            .unwrap_docs();
        assert_eq!(
            alice_results.len(),
            1,
            "BUG #3 regression: 'alice' has duplicates!"
        );

        let bob_key = IndexKey::String("bob".to_string());
        let bob_results = tree
            .range_query(&bob_key, &bob_key, true, true, mode)
            .unwrap_docs();
        assert_eq!(
            bob_results.len(),
            1,
            "BUG #3 regression: 'bob' has duplicates!"
        );
    }

    /// Regression test for BUG #3: Verify clear() properly resets the tree
    #[test]
    fn test_clear_resets_tree() {
        let mut tree = BPlusTree::new("test_idx".to_string(), "value".to_string(), false, false);

        // Insert some entries
        for i in 0..10 {
            tree.insert(IndexKey::Int(i), DocumentId::Int(i)).unwrap();
        }

        assert_eq!(tree.metadata.num_keys, 10);
        assert!(tree.metadata.tree_height >= 1);

        // Clear the tree
        tree.clear();

        // Verify reset state
        assert_eq!(
            tree.metadata.num_keys, 0,
            "num_keys should be 0 after clear"
        );
        assert_eq!(
            tree.metadata.tree_height, 1,
            "tree_height should be 1 after clear"
        );

        // Verify tree is empty
        let all_entries = tree.get_all_entries();
        assert!(
            all_entries.is_empty(),
            "Tree should have no entries after clear"
        );

        // Verify we can insert again
        tree.insert(IndexKey::Int(100), DocumentId::Int(100))
            .unwrap();
        assert_eq!(tree.metadata.num_keys, 1);
    }

    #[test]
    fn test_refresh_stats_single_field() {
        let mut tree = BPlusTree::new("idx".into(), "field".into(), false, false);
        tree.insert(IndexKey::String("a".into()), DocumentId::Int(1))
            .unwrap();
        tree.insert(IndexKey::String("b".into()), DocumentId::Int(2))
            .unwrap();
        tree.insert(IndexKey::String("a".into()), DocumentId::Int(3))
            .unwrap(); // duplicate key

        tree.refresh_stats();

        assert_eq!(tree.metadata.stats.distinct_count, 2); // "a" and "b"
        assert_eq!(tree.metadata.stats.null_count, 0);
        assert!(tree.metadata.stats.last_analyzed > 0);
        assert!((tree.metadata.stats.sample_rate - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_refresh_stats_with_nulls() {
        let mut tree = BPlusTree::new("idx".into(), "field".into(), true, false); // unique
        tree.insert(IndexKey::String("a".into()), DocumentId::Int(1))
            .unwrap();
        tree.insert(IndexKey::Null, DocumentId::Int(2)).unwrap();

        tree.refresh_stats();

        assert_eq!(tree.metadata.stats.distinct_count, 2); // "a" and Null
        assert_eq!(tree.metadata.stats.null_count, 1);
    }

    #[test]
    fn test_refresh_stats_empty_index() {
        let mut tree = BPlusTree::new("idx".into(), "field".into(), false, false);
        tree.refresh_stats();

        assert_eq!(tree.metadata.stats.distinct_count, 0);
        assert_eq!(tree.metadata.stats.null_count, 0);
    }

    #[test]
    fn test_refresh_stats_many_duplicates() {
        let mut tree = BPlusTree::new("idx".into(), "status".into(), false, false);
        // Insert 100 docs with only 3 distinct values
        for i in 0..100 {
            let status = match i % 3 {
                0 => "active",
                1 => "pending",
                _ => "closed",
            };
            tree.insert(IndexKey::String(status.into()), DocumentId::Int(i))
                .unwrap();
        }

        tree.refresh_stats();

        assert_eq!(tree.metadata.stats.distinct_count, 3);
        assert_eq!(tree.metadata.stats.null_count, 0);
    }

    #[test]
    fn test_stats_validate_and_fix() {
        use crate::index::btree::IndexStats;

        // Test 1: Valid stats - no changes
        let mut stats = IndexStats {
            distinct_count: 50,
            null_count: 10,
            multikey_ratio: 0.5,
            sample_rate: 1.0,
            last_analyzed: 0,
        };
        assert!(!stats.validate_and_fix(100)); // No fixes needed
        assert_eq!(stats.distinct_count, 50);
        assert_eq!(stats.null_count, 10);

        // Test 2: distinct_count > num_keys - should be capped
        let mut stats = IndexStats {
            distinct_count: 200,
            null_count: 10,
            multikey_ratio: 0.5,
            sample_rate: 1.0,
            last_analyzed: 0,
        };
        assert!(stats.validate_and_fix(100)); // Fixed!
        assert_eq!(stats.distinct_count, 100);

        // Test 3: null_count > num_keys - should be capped
        let mut stats = IndexStats {
            distinct_count: 50,
            null_count: 150,
            multikey_ratio: 0.5,
            sample_rate: 1.0,
            last_analyzed: 0,
        };
        assert!(stats.validate_and_fix(100)); // Fixed!
        assert_eq!(stats.null_count, 100);

        // Test 4: Invalid multikey_ratio - should be reset
        let mut stats = IndexStats {
            distinct_count: 50,
            null_count: 10,
            multikey_ratio: 1.5, // Invalid!
            sample_rate: 1.0,
            last_analyzed: 0,
        };
        assert!(stats.validate_and_fix(100)); // Fixed!
        assert_eq!(stats.multikey_ratio, 0.0);

        // Test 5: NaN values - should be reset
        let mut stats = IndexStats {
            distinct_count: 50,
            null_count: 10,
            multikey_ratio: f32::NAN,
            sample_rate: f32::NAN,
            last_analyzed: 0,
        };
        assert!(stats.validate_and_fix(100)); // Fixed!
        assert_eq!(stats.multikey_ratio, 0.0);
        assert_eq!(stats.sample_rate, 0.0);
    }
}

#[cfg(test)]
mod debug_tests {
    use super::*;
    use crate::document::DocumentId;
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
                false,
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
    use crate::document::DocumentId;
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
    use crate::document::DocumentId;

    /// Test that inserting more than MAX_KEYS_PER_NODE triggers a split
    /// and increases tree height
    #[test]
    fn test_btree_insert_triggers_split() {
        let mut tree = BPlusTree::new("test_idx".to_string(), "field".to_string(), false, false);

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
        let mut tree = BPlusTree::new("large_idx".to_string(), "id".to_string(), false, false);

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

        // Verify range query works on multi-level tree
        let mode = RangeQueryMode::Scan {
            skip: 0,
            limit: None,
            order: ScanOrder::Asc,
        };
        let results = tree
            .range_query(&IndexKey::Int(100), &IndexKey::Int(200), true, false, mode)
            .unwrap_docs();
        assert_eq!(results.len(), 100, "Range query should return 100 elements");
    }

    /// Test that multiple splits create correct tree structure
    #[test]
    fn test_btree_multiple_splits() {
        let mut tree = BPlusTree::new("multi_split_idx".to_string(), "x".to_string(), false, false);

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
        let mut tree = BPlusTree::new("persist_idx".to_string(), "id".to_string(), false, false);

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
    use crate::document::DocumentId;

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

    /// Test compound index prefix query via range_query
    #[test]
    fn test_compound_prefix_query_range_query() {
        let mut tree = BPlusTree::new_compound(
            "users_country_city".to_string(),
            vec!["country".to_string(), "city".to_string()],
            false,
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

        let mode = RangeQueryMode::Scan {
            skip: 0,
            limit: None,
            order: ScanOrder::Asc,
        };

        // Query: prefix = "US" (should find LA, NYC, SF)
        let prefix = IndexKey::String("US".to_string());
        let (start, end) = tree.build_prefix_range(prefix);
        let results = tree
            .range_query(&start, &end, true, true, mode.clone())
            .unwrap_docs();

        assert_eq!(results.len(), 3, "Should find 3 US cities");
        assert!(results.contains(&DocumentId::Int(3)));
        assert!(results.contains(&DocumentId::Int(4)));
        assert!(results.contains(&DocumentId::Int(5)));

        // Query: prefix = "HU" (should find Budapest, Debrecen)
        let prefix = IndexKey::String("HU".to_string());
        let (start, end) = tree.build_prefix_range(prefix);
        let results = tree
            .range_query(&start, &end, true, true, mode.clone())
            .unwrap_docs();

        assert_eq!(results.len(), 2, "Should find 2 HU cities");
        assert!(results.contains(&DocumentId::Int(1)));
        assert!(results.contains(&DocumentId::Int(2)));

        // Query: prefix = "DE" (should find nothing)
        let prefix = IndexKey::String("DE".to_string());
        let (start, end) = tree.build_prefix_range(prefix);
        let results = tree
            .range_query(&start, &end, true, true, mode)
            .unwrap_docs();

        assert_eq!(results.len(), 0, "Should find no DE cities");
    }

    /// Test IndexPrefixInfo for compound indexes
    #[test]
    fn test_index_prefix_info_compound() {
        let mut manager = IndexManager::new();

        // Create single-field index
        manager
            .create_btree_index("users_age".to_string(), "age".to_string(), false, false)
            .unwrap();
        // Mark index as ready (normally done by collection after population)
        manager.set_index_ready("users_age").unwrap();

        // Create compound index
        manager
            .create_compound_index(
                "users_country_city".to_string(),
                vec!["country".to_string(), "city".to_string()],
                false,
                false,
            )
            .unwrap();
        // Mark index as ready (normally done by collection after population)
        manager.set_index_ready("users_country_city").unwrap();

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

#[cfg(test)]
mod non_unique_delete_tests {
    use super::*;
    use crate::document::DocumentId;

    /// Test that delete works correctly for non-unique indexes with duplicate keys.
    ///
    /// BUG FIX TEST: Previously, delete used binary_search which only returns
    /// ONE position for a given key. In non-unique indexes, multiple documents
    /// can share the same key, so we must scan all matching entries to find
    /// the correct doc_id to delete.
    #[test]
    fn test_non_unique_index_delete_with_duplicates() {
        let mut tree = BPlusTree::new("email_idx".to_string(), "email".to_string(), false, false);

        // Insert 5 documents with the SAME email (non-unique index allows this)
        let email_key = IndexKey::String("alice@test.com".to_string());
        tree.insert(email_key.clone(), DocumentId::Int(100))
            .unwrap();
        tree.insert(email_key.clone(), DocumentId::Int(200))
            .unwrap();
        tree.insert(email_key.clone(), DocumentId::Int(300))
            .unwrap();
        tree.insert(email_key.clone(), DocumentId::Int(400))
            .unwrap();
        tree.insert(email_key.clone(), DocumentId::Int(500))
            .unwrap();

        assert_eq!(tree.metadata.num_keys, 5);

        // Delete doc 300 (middle document) - this was the bug case
        tree.delete(&email_key, &DocumentId::Int(300)).unwrap();
        assert_eq!(tree.metadata.num_keys, 4);

        // Delete doc 100 (first document)
        tree.delete(&email_key, &DocumentId::Int(100)).unwrap();
        assert_eq!(tree.metadata.num_keys, 3);

        // Delete doc 500 (last document)
        tree.delete(&email_key, &DocumentId::Int(500)).unwrap();
        assert_eq!(tree.metadata.num_keys, 2);

        // Verify remaining documents are still in the index via range_query
        let mode = RangeQueryMode::Scan {
            skip: 0,
            limit: None,
            order: ScanOrder::Asc,
        };
        let remaining = tree
            .range_query(&email_key, &email_key, true, true, mode)
            .unwrap_docs();
        assert_eq!(remaining.len(), 2);
        assert!(remaining.contains(&DocumentId::Int(200)));
        assert!(remaining.contains(&DocumentId::Int(400)));
    }

    /// Test that range_query returns all documents with the same key (no duplicates issue)
    #[test]
    fn test_non_unique_range_query_no_duplicates() {
        let mut tree = BPlusTree::new("status_idx".to_string(), "status".to_string(), false, false);

        let active_key = IndexKey::String("active".to_string());
        let inactive_key = IndexKey::String("inactive".to_string());

        // Insert 3 active documents
        tree.insert(active_key.clone(), DocumentId::Int(1)).unwrap();
        tree.insert(active_key.clone(), DocumentId::Int(2)).unwrap();
        tree.insert(active_key.clone(), DocumentId::Int(3)).unwrap();

        // Insert 2 inactive documents
        tree.insert(inactive_key.clone(), DocumentId::Int(10))
            .unwrap();
        tree.insert(inactive_key.clone(), DocumentId::Int(20))
            .unwrap();

        // Range query for "active" should return exactly 3 unique doc_ids
        let mode = RangeQueryMode::Scan {
            skip: 0,
            limit: None,
            order: ScanOrder::Asc,
        };
        let active_results = tree
            .range_query(&active_key, &active_key, true, true, mode)
            .unwrap_docs();
        assert_eq!(active_results.len(), 3);

        // Verify NO duplicates in results
        let mut seen = std::collections::HashSet::new();
        for doc_id in &active_results {
            assert!(
                seen.insert(doc_id.clone()),
                "Duplicate doc_id found: {:?}",
                doc_id
            );
        }
    }
}
