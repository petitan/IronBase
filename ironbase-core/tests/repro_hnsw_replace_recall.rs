//! Regression tests for #77 / #53: HNSW search recall must survive repeated
//! replace (delete + re-insert) of vectors.
//!
//! Root cause: `remove()` is lazy — a removed node stays in the graph (still in
//! `nodes` and reachable via edges) but is dropped from `id_to_index`. The old
//! `search_layer` added those orphans to the `ef`-capped result set, so an
//! orphan-dense neighbourhood (a chunk replaced many times — exactly the RAG
//! `if_exists="replace"` path) starved the result budget and search recall
//! collapsed (#77: 86% self-retrieval miss in production). Deleted vectors could
//! also keep surfacing until a rebuild (#53).
//!
//! Fix: orphans stay navigable (frontier) but never enter the result/`ef` set,
//! so search keeps expanding until it has `ef` *active* hits.

use ironbase_core::database::DatabaseCore;
use ironbase_core::storage::MemoryStorage;
use ironbase_core::vector::{DistanceMetric, VectorIndexConfig};
use serde_json::json;
use std::collections::HashMap;

fn embedding_doc(tag: &str, vec: &[f32]) -> HashMap<String, serde_json::Value> {
    HashMap::from([
        ("doc_id".to_string(), json!(tag)),
        ("embedding".to_string(), json!(vec)),
    ])
}

fn make_index(coll: &ironbase_core::collection_core::CollectionCore<MemoryStorage>, dim: usize) {
    let mut cfg = VectorIndexConfig::new(dim);
    cfg.field = "embedding".to_string();
    cfg.metric = DistanceMetric::Cosine;
    coll.create_vector_index("embedding", cfg).unwrap();
}

/// One vector replaced 80×: the orphan-dense neighbourhood must not hide the
/// live anchor from a self-query.
#[test]
fn replace_same_vector_many_times_self_retrieval_holds() {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    let coll = db.collection("kb").unwrap();
    make_index(&coll, 8);

    let anchor: Vec<f32> = vec![1.0, 0.5, 0.25, 0.125, 0.0, 0.0, 0.0, 0.0];

    for i in 0..10 {
        let mut v = vec![0.0f32; 8];
        v[i % 8] = 1.0;
        v[(i + 3) % 8] = 0.3;
        db.insert_many("kb", vec![embedding_doc(&format!("other{i}"), &v)])
            .unwrap();
    }

    let mut current_ids = db
        .insert_many("kb", vec![embedding_doc("anchor", &anchor)])
        .unwrap();
    for _ in 0..80 {
        let new_ids = db
            .insert_many("kb", vec![embedding_doc("anchor", &anchor)])
            .unwrap();
        let old: Vec<serde_json::Value> = current_ids.iter().map(|id| json!(id)).collect();
        db.delete_many("kb", &json!({ "_id": { "$in": old } }))
            .unwrap();
        current_ids = new_ids;
    }

    let results = coll.vector_search("embedding", &anchor, 1).unwrap();
    assert!(
        !results.is_empty(),
        "self-retrieval returned nothing for the anchor after replaces (orphan poisoning)"
    );
    assert_eq!(
        results[0].0.get("doc_id").and_then(|v| v.as_str()),
        Some("anchor"),
        "top hit for the anchor embedding should be the anchor itself"
    );
}

/// Production-like: many docs, each replaced several times. Global self-retrieval
/// recall must stay perfect (this is the 1025-chunk / 86%-miss scenario at scale).
#[test]
fn global_recall_after_many_replaces() {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    let coll = db.collection("kb").unwrap();
    const DIM: usize = 48;
    const DOCS: usize = 40;
    make_index(&coll, DIM);

    // Each doc gets a clearly distinct embedding: a unique dominant dimension
    // (d < DIM ⇒ position d is unique) plus two smaller offset components.
    let embedding = |d: usize| -> Vec<f32> {
        let mut v = vec![0.0f32; DIM];
        v[d % DIM] = 1.0;
        v[(d + 1) % DIM] = 0.3;
        v[(d + 2) % DIM] = 0.1;
        v
    };

    // Initial import.
    let mut current: Vec<Vec<serde_json::Value>> = Vec::with_capacity(DOCS);
    for d in 0..DOCS {
        let ids = db
            .insert_many("kb", vec![embedding_doc(&format!("doc{d}"), &embedding(d))])
            .unwrap();
        current.push(ids.into_iter().map(|id| json!(id)).collect());
    }

    // 5 re-enrich rounds: replace every doc (delete old + insert new).
    for _round in 0..5 {
        for (d, ids) in current.iter_mut().enumerate() {
            let new_ids = db
                .insert_many("kb", vec![embedding_doc(&format!("doc{d}"), &embedding(d))])
                .unwrap();
            db.delete_many("kb", &json!({ "_id": { "$in": ids.clone() } }))
                .unwrap();
            *ids = new_ids.into_iter().map(|id| json!(id)).collect();
        }
    }

    // Every doc must retrieve itself as the top hit.
    let mut hits = 0;
    for d in 0..DOCS {
        let res = coll.vector_search("embedding", &embedding(d), 1).unwrap();
        if res
            .first()
            .and_then(|(doc, _)| doc.get("doc_id"))
            .and_then(|v| v.as_str())
            == Some(&format!("doc{d}"))
        {
            hits += 1;
        }
    }
    assert_eq!(
        hits, DOCS,
        "global self-retrieval recall must be 100% after replaces, got {hits}/{DOCS}"
    );
}

/// #53 correctness: a deleted vector must not be returned by search.
#[test]
fn deleted_vector_not_returned() {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    let coll = db.collection("kb").unwrap();
    make_index(&coll, 8);

    let target: Vec<f32> = vec![0.9, 0.1, 0.0, 0.0, 0.2, 0.0, 0.0, 0.0];
    for i in 0..5 {
        let mut v = vec![0.05f32; 8];
        v[i] = 1.0;
        db.insert_many("kb", vec![embedding_doc(&format!("keep{i}"), &v)])
            .unwrap();
    }
    let target_ids = db
        .insert_many("kb", vec![embedding_doc("victim", &target)])
        .unwrap();

    // Present before delete.
    let before = coll.vector_search("embedding", &target, 1).unwrap();
    assert_eq!(
        before[0].0.get("doc_id").and_then(|v| v.as_str()),
        Some("victim")
    );

    // Delete and confirm it never resurfaces.
    let ids: Vec<serde_json::Value> = target_ids.iter().map(|id| json!(id)).collect();
    db.delete_many("kb", &json!({ "_id": { "$in": ids } }))
        .unwrap();

    let after = coll.vector_search("embedding", &target, 5).unwrap();
    assert!(
        after
            .iter()
            .all(|(doc, _)| doc.get("doc_id").and_then(|v| v.as_str()) != Some("victim")),
        "deleted vector must not be returned by search"
    );
}
