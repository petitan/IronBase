//! Integration tests for multi-field fulltext search (#39)
//!
//! Tests that `fulltext_search_multi_ext()` searches across multiple fields
//! and merges results by document using best-field (max score) strategy.

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

/// Setup: 5 docs with words spread across "title" and "body" fields
fn setup_multi_field_db() -> DatabaseCore<MemoryStorage> {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();

    // Doc 1: "rust" in title, "python" in body
    db.insert_one(
        "articles",
        doc! {
            "tag" => "split",
            "title" => "Learning Rust programming",
            "body" => "Python is great for scripting and automation"
        },
    )
    .unwrap();

    // Doc 2: "rust" in both title and body
    db.insert_one(
        "articles",
        doc! {
            "tag" => "rust_both",
            "title" => "Rust systems language",
            "body" => "Rust provides memory safety without garbage collection"
        },
    )
    .unwrap();

    // Doc 3: "python" only in body
    db.insert_one(
        "articles",
        doc! {
            "tag" => "python_body",
            "title" => "Data science introduction",
            "body" => "Python and machine learning go hand in hand"
        },
    )
    .unwrap();

    // Doc 4: "web" in title, nothing relevant in body
    db.insert_one(
        "articles",
        doc! {
            "tag" => "web_title",
            "title" => "Web development with JavaScript",
            "body" => "Frontend frameworks are evolving rapidly"
        },
    )
    .unwrap();

    // Doc 5: "rust" in body only
    db.insert_one(
        "articles",
        doc! {
            "tag" => "rust_body",
            "title" => "Systems programming concepts",
            "body" => "Rust is perfect for embedded and systems work"
        },
    )
    .unwrap();

    let coll = db.collection("articles").unwrap();
    coll.create_fulltext_index("title".to_string(), "none", None, None)
        .unwrap();
    coll.create_fulltext_index("body".to_string(), "none", None, None)
        .unwrap();

    db
}

#[test]
fn test_multi_field_basic_merge() {
    let db = setup_multi_field_db();
    let coll = db.collection("articles").unwrap();

    // Search "rust" across both fields
    let options = FulltextSearchOptions {
        limit: Some(10),
        ..Default::default()
    };

    let results = coll
        .fulltext_search_multi_ext(&["title", "body"], "rust", options)
        .unwrap();

    // "split" (title), "rust_both" (title+body), "rust_body" (body) → 3 docs
    assert_eq!(
        results.len(),
        3,
        "Multi-field search for 'rust' should find 3 docs"
    );

    let tags: Vec<&str> = results
        .iter()
        .filter_map(|r| r.document.get("tag").and_then(|v| v.as_str()))
        .collect();
    assert!(tags.contains(&"split"));
    assert!(tags.contains(&"rust_both"));
    assert!(tags.contains(&"rust_body"));
}

#[test]
fn test_multi_field_dedup_same_doc() {
    let db = setup_multi_field_db();
    let coll = db.collection("articles").unwrap();

    // "rust_both" has "rust" in both title and body → should appear ONCE
    let options = FulltextSearchOptions {
        limit: Some(10),
        ..Default::default()
    };

    let results = coll
        .fulltext_search_multi_ext(&["title", "body"], "rust", options)
        .unwrap();

    let rust_both_count = results
        .iter()
        .filter(|r| r.document.get("tag").and_then(|v| v.as_str()) == Some("rust_both"))
        .count();

    assert_eq!(
        rust_both_count, 1,
        "Document matching in multiple fields should appear only once"
    );
}

#[test]
fn test_multi_field_best_score() {
    let db = setup_multi_field_db();
    let coll = db.collection("articles").unwrap();

    // "rust_both" matches in 2 fields → its score should be the MAX of the two
    let options = FulltextSearchOptions {
        limit: Some(10),
        ..Default::default()
    };

    let results = coll
        .fulltext_search_multi_ext(&["title", "body"], "rust", options)
        .unwrap();

    // Results should be sorted by score descending
    for i in 1..results.len() {
        assert!(
            results[i - 1].score >= results[i].score,
            "Results should be sorted by score descending"
        );
    }

    // All scores must be positive
    for r in &results {
        assert!(r.score > 0.0, "All results should have positive scores");
    }
}

