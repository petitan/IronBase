//! Integration tests for fulltext search AND mode (#38)
//!
//! Tests that `fulltext_search_ext()` with `and_mode: true` only returns
//! documents matching ALL query tokens, not just any.

use ironbase_core::fulltext::FulltextSearchOptions;
use ironbase_core::storage::MemoryStorage;
use ironbase_core::DatabaseCore;
use serde_json::json;
use std::collections::HashMap;

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

/// Setup: 5 docs with different word combinations for AND/OR testing
fn setup_and_mode_db() -> DatabaseCore<MemoryStorage> {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();

    // Doc 1: has both "rust" and "python"
    db.insert_one(
        "articles",
        doc! {
            "title" => "both",
            "content" => "Rust and Python are both great programming languages"
        },
    )
    .unwrap();

    // Doc 2: has only "rust"
    db.insert_one(
        "articles",
        doc! {
            "title" => "rust_only",
            "content" => "Rust is a systems programming language with memory safety"
        },
    )
    .unwrap();

    // Doc 3: has only "python"
    db.insert_one(
        "articles",
        doc! {
            "title" => "python_only",
            "content" => "Python is great for data science and machine learning"
        },
    )
    .unwrap();

    // Doc 4: has "rust", "python", and "web"
    db.insert_one(
        "articles",
        doc! {
            "title" => "all_three",
            "content" => "Rust Python and web technologies are the future of programming"
        },
    )
    .unwrap();

    // Doc 5: has neither "rust" nor "python"
    db.insert_one(
        "articles",
        doc! {
            "title" => "neither",
            "content" => "JavaScript and TypeScript dominate the web development world"
        },
    )
    .unwrap();

    let coll = db.collection("articles").unwrap();
    coll.create_fulltext_index("content".to_string(), "none", None, None)
        .unwrap();

    db
}

#[test]
fn test_and_mode_both_words_required() {
    let db = setup_and_mode_db();
    let coll = db.collection("articles").unwrap();

    // AND mode: "rust python" → only docs with BOTH words
    let options = FulltextSearchOptions {
        limit: Some(10),
        and_mode: true,
        ..Default::default()
    };

    let results = coll
        .fulltext_search_ext("content", "rust python", options)
        .unwrap();

    // Only "both" and "all_three" have both words
    assert_eq!(
        results.len(),
        2,
        "AND mode should return only docs with both 'rust' AND 'python'"
    );

    let titles: Vec<&str> = results
        .iter()
        .filter_map(|r| r.document.get("title").and_then(|v| v.as_str()))
        .collect();
    assert!(titles.contains(&"both"));
    assert!(titles.contains(&"all_three"));
}

#[test]
fn test_or_mode_any_word_matches() {
    let db = setup_and_mode_db();
    let coll = db.collection("articles").unwrap();

    // OR mode (default): "rust python" → docs with either word
    let options = FulltextSearchOptions {
        limit: Some(10),
        and_mode: false,
        ..Default::default()
    };

    let results = coll
        .fulltext_search_ext("content", "rust python", options)
        .unwrap();

    // "both", "rust_only", "python_only", "all_three" all match at least one word
    assert_eq!(
        results.len(),
        4,
        "OR mode should return docs with 'rust' OR 'python'"
    );
}

#[test]
fn test_and_mode_three_words() {
    let db = setup_and_mode_db();
    let coll = db.collection("articles").unwrap();

    // AND mode: "rust python web" → only docs with ALL three
    let options = FulltextSearchOptions {
        limit: Some(10),
        and_mode: true,
        ..Default::default()
    };

    let results = coll
        .fulltext_search_ext("content", "rust python web", options)
        .unwrap();

    // Only "all_three" has all three words
    assert_eq!(
        results.len(),
        1,
        "AND mode with 3 words should return only doc with all 3"
    );

    let title = results[0]
        .document
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap();
    assert_eq!(title, "all_three");
}

#[test]
fn test_and_mode_no_match() {
    let db = setup_and_mode_db();
    let coll = db.collection("articles").unwrap();

    // AND mode: "rust javascript" → no doc has both
    let options = FulltextSearchOptions {
        limit: Some(10),
        and_mode: true,
        ..Default::default()
    };

    let results = coll
        .fulltext_search_ext("content", "rust javascript", options)
        .unwrap();

    assert_eq!(
        results.len(),
        0,
        "AND mode should return 0 when no doc has both words"
    );
}

#[test]
fn test_and_mode_single_word_same_as_or() {
    let db = setup_and_mode_db();
    let coll = db.collection("articles").unwrap();

    // AND mode with 1 word: same result as OR
    let and_options = FulltextSearchOptions {
        limit: Some(10),
        and_mode: true,
        ..Default::default()
    };
    let or_options = FulltextSearchOptions {
        limit: Some(10),
        and_mode: false,
        ..Default::default()
    };

    let and_results = coll
        .fulltext_search_ext("content", "rust", and_options)
        .unwrap();
    let or_results = coll
        .fulltext_search_ext("content", "rust", or_options)
        .unwrap();

    assert_eq!(
        and_results.len(),
        or_results.len(),
        "AND mode with single word should match OR mode"
    );
}

#[test]
fn test_and_mode_preserves_tfidf_scoring() {
    let db = setup_and_mode_db();
    let coll = db.collection("articles").unwrap();

    // AND mode results should still have TF-IDF scores
    let options = FulltextSearchOptions {
        limit: Some(10),
        and_mode: true,
        ..Default::default()
    };

    let results = coll
        .fulltext_search_ext("content", "rust python", options)
        .unwrap();

    for result in &results {
        assert!(
            result.score > 0.0,
            "AND mode results should have positive TF-IDF scores"
        );
        assert!(
            !result.matched_tokens.is_empty(),
            "AND mode results should have matched_tokens"
        );
    }
}

#[test]
fn test_and_mode_with_limit() {
    let db = setup_and_mode_db();
    let coll = db.collection("articles").unwrap();

    // AND mode with limit=1: should return only top result
    let options = FulltextSearchOptions {
        limit: Some(1),
        and_mode: true,
        ..Default::default()
    };

    let results = coll
        .fulltext_search_ext("content", "rust python", options)
        .unwrap();

    assert_eq!(
        results.len(),
        1,
        "AND mode with limit=1 should return 1 result"
    );
}

#[test]
fn test_and_mode_hungarian_stemming() {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();

    // Hungarian docs — stemming should allow AND matching on stemmed forms
    db.insert_one(
        "docs",
        doc! {
            "title" => "both",
            "content" => "A görgős fékerőmérő használata kötelező az éves vizsgálatkor"
        },
    )
    .unwrap();
    db.insert_one(
        "docs",
        doc! {
            "title" => "only_gorgo",
            "content" => "A görgős szalag gyorsítja a gyártást"
        },
    )
    .unwrap();
    db.insert_one(
        "docs",
        doc! {
            "title" => "only_fek",
            "content" => "A fékerőmérő kalibrálása fontos"
        },
    )
    .unwrap();

    let coll = db.collection("docs").unwrap();
    coll.create_fulltext_index("content".to_string(), "hungarian", None, None)
        .unwrap();

    // AND mode: both words required
    let options = FulltextSearchOptions {
        limit: Some(10),
        and_mode: true,
        ..Default::default()
    };

    let results = coll
        .fulltext_search_ext("content", "görgős fékerőmérő", options)
        .unwrap();

    assert_eq!(
        results.len(),
        1,
        "AND mode with Hungarian should find doc with both words"
    );

    let title = results[0]
        .document
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap();
    assert_eq!(title, "both");
}
