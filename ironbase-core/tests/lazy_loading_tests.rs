//! Integration tests for LazyLoadable trait implementations
//!
//! Tests verify that:
//! - `is_lazy_mode()` correctly reports lazy state
//! - `ensure_fully_loaded()` is idempotent
//! - `hot_ratio()` returns correct values
//! - Lazy-loaded indexes work correctly after loading
//!
//! Note: Actual threshold-based lazy loading requires indexes > 50-500MB
//! (depending on system RAM). These tests verify the API behavior.

use ironbase_core::document::DocumentId;
use ironbase_core::index::btree::{BPlusTree, IndexMetadata, IndexStats};
use ironbase_core::index::fuzzy::{FuzzyAlgorithm, FuzzyIndex};
use ironbase_core::index::key::IndexKey;
use ironbase_core::index::traits::{calculate_lazy_threshold, IndexTrait, LazyLoadable};
use std::fs::File;
use tempfile::TempDir;

/// Helper to create index metadata
fn test_metadata(name: &str, field: &str) -> IndexMetadata {
    IndexMetadata {
        name: name.to_string(),
        field: field.to_string(),
        fields: vec![field.to_string()],
        unique: false,
        sparse: false,
        multikey: false,
        case_insensitive: false,
        num_keys: 0,
        tree_height: 0,
        root_offset: 0,
        stats: IndexStats::default(),
        building: false,
    }
}

// ============================================================================
// calculate_lazy_threshold() tests
// ============================================================================

#[test]
fn test_lazy_threshold_is_within_bounds() {
    let threshold = calculate_lazy_threshold();

    // Must be between 50MB and 500MB
    assert!(
        threshold >= 50 * 1024 * 1024,
        "Threshold {} should be >= 50MB",
        threshold
    );
    assert!(
        threshold <= 500 * 1024 * 1024,
        "Threshold {} should be <= 500MB",
        threshold
    );
}

#[test]
fn test_lazy_threshold_is_deterministic() {
    let t1 = calculate_lazy_threshold();
    let t2 = calculate_lazy_threshold();
    let t3 = calculate_lazy_threshold();

    assert_eq!(t1, t2);
    assert_eq!(t2, t3);
}

// ============================================================================
// BPlusTree LazyLoadable tests
// ============================================================================

#[test]
fn test_btree_new_is_not_lazy() {
    let btree = BPlusTree::new("field_idx".to_string(), "field".to_string(), false, false);

    // New in-memory index should never be in lazy mode
    assert!(!btree.is_lazy_mode());
    assert_eq!(btree.hot_ratio(), 1.0);
}

#[test]
fn test_btree_ensure_fully_loaded_is_idempotent() {
    let mut btree = BPlusTree::new("field_idx".to_string(), "field".to_string(), false, false);

    // Insert some data
    btree
        .insert(IndexKey::String("value1".to_string()), DocumentId::Int(1))
        .unwrap();
    btree
        .insert(IndexKey::String("value2".to_string()), DocumentId::Int(2))
        .unwrap();

    // Multiple calls should succeed without error
    btree.ensure_fully_loaded().unwrap();
    btree.ensure_fully_loaded().unwrap();
    btree.ensure_fully_loaded().unwrap();

    // Data should still be accessible
    let result = btree.search(&IndexKey::String("value1".to_string()));
    assert_eq!(result, Some(DocumentId::Int(1)));
}

#[test]
fn test_btree_save_and_reload_small_file_not_lazy() {
    let temp_dir = TempDir::new().unwrap();
    let idx_path = temp_dir.path().join("test.idx");

    // Create and populate B+Tree
    let mut btree = BPlusTree::new("name_idx".to_string(), "name".to_string(), false, false);
    for i in 0..100 {
        btree
            .insert(
                IndexKey::String(format!("user_{}", i)),
                DocumentId::Int(i as i64),
            )
            .unwrap();
    }

    // Save to file
    {
        let mut file = File::create(&idx_path).unwrap();
        btree.save_to_file(&mut file).unwrap();
    }

    // Reload - small file should NOT trigger lazy mode
    let reloaded = BPlusTree::load_from_path(idx_path, test_metadata("name_idx", "name")).unwrap();

    // Small file should be fully loaded
    assert!(
        !reloaded.is_lazy_mode(),
        "Small index should not use lazy loading"
    );
    assert_eq!(reloaded.hot_ratio(), 1.0);

    // Data should be accessible immediately
    let result = reloaded.search(&IndexKey::String("user_50".to_string()));
    assert_eq!(result, Some(DocumentId::Int(50)));
}

