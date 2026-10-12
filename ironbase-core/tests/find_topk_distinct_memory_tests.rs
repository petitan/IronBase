// find_topk_distinct_memory_tests.rs
// Working-set byte limits for the find top-k heap (audit 2026-10-07 M6) and
// for distinct (M7).

use ironbase_core::find_options::FindOptions;
use ironbase_core::storage::MemoryStorage;
use ironbase_core::DatabaseCore;
use serde_json::{json, Value};
use std::collections::HashMap;

/// 200 documents of ~10 KB each, `n` = 0..200.
fn db_with_large_docs() -> DatabaseCore<MemoryStorage> {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    let pad = "x".repeat(10_000);
    for n in 0..200 {
        let doc = json!({"n": n, "pad": pad});
        let fields: HashMap<String, Value> = doc.as_object().unwrap().clone().into_iter().collect();
        db.insert_one("c", fields).unwrap();
    }
    db
}

/// M6: a deep skip keeps skip + limit documents in the top-k heap; the byte
/// limit applies to them during the scan, not only to the final page.
#[test]
fn find_sort_deep_skip_respects_byte_limit() {
    let db = db_with_large_docs();
    let coll = db.collection("c").unwrap();
    let opts = FindOptions::new()
        .with_sort(vec![("n".to_string(), 1)])
        .with_skip(190)
        .with_limit(5)
        .with_max_response_bytes(500_000);
    // 195 retained documents (~2 MB) against a 500 KB limit; the 5-document
    // page itself (~50 KB) is under it
    let err = coll.find_with_options(&json!({}), opts).unwrap_err();
    assert!(err.to_string().contains("byte limit"), "{err}");
}

/// M6: within the limit the deep-skip page is unchanged.
#[test]
fn find_sort_skip_within_byte_limit_succeeds() {
    let db = db_with_large_docs();
    let coll = db.collection("c").unwrap();
    let opts = FindOptions::new()
        .with_sort(vec![("n".to_string(), -1)])
        .with_skip(10)
        .with_limit(5)
        .with_max_response_bytes(500_000);
    let docs = coll.find_with_options(&json!({}), opts).unwrap();
    let ns: Vec<i64> = docs.iter().map(|d| d["n"].as_i64().unwrap()).collect();
    assert_eq!(ns, vec![189, 188, 187, 186, 185]);
}

/// M6: the hinted find path holds the same heap and has the same limit.
#[test]
fn find_with_hint_deep_skip_respects_byte_limit() {
    let db = db_with_large_docs();
    let coll = db.collection("c").unwrap();
    let index = coll.create_index("n".to_string(), false, false).unwrap();
    let opts = FindOptions::new()
        .with_sort(vec![("n".to_string(), 1)])
        .with_skip(190)
        .with_limit(5)
        .with_max_response_bytes(500_000);
    let err = coll
        .find_with_hint_ext(&json!({"n": {"$gte": 0}}), &index, opts)
        .unwrap_err();
    assert!(err.to_string().contains("byte limit"), "{err}");
}
