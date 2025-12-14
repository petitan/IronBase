//! Integration tests for full-text search functionality
//!
//! Tests cover:
//! - Basic fulltext index creation and search
//! - Language-specific processing (Hungarian, English, German)
//! - Index persistence across database restarts
//! - Index drop and recreation

use ironbase_core::DatabaseCore;
use serde_json::json;
use std::collections::HashMap;
use tempfile::TempDir;

/// Helper macro to create a document HashMap from JSON-like syntax
macro_rules! doc {
    ($($key:expr => $value:expr),* $(,)?) => {{
        let mut map: HashMap<String, serde_json::Value> = HashMap::new();
        $(
            map.insert($key.to_string(), json!($value));
        )*
        map
    }};
}

// ============================================================================
// BASIC FULLTEXT INDEX TESTS
// ============================================================================

#[test]
fn test_fulltext_index_basic() {
    let db = DatabaseCore::open_memory().unwrap();

    // Insert documents
    db.insert_one(
        "articles",
        doc! {
            "title" => "Rust Programming",
            "content" => "Rust is a systems programming language"
        },
    )
    .unwrap();
    db.insert_one(
        "articles",
        doc! {
            "title" => "Python Guide",
            "content" => "Python is great for data science"
        },
    )
    .unwrap();
    db.insert_one(
        "articles",
        doc! {
            "title" => "Rust Web",
            "content" => "Building web applications with Rust and Actix"
        },
    )
    .unwrap();

    let coll = db.collection("articles").unwrap();

    // Create fulltext index
    let index_name = coll
        .create_fulltext_index("content".to_string(), "none", None, None)
        .unwrap();
    assert!(index_name.contains("_fts"));

    // Search for "rust"
    let results = coll
        .fulltext_search("content", "rust", Some(10), None, None, None)
        .unwrap();
    assert_eq!(results.len(), 2, "Should find 2 documents with 'rust'");

    // Search for "python"
    let results = coll
        .fulltext_search("content", "python", Some(10), None, None, None)
        .unwrap();
    assert_eq!(results.len(), 1, "Should find 1 document with 'python'");
}

#[test]
fn test_fulltext_index_hungarian() {
    let db = DatabaseCore::open_memory().unwrap();

    // Insert Hungarian documents
    db.insert_one(
        "cikkek",
        doc! {
            "cim" => "Teszt",
            "tartalom" => "Az autó nagyon szép és gyors"
        },
    )
    .unwrap();
    db.insert_one(
        "cikkek",
        doc! {
            "cim" => "Másik",
            "tartalom" => "A kutya fut az utcán"
        },
    )
    .unwrap();
    db.insert_one(
        "cikkek",
        doc! {
            "cim" => "Harmadik",
            "tartalom" => "Az autók versenye izgalmas volt"
        },
    )
    .unwrap();

    let coll = db.collection("cikkek").unwrap();

    // Create fulltext index with Hungarian language
    coll.create_fulltext_index("tartalom".to_string(), "hungarian", None, None)
        .unwrap();

    // Search - Hungarian stop words should be filtered
    let results = coll
        .fulltext_search("tartalom", "az", Some(10), None, None, None)
        .unwrap();
    assert_eq!(results.len(), 0, "'az' is a Hungarian stop word");

    // Search for actual content
    let results = coll
        .fulltext_search("tartalom", "autó", Some(10), None, None, None)
        .unwrap();
    assert_eq!(
        results.len(),
        2,
        "Should find documents with 'autó' (accent folded)"
    );
}

#[test]
fn test_fulltext_index_english() {
    let db = DatabaseCore::open_memory().unwrap();

    db.insert_one(
        "posts",
        doc! {
            "title" => "Test",
            "body" => "The quick brown fox jumps over the lazy dog"
        },
    )
    .unwrap();
    db.insert_one(
        "posts",
        doc! {
            "title" => "Test2",
            "body" => "Dogs are wonderful pets and companions"
        },
    )
    .unwrap();

    let coll = db.collection("posts").unwrap();

    coll.create_fulltext_index("body".to_string(), "english", None, None)
        .unwrap();

    // "the" is an English stop word
    let results = coll
        .fulltext_search("body", "the", Some(10), None, None, None)
        .unwrap();
    assert_eq!(results.len(), 0, "'the' is an English stop word");

    // Search for "dog" - should match both (stemming: dogs -> dog)
    let results = coll
        .fulltext_search("body", "dog", Some(10), None, None, None)
        .unwrap();
    assert!(!results.is_empty(), "Should find documents with 'dog'");
}