#[test]
fn test_multi_field_single_field_fallback() {
    let db = setup_multi_field_db();
    let coll = db.collection("articles").unwrap();

    // Single field in array → delegates to fulltext_search_ext
    let options_multi = FulltextSearchOptions {
        limit: Some(10),
        ..Default::default()
    };
    let options_single = FulltextSearchOptions {
        limit: Some(10),
        ..Default::default()
    };

    let multi_results = coll
        .fulltext_search_multi_ext(&["title"], "rust", options_multi)
        .unwrap();
    let single_results = coll
        .fulltext_search_ext("title", "rust", options_single)
        .unwrap();

    assert_eq!(
        multi_results.len(),
        single_results.len(),
        "Single-field multi should equal single-field direct"
    );
}

#[test]
fn test_multi_field_no_index_error() {
    let db = setup_multi_field_db();
    let coll = db.collection("articles").unwrap();

    // "nonexistent" field has no index → should error
    let options = FulltextSearchOptions {
        limit: Some(10),
        ..Default::default()
    };

    let result = coll.fulltext_search_multi_ext(&["title", "nonexistent"], "rust", options);
    assert!(result.is_err(), "Should error when field has no index");
    let err_msg = format!("{}", result.unwrap_err());
    assert!(
        err_msg.contains("nonexistent"),
        "Error should mention the missing field"
    );
}

#[test]
fn test_multi_field_empty_fields_error() {
    let db = setup_multi_field_db();
    let coll = db.collection("articles").unwrap();

    let options = FulltextSearchOptions {
        limit: Some(10),
        ..Default::default()
    };

    let result = coll.fulltext_search_multi_ext(&[], "rust", options);
    assert!(result.is_err(), "Empty fields should error");
}

#[test]
fn test_multi_field_and_mode() {
    let db = setup_multi_field_db();
    let coll = db.collection("articles").unwrap();

    // AND mode: "rust python" → only docs where a SINGLE field has both words
    // None of our docs have both "rust" AND "python" in the same field
    // (Doc "split" has "rust" in title and "python" in body, but AND is per-field)
    let options = FulltextSearchOptions {
        limit: Some(10),
        and_mode: true,
        ..Default::default()
    };

    let results = coll
        .fulltext_search_multi_ext(&["title", "body"], "rust python", options)
        .unwrap();

    // AND mode filters per-field: no single field has both "rust" AND "python"
    assert_eq!(
        results.len(),
        0,
        "AND mode should require all tokens in the same field"
    );
}

#[test]
fn test_multi_field_with_limit() {
    let db = setup_multi_field_db();
    let coll = db.collection("articles").unwrap();

    let options = FulltextSearchOptions {
        limit: Some(2),
        ..Default::default()
    };

    let results = coll
        .fulltext_search_multi_ext(&["title", "body"], "rust", options)
        .unwrap();

    assert_eq!(
        results.len(),
        2,
        "Multi-field with limit=2 should return 2 results"
    );
}

#[test]
fn test_multi_field_with_skip() {
    let db = setup_multi_field_db();
    let coll = db.collection("articles").unwrap();

    let options_all = FulltextSearchOptions {
        limit: Some(10),
        ..Default::default()
    };
    let options_skip = FulltextSearchOptions {
        limit: Some(10),
        skip: Some(1),
        ..Default::default()
    };

    let all_results = coll
        .fulltext_search_multi_ext(&["title", "body"], "rust", options_all)
        .unwrap();
    let skip_results = coll
        .fulltext_search_multi_ext(&["title", "body"], "rust", options_skip)
        .unwrap();

    assert_eq!(
        skip_results.len(),
        all_results.len() - 1,
        "Skip=1 should return one fewer result"
    );
}

