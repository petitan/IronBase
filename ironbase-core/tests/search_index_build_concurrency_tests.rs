// search_index_build_concurrency_tests.rs
// Fuzzy, fulltext and vector index builds while other threads update, delete
// and insert: the finished index must hold exactly the live documents' current
// values, with no phantom (deleted), stale (pre-update), duplicate or missing
// entries. The B+ tree twin is index_build_concurrency_tests.rs (audit
// 2026-10-07 T2).

use ironbase_core::storage::MemoryStorage;
use ironbase_core::{DatabaseCore, DocumentId, FuzzyAlgorithm, VectorIndexConfig};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};

const DOCS: i64 = 1500;
const ROUNDS: i64 = 3;

fn fields(v: Value) -> HashMap<String, Value> {
    v.as_object().unwrap().clone().into_iter().collect()
}

fn seed(db: &DatabaseCore<MemoryStorage>) {
    for i in 0..DOCS {
        db.insert_one(
            "c",
            fields(
                json!({"_id": i, "name": "alpha", "text": "alpha", "vec": [1.0, 0.0, 0.0, 0.0]}),
            ),
        )
        .unwrap();
    }
}

/// Runs `build` while a writer updates ("alpha" -> "beta"), deletes and
/// inserts ("gamma") documents.
fn build_under_writes(
    db: &DatabaseCore<MemoryStorage>,
    round: i64,
    build: impl FnOnce() -> ironbase_core::Result<String>,
) {
    let done = AtomicBool::new(false);
    std::thread::scope(|s| {
        s.spawn(|| {
            let mut k: i64 = 0;
            let mut next = DOCS;
            while !done.load(Ordering::SeqCst) {
                let id = (k * 7919 + round) % DOCS;
                match k % 3 {
                    0 => {
                        let _ = db.update_one(
                            "c",
                            &json!({"_id": id}),
                            &json!({"$set": {"name": "beta", "text": "beta", "vec": [0.0, 1.0, 0.0, 0.0]}}),
                        );
                    }
                    1 => {
                        let _ = db.delete_one("c", &json!({"_id": id}));
                    }
                    _ => {
                        db.insert_one(
                            "c",
                            fields(json!({"_id": next, "name": "gamma", "text": "gamma", "vec": [0.0, 0.0, 1.0, 0.0]})),
                        )
                        .unwrap();
                        next += 1;
                    }
                }
                k += 1;
            }
        });
        let built = build();
        done.store(true, Ordering::SeqCst);
        built.expect("index build failed under concurrent writes");
    });
}

/// Live documents grouped by the value of `field`.
fn live_by_value(db: &DatabaseCore<MemoryStorage>, field: &str) -> HashMap<String, HashSet<i64>> {
    let mut by_value: HashMap<String, HashSet<i64>> = HashMap::new();
    for d in db.collection("c").unwrap().find(&json!({})).unwrap() {
        let key = match &d[field] {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        by_value
            .entry(key)
            .or_default()
            .insert(d["_id"].as_i64().unwrap());
    }
    by_value
}

/// Compares id sets, reporting only the differences.
fn assert_same_ids(actual: &HashSet<i64>, expected: &HashSet<i64>, what: &str) {
    let extra: Vec<_> = actual.difference(expected).take(10).collect();
    let missing: Vec<_> = expected.difference(actual).take(10).collect();
    assert!(
        extra.is_empty() && missing.is_empty(),
        "{what}: {} extra (e.g. {extra:?}), {} missing (e.g. {missing:?})",
        actual.difference(expected).count(),
        expected.difference(actual).count()
    );
}

fn int_ids(ids: impl IntoIterator<Item = DocumentId>) -> HashSet<i64> {
    ids.into_iter()
        .map(|id| match id {
            DocumentId::Int(i) => i,
            other => panic!("unexpected id {other:?}"),
        })
        .collect()
}

#[test]
fn fuzzy_index_build_concurrent_with_writers_is_consistent() {
    for round in 0..ROUNDS {
        let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
        seed(&db);
        let coll = db.collection("c").unwrap();
        build_under_writes(&db, round, || {
            coll.create_fuzzy_index("name".to_string(), FuzzyAlgorithm::Levenshtein, 0.8)
        });

        let live = live_by_value(&db, "name");
        let indexes = coll.indexes.read();
        let index = indexes.get_fuzzy_index("c_name_fuzzy").unwrap();
        let live_count: usize = live.values().map(HashSet::len).sum();
        assert_eq!(
            index.entry_count(),
            live_count,
            "round {round}: fuzzy entries != live documents"
        );
        for (value, ids) in &live {
            let hits = int_ids(index.search(value, Some(1.0)).into_iter().map(|(id, _)| id));
            assert_same_ids(
                &hits,
                ids,
                &format!("round {round}: fuzzy entries for {value:?}"),
            );
        }
    }
}

#[test]
fn fulltext_index_build_concurrent_with_writers_is_consistent() {
    for round in 0..ROUNDS {
        let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
        seed(&db);
        let coll = db.collection("c").unwrap();
        build_under_writes(&db, round, || {
            coll.create_fulltext_index("text".to_string(), "none", None, None)
        });

        let live = live_by_value(&db, "text");
        let indexes = coll.indexes.read();
        let index = indexes.get_fulltext_index("c_text_fts").unwrap();
        let live_count: usize = live.values().map(HashSet::len).sum();
        assert_eq!(
            index.doc_count(),
            live_count,
            "round {round}: fulltext documents != live documents"
        );
        for word in ["alpha", "beta", "gamma"] {
            let token = index.tokenize_query(word).remove(0);
            let hits = int_ids(index.token_chunk_ids(&token));
            let expected = live.get(word).cloned().unwrap_or_default();
            assert_same_ids(
                &hits,
                &expected,
                &format!("round {round}: fulltext postings for {word:?}"),
            );
        }
    }
}

#[test]
fn vector_index_build_concurrent_with_writers_is_consistent() {
    for round in 0..ROUNDS {
        let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
        seed(&db);
        let coll = db.collection("c").unwrap();
        build_under_writes(&db, round, || {
            coll.create_vector_index("vec", VectorIndexConfig::new(4))
        });

        let live = live_by_value(&db, "vec");
        let indexes = coll.indexes.read();
        let index = indexes.get_vector_index("c_vec_vec").unwrap();
        let live_count: usize = live.values().map(HashSet::len).sum();
        assert_eq!(
            index.len(),
            live_count,
            "round {round}: vector nodes != live documents"
        );
        for (value, ids) in &live {
            let expected: Vec<f32> = serde_json::from_str(value).unwrap();
            for id in ids {
                assert_eq!(
                    index.get_vector(&format!("i:{id}")),
                    Some(expected.as_slice()),
                    "round {round}: vector of document {id}"
                );
            }
        }
    }
}