#[test]
fn test_fulltext_index_accent_folding() {
    let db = DatabaseCore::open_memory().unwrap();

    db.insert_one(
        "test",
        doc! {
            "text" => "Árvíztűrő tükörfúrógép"
        },
    )
    .unwrap();
    db.insert_one(
        "test",
        doc! {
            "text" => "Normal text without accents"
        },
    )
    .unwrap();

    let coll = db.collection("test").unwrap();

    coll.create_fulltext_index("text".to_string(), "none", None, Some(true))
        .unwrap();

    // Search without accents should find accented text
    let results = coll
        .fulltext_search("text", "arvizturo", Some(10), None, None, None)
        .unwrap();
    assert_eq!(results.len(), 1, "Accent folding should match");
}

#[test]
fn test_fulltext_search_tfidf_ranking() {
    let db = DatabaseCore::open_memory().unwrap();

    // doc1 has "rust" many times
    db.insert_one(
        "docs",
        doc! {
            "content" => "rust rust rust rust rust programming"
        },
    )
    .unwrap();
    // doc2 has "rust" once
    db.insert_one(
        "docs",
        doc! {
            "content" => "rust is nice"
        },
    )
    .unwrap();
    // doc3 has no "rust"
    db.insert_one(
        "docs",
        doc! {
            "content" => "python is great"
        },
    )
    .unwrap();

    let coll = db.collection("docs").unwrap();

    coll.create_fulltext_index("content".to_string(), "none", None, None)
        .unwrap();

    let results = coll
        .fulltext_search("content", "rust", Some(10), None, None, None)
        .unwrap();

    assert_eq!(results.len(), 2);
    // First result should have higher score (more "rust" occurrences)
    assert!(
        results[0].1 > results[1].1,
        "TF-IDF should rank by term frequency"
    );
}

// ============================================================================
// PERSISTENCE TESTS
// ============================================================================

#[test]
fn test_fulltext_index_persistence() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test_fts_persist.mlite");

    // Phase 1: Create database, add data and create fulltext index
    {
        let db = DatabaseCore::open(&db_path).unwrap();

        db.insert_one(
            "articles",
            doc! {
                "title" => "Test",
                "content" => "Rust programming language"
            },
        )
        .unwrap();
        db.insert_one(
            "articles",
            doc! {
                "title" => "Test2",
                "content" => "Python for data science"
            },
        )
        .unwrap();

        let coll = db.collection("articles").unwrap();

        // Create fulltext index
        coll.create_fulltext_index("content".to_string(), "english", None, None)
            .unwrap();

        // Verify search works
        let results = coll
            .fulltext_search("content", "rust", Some(10), None, None, None)
            .unwrap();
        assert_eq!(results.len(), 1, "Should find Rust article");
    }
    // db dropped here - should flush

    // Phase 2: Reopen database and verify fulltext index was restored
    {
        let db2 = DatabaseCore::open(&db_path).unwrap();
        let coll2 = db2.collection("articles").unwrap();

        // Verify fulltext search still works after reload
        let results = coll2
            .fulltext_search("content", "rust", Some(10), None, None, None)
            .unwrap();
        assert!(
            !results.is_empty(),
            "Fulltext search should work after database reload, found {} results",
            results.len()
        );

        // Verify Python search also works
        let results = coll2
            .fulltext_search("content", "python", Some(10), None, None, None)
            .unwrap();
        assert_eq!(results.len(), 1, "Should find Python article after reload");
    }
}

#[test]
fn test_fulltext_index_persistence_with_drop() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test_fts_drop.mlite");

    // Phase 1: Create database with fulltext index, then drop it
    {
        let db = DatabaseCore::open(&db_path).unwrap();

        db.insert_one(
            "articles",
            doc! {
                "content" => "Test document"
            },
        )
        .unwrap();

        let coll = db.collection("articles").unwrap();

        let index_name = coll
            .create_fulltext_index("content".to_string(), "none", None, None)
            .unwrap();

        // Verify it works
        let results = coll
            .fulltext_search("content", "test", Some(10), None, None, None)
            .unwrap();
        assert_eq!(results.len(), 1);

        // Drop the index
        coll.drop_index(&index_name).unwrap();
    }

    // Phase 2: Reopen - index should NOT exist
    {
        let db2 = DatabaseCore::open(&db_path).unwrap();
        let coll2 = db2.collection("articles").unwrap();

        // Search should fail because index was dropped
        let result = coll2.fulltext_search("content", "test", Some(10), None, None, None);
        assert!(
            result.is_err(),
            "Fulltext search should fail - index was dropped"
        );
    }
}