#[test]
fn test_btree_persisted_size_after_save() {
    let temp_dir = TempDir::new().unwrap();
    let idx_path = temp_dir.path().join("test.idx");

    let mut btree = BPlusTree::new("field_idx".to_string(), "field".to_string(), false, false);
    for i in 0..50 {
        btree.insert(IndexKey::Int(i), DocumentId::Int(i)).unwrap();
    }

    // Save to file
    {
        let mut file = File::create(&idx_path).unwrap();
        btree.save_to_file(&mut file).unwrap();
    }

    // Reload and check persisted size
    let reloaded =
        BPlusTree::load_from_path(idx_path.clone(), test_metadata("field_idx", "field")).unwrap();

    let persisted_size = reloaded.persisted_size_bytes();
    assert!(persisted_size.is_some(), "Should report persisted size");

    let file_size = std::fs::metadata(&idx_path).unwrap().len();
    assert_eq!(
        persisted_size.unwrap(),
        file_size,
        "Persisted size should match file size"
    );
}

// ============================================================================
// FuzzyIndex LazyLoadable tests
// ============================================================================

#[test]
fn test_fuzzy_new_is_not_lazy() {
    let fuzzy = FuzzyIndex::new("name_fuzzy", "name", FuzzyAlgorithm::JaroWinkler, 0.8);

    assert!(!fuzzy.is_lazy_mode());
    assert_eq!(fuzzy.hot_ratio(), 1.0);
}

#[test]
fn test_fuzzy_ensure_fully_loaded_is_idempotent() {
    let mut fuzzy = FuzzyIndex::new("name_fuzzy", "name", FuzzyAlgorithm::JaroWinkler, 0.8);

    // Insert some data
    fuzzy.insert("Alice", DocumentId::Int(1));
    fuzzy.insert("Bob", DocumentId::Int(2));
    fuzzy.insert("Charlie", DocumentId::Int(3));

    // Multiple calls should succeed
    fuzzy.ensure_fully_loaded().unwrap();
    fuzzy.ensure_fully_loaded().unwrap();
    fuzzy.ensure_fully_loaded().unwrap();

    // Data should still be accessible
    let results = fuzzy.search("Alice", Some(0.8));
    assert!(!results.is_empty());
}

#[test]
fn test_fuzzy_save_and_reload_small_file() {
    let temp_dir = TempDir::new().unwrap();
    let idx_path = temp_dir.path().join("test.fzidx");

    // Create and populate FuzzyIndex
    let mut fuzzy = FuzzyIndex::new("name_fuzzy", "name", FuzzyAlgorithm::JaroWinkler, 0.8);
    fuzzy.set_storage_path(idx_path.clone());

    for i in 0..100 {
        fuzzy.insert(&format!("User_{}", i), DocumentId::Int(i as i64));
    }

    // Save to file
    fuzzy.flush().unwrap();

    // Reload - small file should NOT be lazy
    let reloaded = FuzzyIndex::load_from_file(&idx_path).unwrap();

    // Verify data is accessible
    let results = reloaded.search("User_50", Some(0.9));
    assert!(!results.is_empty(), "Should find User_50");
}

// ============================================================================
// Hot ratio tests
// ============================================================================

#[test]
fn test_hot_ratio_fully_loaded_is_one() {
    let btree = BPlusTree::new("field_idx".to_string(), "field".to_string(), false, false);
    let fuzzy = FuzzyIndex::new("field_fuzzy", "field", FuzzyAlgorithm::JaroWinkler, 0.8);

    // Fully loaded indexes should have hot_ratio = 1.0
    assert_eq!(btree.hot_ratio(), 1.0);
    assert_eq!(fuzzy.hot_ratio(), 1.0);
}