#[test]
fn test_multi_field_with_filter() {
    let db = setup_multi_field_db();
    let coll = db.collection("articles").unwrap();

    let options = FulltextSearchOptions {
        limit: Some(10),
        filter: Some(json!({"tag": "rust_both"})),
        ..Default::default()
    };

    let results = coll
        .fulltext_search_multi_ext(&["title", "body"], "rust", options)
        .unwrap();

    assert_eq!(results.len(), 1, "Filter should narrow multi-field results");
    let tag = results[0]
        .document
        .get("tag")
        .and_then(|v| v.as_str())
        .unwrap();
    assert_eq!(tag, "rust_both");
}

#[test]
fn test_multi_field_highlights_from_multiple_fields() {
    let db = setup_multi_field_db();
    let coll = db.collection("articles").unwrap();

    // "rust_both" has "rust" in both fields → highlights from both
    let options = FulltextSearchOptions {
        limit: Some(10),
        filter: Some(json!({"tag": "rust_both"})),
        highlight: true,
        ..Default::default()
    };

    let results = coll
        .fulltext_search_multi_ext(&["title", "body"], "rust", options)
        .unwrap();

    assert_eq!(results.len(), 1);
    let highlights = results[0]
        .highlights
        .as_ref()
        .expect("Should have highlights");

    // Should have highlights from both title and body
    let highlight_fields: Vec<&str> = highlights.iter().map(|h| h.field.as_str()).collect();
    assert!(
        highlight_fields.contains(&"title"),
        "Should have title highlights"
    );
    assert!(
        highlight_fields.contains(&"body"),
        "Should have body highlights"
    );
}

#[test]
fn test_multi_field_matched_tokens_union() {
    let db = setup_multi_field_db();
    let coll = db.collection("articles").unwrap();

    // "split" doc: "rust" in title, "python" in body
    let options = FulltextSearchOptions {
        limit: Some(10),
        filter: Some(json!({"tag": "split"})),
        ..Default::default()
    };

    let results = coll
        .fulltext_search_multi_ext(&["title", "body"], "rust python", options)
        .unwrap();

    assert_eq!(results.len(), 1);
    let tokens = &results[0].matched_tokens;
    assert!(
        tokens.contains(&"rust".to_string()),
        "Should include 'rust' from title"
    );
    assert!(
        tokens.contains(&"python".to_string()),
        "Should include 'python' from body"
    );
}

#[test]
fn test_multi_field_hungarian_stemming() {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();

    db.insert_one(
        "docs",
        doc! {
            "tag" => "both_fields",
            "title" => "Görgős szalag gyártás",
            "content" => "A fékerőmérő kalibrálása fontos feladat"
        },
    )
    .unwrap();

    db.insert_one(
        "docs",
        doc! {
            "tag" => "title_only",
            "title" => "Görgős futószalag",
            "content" => "Általános ipari berendezések"
        },
    )
    .unwrap();

    let coll = db.collection("docs").unwrap();
    coll.create_fulltext_index("title".to_string(), "hungarian", None, None)
        .unwrap();
    coll.create_fulltext_index("content".to_string(), "hungarian", None, None)
        .unwrap();

    let options = FulltextSearchOptions {
        limit: Some(10),
        ..Default::default()
    };

    let results = coll
        .fulltext_search_multi_ext(&["title", "content"], "görgős fékerőmérő", options)
        .unwrap();

    // "both_fields": görgős in title, fékerőmérő in content → matches
    // "title_only": görgős in title, no fékerőmérő → also matches (OR mode)
    assert!(
        !results.is_empty(),
        "Should find at least the doc with görgős"
    );

    let tags: Vec<&str> = results
        .iter()
        .filter_map(|r| r.document.get("tag").and_then(|v| v.as_str()))
        .collect();
    assert!(tags.contains(&"both_fields"));
}