#[test]
fn test_fulltext_index_persistence_multiple_indexes() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test_fts_multi.mlite");

    // Phase 1: Create multiple fulltext indexes
    {
        let db = DatabaseCore::open(&db_path).unwrap();

        db.insert_one(
            "articles",
            doc! {
                "title" => "Rust Guide",
                "body" => "Learn Rust programming"
            },
        )
        .unwrap();
        db.insert_one(
            "articles",
            doc! {
                "title" => "Python Tips",
                "body" => "Python best practices"
            },
        )
        .unwrap();

        let coll = db.collection("articles").unwrap();

        // Create two fulltext indexes
        coll.create_fulltext_index("title".to_string(), "none", None, None)
            .unwrap();
        coll.create_fulltext_index("body".to_string(), "english", None, None)
            .unwrap();

        // Verify both work
        let results1 = coll
            .fulltext_search("title", "rust", Some(10), None, None, None)
            .unwrap();
        assert_eq!(results1.len(), 1);

        let results2 = coll
            .fulltext_search("body", "programming", Some(10), None, None, None)
            .unwrap();
        assert_eq!(results2.len(), 1);
    }

    // Phase 2: Verify both indexes restored
    {
        let db2 = DatabaseCore::open(&db_path).unwrap();
        let coll2 = db2.collection("articles").unwrap();

        let results1 = coll2
            .fulltext_search("title", "python", Some(10), None, None, None)
            .unwrap();
        assert_eq!(results1.len(), 1, "Title index should be restored");

        let results2 = coll2
            .fulltext_search("body", "best", Some(10), None, None, None)
            .unwrap();
        assert_eq!(results2.len(), 1, "Body index should be restored");
    }
}

// ============================================================================
// EDGE CASE TESTS
// ============================================================================

#[test]
fn test_fulltext_empty_query() {
    let db = DatabaseCore::open_memory().unwrap();

    db.insert_one("test", doc! {"text" => "Hello world"})
        .unwrap();

    let coll = db.collection("test").unwrap();
    coll.create_fulltext_index("text".to_string(), "none", None, None)
        .unwrap();

    let results = coll
        .fulltext_search("text", "", Some(10), None, None, None)
        .unwrap();
    assert_eq!(results.len(), 0, "Empty query should return no results");
}

#[test]
fn test_fulltext_min_word_length() {
    let db = DatabaseCore::open_memory().unwrap();

    db.insert_one("test", doc! {"text" => "a ab abc abcd"})
        .unwrap();

    let coll = db.collection("test").unwrap();

    // Create index with min_word_length = 3
    coll.create_fulltext_index("text".to_string(), "none", Some(3), None)
        .unwrap();

    // "a" and "ab" should not be indexed
    let results = coll
        .fulltext_search("text", "a", Some(10), None, None, None)
        .unwrap();
    assert_eq!(
        results.len(),
        0,
        "Single char should not be indexed with min_word_length=3"
    );

    let results = coll
        .fulltext_search("text", "ab", Some(10), None, None, None)
        .unwrap();
    assert_eq!(
        results.len(),
        0,
        "2-char word should not be indexed with min_word_length=3"
    );

    // "abc" and "abcd" should be indexed
    let results = coll
        .fulltext_search("text", "abc", Some(10), None, None, None)
        .unwrap();
    assert_eq!(results.len(), 1, "3-char word should be indexed");
}

#[test]
fn test_fulltext_pagination() {
    let db = DatabaseCore::open_memory().unwrap();

    // Insert 10 documents all containing "test"
    for i in 0..10 {
        db.insert_one(
            "test",
            doc! {
                "text" => format!("test document {}", i)
            },
        )
        .unwrap();
    }

    let coll = db.collection("test").unwrap();
    coll.create_fulltext_index("text".to_string(), "none", None, None)
        .unwrap();

    // Get first 3
    let results = coll
        .fulltext_search("text", "test", Some(3), Some(0), None, None)
        .unwrap();
    assert_eq!(results.len(), 3);

    // Skip 3, get next 3
    let results = coll
        .fulltext_search("text", "test", Some(3), Some(3), None, None)
        .unwrap();
    assert_eq!(results.len(), 3);

    // Skip 8, get remaining
    let results = coll
        .fulltext_search("text", "test", Some(10), Some(8), None, None)
        .unwrap();
    assert_eq!(results.len(), 2);
}