// ============================================================================
// IndexTrait common tests
// ============================================================================

#[test]
fn test_btree_index_trait_methods() {
    let mut btree = BPlusTree::new("age_idx".to_string(), "age".to_string(), false, false);

    btree.insert(IndexKey::Int(25), DocumentId::Int(1)).unwrap();
    btree.insert(IndexKey::Int(30), DocumentId::Int(2)).unwrap();

    assert_eq!(btree.name(), "age_idx");
    assert_eq!(btree.fields(), vec!["age"]);
    assert_eq!(btree.entry_count(), 2);
    assert!(btree.memory_usage_bytes() > 0);
}

#[test]
fn test_fuzzy_index_trait_methods() {
    let mut fuzzy = FuzzyIndex::new("name_fuzzy", "name", FuzzyAlgorithm::JaroWinkler, 0.8);

    fuzzy.insert("Alice", DocumentId::Int(1));
    fuzzy.insert("Bob", DocumentId::Int(2));

    assert!(fuzzy.name().contains("name"));
    assert_eq!(fuzzy.fields(), vec!["name"]);
    assert_eq!(fuzzy.entry_count(), 2);
    assert!(fuzzy.memory_usage_bytes() > 0);
}

// ============================================================================
// Edge cases
// ============================================================================

#[test]
fn test_empty_btree_lazy_loading() {
    let temp_dir = TempDir::new().unwrap();
    let idx_path = temp_dir.path().join("empty.idx");

    // Create empty B+Tree
    let mut btree = BPlusTree::new("field_idx".to_string(), "field".to_string(), false, false);

    // Save empty tree
    {
        let mut file = File::create(&idx_path).unwrap();
        btree.save_to_file(&mut file).unwrap();
    }

    // Reload
    let reloaded =
        BPlusTree::load_from_path(idx_path, test_metadata("field_idx", "field")).unwrap();

    assert!(!reloaded.is_lazy_mode());
    assert_eq!(reloaded.entry_count(), 0);
}

#[test]
fn test_preload_delegates_to_ensure_fully_loaded() {
    let mut btree = BPlusTree::new("field_idx".to_string(), "field".to_string(), false, false);
    btree
        .insert(IndexKey::String("test".to_string()), DocumentId::Int(1))
        .unwrap();

    // preload() should work the same as ensure_fully_loaded()
    btree.preload().unwrap();
    btree.preload().unwrap();

    let result = btree.search(&IndexKey::String("test".to_string()));
    assert_eq!(result, Some(DocumentId::Int(1)));
}

#[test]
fn test_evict_cold_data_returns_zero_for_opcion_a() {
    let mut btree = BPlusTree::new("field_idx".to_string(), "field".to_string(), false, false);
    btree
        .insert(IndexKey::String("test".to_string()), DocumentId::Int(1))
        .unwrap();

    // "Opció A" (read-only lazy loading) doesn't support eviction
    let freed = btree.evict_cold_data(1024 * 1024);
    assert_eq!(freed, 0, "Opció A should not support eviction");
}

// ============================================================================
// Threshold-based lazy loading documentation test
// ============================================================================

/// This test documents the threshold-based lazy loading behavior.
///
/// In production, lazy loading activates when:
/// - File size > calculate_lazy_threshold()
/// - Threshold ranges from 50MB (low RAM) to 500MB (high RAM)
///
/// For testing, we verify the threshold calculation is working.
#[test]
fn test_lazy_loading_threshold_documentation() {
    let threshold = calculate_lazy_threshold();

    // Log the threshold for debugging
    eprintln!(
        "Lazy loading threshold on this system: {} MB",
        threshold / (1024 * 1024)
    );

    // The threshold should be a reasonable value based on RAM
    // On a typical dev machine (8-32GB RAM), expect 200-500MB
    // On CI with less RAM, expect 50-200MB
    assert!(
        threshold >= 50 * 1024 * 1024,
        "Threshold should be at least 50MB"
    );
}
