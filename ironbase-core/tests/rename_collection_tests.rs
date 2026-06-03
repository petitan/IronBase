//! Integration tests for `DatabaseCore::rename_collection`.
//!
//! Covers the full rename path on a file-backed database: catalog move, on-disk
//! index-file rename (not rebuild), in-memory index re-key, error cases, and
//! persistence across a reopen.

use ironbase_core::{DatabaseCore, StorageEngine};
use serde_json::{json, Value};
use std::collections::HashMap;
use tempfile::TempDir;

fn obj(v: Value) -> HashMap<String, Value> {
    v.as_object()
        .expect("object literal")
        .iter()
        .map(|(k, val)| (k.clone(), val.clone()))
        .collect()
}

/// Count files with a given extension in a directory (non-recursive).
fn count_ext(dir: &std::path::Path, ext: &str) -> usize {
    std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some(ext))
        .count()
}

#[test]
fn rename_collection_moves_data_indexes_and_persists() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().to_path_buf();
    let db_path = dir.join("rename_test.mlite");

    {
        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        db.insert_one(
            "articles",
            obj(json!({"title": "A", "content": "rust embedded database"})),
        )
        .unwrap();
        db.insert_one(
            "articles",
            obj(json!({"title": "B", "content": "rust async runtime"})),
        )
        .unwrap();
        db.insert_one(
            "articles",
            obj(json!({"title": "C", "content": "python data science"})),
        )
        .unwrap();

        let coll = db.collection("articles").unwrap();
        coll.create_fulltext_index("content".to_string(), "english", None, None)
            .unwrap();
        coll.create_index("title".to_string(), false, false)
            .unwrap();

        // Flush indexes to disk so rename exercises the real fs::rename move path.
        db.checkpoint().unwrap();
        let ftidx_before = count_ext(&dir, "ftidx");
        assert_eq!(ftidx_before, 1, "one fulltext index file before rename");

        let before = coll
            .fulltext_search("content", "rust", Some(10), None, None, None)
            .unwrap();
        assert_eq!(before.len(), 2, "two docs mention rust before rename");

        // --- rename ---
        db.rename_collection("articles", "posts").unwrap();

        let cols = db.list_collections();
        assert!(cols.contains(&"posts".to_string()), "new name present");
        assert!(
            !cols.contains(&"articles".to_string()),
            "old name gone after rename"
        );

        // No orphaned index file left behind: still exactly one .ftidx (moved).
        assert_eq!(
            count_ext(&dir, "ftidx"),
            1,
            "fulltext index file moved, not duplicated/leaked"
        );

        let posts = db.collection("posts").unwrap();
        assert_eq!(
            posts.count_documents(&json!({})).unwrap(),
            3,
            "all documents accessible under the new name"
        );

        let after = posts
            .fulltext_search("content", "rust", Some(10), None, None, None)
            .unwrap();
        assert_eq!(
            after.len(),
            before.len(),
            "fulltext search works under the new name (index moved + re-keyed)"
        );

        // --- error / edge cases ---
        db.rename_collection("posts", "posts").unwrap(); // identity no-op
        db.insert_one("other", obj(json!({"x": 1}))).unwrap();
        assert!(
            db.rename_collection("posts", "other").is_err(),
            "renaming onto an existing collection must fail"
        );
        assert!(
            db.rename_collection("ghost", "z").is_err(),
            "renaming a missing collection must fail"
        );

        db.checkpoint().unwrap();
    }

    // --- reopen: data + fulltext index survive under the new name ---
    {
        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        let posts = db.collection("posts").unwrap();
        assert_eq!(
            posts.count_documents(&json!({})).unwrap(),
            3,
            "documents persist after reopen under the new name"
        );
        let hits = posts
            .fulltext_search("content", "rust", Some(10), None, None, None)
            .unwrap();
        assert_eq!(
            hits.len(),
            2,
            "fulltext index persists under the new name after reopen"
        );
        let cols = db.list_collections();
        assert!(cols.contains(&"posts".to_string()));
        assert!(!cols.contains(&"articles".to_string()));
    }
}

#[test]
fn rename_collection_memory_storage() {
    use ironbase_core::storage::MemoryStorage;

    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    db.insert_one("src", obj(json!({"v": 1}))).unwrap();
    db.insert_one("src", obj(json!({"v": 2}))).unwrap();

    db.rename_collection("src", "dst").unwrap();

    let cols = db.list_collections();
    assert!(cols.contains(&"dst".to_string()));
    assert!(!cols.contains(&"src".to_string()));
    assert_eq!(
        db.collection("dst")
            .unwrap()
            .count_documents(&json!({}))
            .unwrap(),
        2
    );
}