#[test]
fn test_fulltext_min_score_filter() {
    let db = DatabaseCore::open_memory().unwrap();

    // Doc with many "rust" occurrences
    db.insert_one(
        "test",
        doc! {
            "text" => "rust rust rust rust rust"
        },
    )
    .unwrap();
    // Doc with one "rust"
    db.insert_one(
        "test",
        doc! {
            "text" => "rust is nice"
        },
    )
    .unwrap();

    let coll = db.collection("test").unwrap();
    coll.create_fulltext_index("text".to_string(), "none", None, None)
        .unwrap();

    // Get all results first to see scores
    let all_results = coll
        .fulltext_search("text", "rust", Some(10), None, None, None)
        .unwrap();
    assert_eq!(all_results.len(), 2);

    let high_score = all_results[0].1;

    // Filter by high score - should only get the first one
    let filtered = coll
        .fulltext_search(
            "text",
            "rust",
            Some(10),
            None,
            Some(high_score - 0.01),
            None,
        )
        .unwrap();
    assert_eq!(filtered.len(), 1, "Min score filter should work");
}

// ============================================================================
// PROJECTION TESTS
// ============================================================================

#[test]
fn test_fulltext_search_with_projection_exclude() {
    let db = DatabaseCore::open_memory().unwrap();

    db.insert_one(
        "docs",
        doc! {
            "title" => "Rust Guide",
            "content" => "Rust programming language guide",
            "large_field" => "This is a very large field that we want to exclude from results"
        },
    )
    .unwrap();

    let coll = db.collection("docs").unwrap();
    coll.create_fulltext_index("content".to_string(), "none", None, None)
        .unwrap();

    // Search WITHOUT projection - all fields returned
    let results_full = coll
        .fulltext_search("content", "rust", Some(10), None, None, None)
        .unwrap();
    assert_eq!(results_full.len(), 1);
    let doc_full = &results_full[0].0;
    assert!(
        doc_full.get("large_field").is_some(),
        "Full doc should have large_field"
    );
    assert!(
        doc_full.get("title").is_some(),
        "Full doc should have title"
    );

    // Search WITH projection - exclude large_field
    let mut projection = HashMap::new();
    projection.insert("large_field".to_string(), 0);
    let results_projected = coll
        .fulltext_search("content", "rust", Some(10), None, None, Some(projection))
        .unwrap();
    assert_eq!(results_projected.len(), 1);
    let doc_projected = &results_projected[0].0;
    assert!(
        doc_projected.get("large_field").is_none(),
        "Projected doc should NOT have large_field"
    );
    assert!(
        doc_projected.get("title").is_some(),
        "Projected doc should still have title"
    );
    assert!(
        doc_projected.get("content").is_some(),
        "Projected doc should still have content"
    );
}

#[test]
fn test_fulltext_search_with_projection_include() {
    let db = DatabaseCore::open_memory().unwrap();

    db.insert_one(
        "docs",
        doc! {
            "title" => "Python Tutorial",
            "content" => "Python is great for beginners",
            "author" => "John Doe",
            "metadata" => json!({"views": 1000, "likes": 50})
        },
    )
    .unwrap();

    let coll = db.collection("docs").unwrap();
    coll.create_fulltext_index("content".to_string(), "none", None, None)
        .unwrap();

    // Search WITH projection - include only title and _id
    let mut projection = HashMap::new();
    projection.insert("title".to_string(), 1);
    projection.insert("_id".to_string(), 1);
    let results = coll
        .fulltext_search("content", "python", Some(10), None, None, Some(projection))
        .unwrap();
    assert_eq!(results.len(), 1);
    let doc = &results[0].0;

    // Should have title and _id
    assert!(doc.get("title").is_some(), "Should have title");
    assert!(doc.get("_id").is_some(), "Should have _id");

    // Should NOT have other fields
    assert!(
        doc.get("content").is_none(),
        "Should NOT have content in include mode"
    );
    assert!(
        doc.get("author").is_none(),
        "Should NOT have author in include mode"
    );
    assert!(
        doc.get("metadata").is_none(),
        "Should NOT have metadata in include mode"
    );
}

// ============================================================================
// REGRESSION TESTS FOR BUG FIXES
// ============================================================================

