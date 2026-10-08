//! Fulltext search with user-controlled limit/skip and repeated query tokens
//! (audit 2026-10-07 M1, M4): memory must follow the matches, not the request.

use ironbase_core::fulltext::FulltextSearchOptions;
use ironbase_core::storage::MemoryStorage;
use ironbase_core::DatabaseCore;
use serde_json::json;
use std::collections::HashMap;

fn setup(docs: usize) -> DatabaseCore<MemoryStorage> {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    for i in 0..docs {
        let mut d: HashMap<String, serde_json::Value> = HashMap::new();
        d.insert("i".to_string(), json!(i));
        d.insert("content".to_string(), json!(format!("apple banana {i}")));
        db.insert_one("articles", d).unwrap();
    }
    db.collection("articles")
        .unwrap()
        .create_fulltext_index("content".to_string(), "english", None, None)
        .unwrap();
    db
}

/// A huge limit used to pre-size the top-k heap (`with_capacity(skip + k)`),
/// which aborted the process on allocation failure.
#[test]
fn huge_limit_and_skip_do_not_preallocate() {
    let db = setup(3);
    let coll = db.collection("articles").unwrap();

    let all = coll
        .fulltext_search_ext(
            "content",
            "apple",
            FulltextSearchOptions::new().with_limit(1usize << 40),
        )
        .unwrap();
    assert_eq!(all.len(), 3);

    let none = coll
        .fulltext_search_ext(
            "content",
            "apple",
            FulltextSearchOptions::new()
                .with_limit(usize::MAX)
                .with_skip(usize::MAX),
        )
        .unwrap();
    assert!(none.is_empty());

    let page = coll
        .fulltext_search_ext(
            "content",
            "apple",
            FulltextSearchOptions::new().with_limit(1).with_skip(2),
        )
        .unwrap();
    assert_eq!(page.len(), 1);
}

/// A repeated query token is scored and reported once.
#[test]
fn repeated_query_tokens_count_once() {
    let db = setup(3);
    let coll = db.collection("articles").unwrap();
    let query = "apple ".repeat(500);

    let repeated = coll
        .fulltext_search_ext(
            "content",
            &query,
            FulltextSearchOptions::new().with_limit(3),
        )
        .unwrap();
    let single = coll
        .fulltext_search_ext(
            "content",
            "apple",
            FulltextSearchOptions::new().with_limit(3),
        )
        .unwrap();

    assert_eq!(repeated.len(), 3);
    for (r, s) in repeated.iter().zip(&single) {
        assert_eq!(r.matched_tokens, s.matched_tokens);
        assert!(
            (r.score - s.score).abs() < 1e-9,
            "{} vs {}",
            r.score,
            s.score
        );
    }
}