/// Regression test for BUG #1: Fulltext index not updated after update_many
///
/// The bug was that batch_update_indexes() only updated btree indexes,
/// not fulltext indexes. After update_many, the fulltext index still had
/// the old content tokens, causing searches to return stale/wrong results.
///
/// Fix: Added fulltext index update loop in batch_update_indexes().
#[test]
fn test_fulltext_index_updated_after_update_many() {
    let db = DatabaseCore::open_memory().unwrap();

    // Insert documents with original content
    db.insert_many(
        "bug1_test",
        vec![
            doc! {"_id" => "doc1", "content" => "ORIGINAL TEST CONTENT A"},
            doc! {"_id" => "doc2", "content" => "ORIGINAL TEST CONTENT B"},
            doc! {"_id" => "doc3", "content" => "ORIGINAL TEST CONTENT C"},
        ],
    )
    .unwrap();

    // Create fulltext index
    let coll = db.collection("bug1_test").unwrap();
    coll.create_fulltext_index("content".to_string(), "english", None, None)
        .unwrap();

    // Verify original content is indexed
    let original_results = coll
        .fulltext_search("content", "ORIGINAL", Some(10), None, None, None)
        .unwrap();
    assert_eq!(
        original_results.len(),
        3,
        "Should find 3 documents with 'ORIGINAL'"
    );

    // Update all documents with update_many
    let (matched, modified) = db
        .update_many(
            "bug1_test",
            &json!({}),
            &json!({"$set": {"content": "BULK UPDATED CONTENT"}}),
        )
        .unwrap();
    assert_eq!(matched, 3);
    assert_eq!(modified, 3);

    // BUG #1 TEST: Search for new content - should find 3 documents
    let new_results = coll
        .fulltext_search("content", "BULK", Some(10), None, None, None)
        .unwrap();
    assert_eq!(
        new_results.len(),
        3,
        "BUG #1 regression: fulltext index not updated after update_many! Search for 'BULK' should find 3 documents."
    );

    // BUG #1 TEST: Search for old content - should find 0 documents
    let old_results = coll
        .fulltext_search("content", "ORIGINAL", Some(10), None, None, None)
        .unwrap();
    assert_eq!(
        old_results.len(),
        0,
        "BUG #1 regression: old content still in fulltext index! Search for 'ORIGINAL' should find 0 documents."
    );
}

/// Regression test for BUG #3: Duplicate documents after update_many
///
/// The bug was that apply_batch_updates() in B+ tree was calling
/// build_from_sorted() without clearing the existing tree first,
/// causing duplicate entries.
///
/// Fix: Added clear() call before build_from_sorted() in apply_batch_updates().
#[test]
fn test_no_duplicate_documents_after_update_many() {
    let db = DatabaseCore::open_memory().unwrap();

    // Insert documents
    db.insert_many(
        "bug3_test",
        vec![
            doc! {"_id" => "dup001", "content" => "ORIGINAL A"},
            doc! {"_id" => "dup002", "content" => "ORIGINAL B"},
            doc! {"_id" => "dup003", "content" => "ORIGINAL C"},
        ],
    )
    .unwrap();

    // Update all documents with update_many
    let (matched, modified) = db
        .update_many(
            "bug3_test",
            &json!({}),
            &json!({"$set": {"content": "UPDATED"}}),
        )
        .unwrap();
    assert_eq!(matched, 3);
    assert_eq!(modified, 3);

    // BUG #3 TEST: Find specific document by _id - should return exactly 1
    let coll = db.collection("bug3_test").unwrap();
    let results = coll.find(&json!({"_id": "dup001"})).unwrap();
    assert_eq!(
        results.len(),
        1,
        "BUG #3 regression: find() returned {} documents for _id='dup001', expected 1. \
         This indicates duplicate index entries were created by update_many.",
        results.len()
    );

    // Verify the content was actually updated
    assert_eq!(
        results[0].get("content").and_then(|v| v.as_str()),
        Some("UPDATED"),
        "Document content should be updated"
    );

    // Double-check: count_documents should also return 1
    let count = coll.count_documents(&json!({"_id": "dup001"})).unwrap();
    assert_eq!(
        count, 1,
        "BUG #3 regression: count_documents returned {} for _id='dup001', expected 1",
        count
    );

    // Total count should be 3
    let total = coll.count_documents(&json!({})).unwrap();
    assert_eq!(total, 3, "Total document count should be 3");
}
