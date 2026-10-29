//! Hybrid retrieval in the engine: `CollectionCore::hybrid_fuse` and
//! `hybrid_group_documents`. The parameter-level behaviors are covered by the
//! mcp-server pipeline tests, which run on these functions.

use ironbase_core::search::hybrid::HybridQuery;
use ironbase_core::storage::MemoryStorage;
use ironbase_core::vector::{DistanceMetric, VectorIndexConfig};
use ironbase_core::{CollectionCore, DatabaseCore};
use serde_json::{json, Value};
use std::collections::HashMap;

fn insert(db: &DatabaseCore<MemoryStorage>, doc: Value) {
    let map: HashMap<String, Value> = serde_json::from_value(doc).unwrap();
    db.insert_one("kb", map).unwrap();
}

/// Three documents: "lex" matches the words, "sem" only the vector, "both" both.
fn kb() -> (DatabaseCore<MemoryStorage>, CollectionCore<MemoryStorage>) {
    let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
    for (doc_id, idx, content, emb) in [
        (
            "lex",
            0,
            "Maintenance schedule for printers.",
            [0.0, 1.0, 0.0, 0.0],
        ),
        (
            "sem",
            0,
            "The automobile needs periodic servicing.",
            [1.0, 0.0, 0.0, 0.0],
        ),
        (
            "both",
            0,
            "Car maintenance checklist.",
            [0.9, 0.1, 0.0, 0.0],
        ),
        ("both", 1, "Tyre pressure table.", [0.0, 0.0, 1.0, 0.0]),
    ] {
        insert(
            &db,
            json!({"doc_id": doc_id, "chunk_index": idx, "content": content, "embedding": emb}),
        );
    }
    let coll = db.collection("kb").unwrap();
    coll.create_fulltext_index("content".to_string(), "none", None, None)
        .unwrap();
    coll.create_vector_index(
        "embedding",
        VectorIndexConfig::new(4).with_metric(DistanceMetric::Cosine),
    )
    .unwrap();
    (db, coll)
}

fn query(text: &str, vector: Option<Vec<f32>>) -> HybridQuery {
    HybridQuery {
        query: text.to_string(),
        vector,
        vector_field: "embedding".to_string(),
        searched_vector_field: None,
        text_field: "content".to_string(),
        text_fields: None,
        vector_weight: 0.5,
        fulltext_weight: 0.5,
        rrf_k: 20.0,
        limit: 10,
        filter: None,
        and_mode: false,
        document_scope: true,
        rerank: false,
        title_field: None,
        max_chunks_per_doc: None,
        merge_chunks: false,
        deduplicate: false,
        mmr_lambda: 0.7,
        group_by_document: false,
        projection: None,
    }
}

fn doc_ids(fused: &[ironbase_core::search::hybrid::FusedResult]) -> Vec<String> {
    fused
        .iter()
        .map(|f| f.doc["doc_id"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn both_lanes_are_fused_and_a_document_in_both_ranks_first() {
    let (_db, coll) = kb();
    let fusion = coll
        .hybrid_fuse(
            &query("car maintenance", Some(vec![1.0, 0.0, 0.0, 0.0])),
            None,
        )
        .unwrap();
    let ids = doc_ids(&fusion.fused);
    assert_eq!(ids[0], "both", "{ids:?}");
    assert!(
        ids.contains(&"sem".to_string()),
        "vector-only document lost: {ids:?}"
    );
    assert!(
        ids.contains(&"lex".to_string()),
        "fulltext-only document lost: {ids:?}"
    );
    let first = &fusion.fused[0];
    assert!(first.v_rank > 0 && first.t_rank > 0);
    // weight / (rrf_k + rank), summed over the lanes that returned it
    let expected = 0.5 / (20.0 + first.v_rank as f64) + 0.5 / (20.0 + first.t_rank as f64);
    assert!((first.rrf_score - expected).abs() < 1e-12);
}

#[test]
fn without_a_vector_only_the_fulltext_lane_runs() {
    let (_db, coll) = kb();
    let fusion = coll.hybrid_fuse(&query("maintenance", None), None).unwrap();
    let mut ids = doc_ids(&fusion.fused);
    ids.sort();
    assert_eq!(ids, vec!["both", "lex"]);
    assert!(fusion
        .fused
        .iter()
        .all(|f| f.v_rank == 0 && f.v_score.is_none()));
}

#[test]
fn document_level_and_qualifies_across_chunks() {
    let (_db, coll) = kb();
    let mut q = query("maintenance tyre", None);
    q.and_mode = true;
    let fusion = coll.hybrid_fuse(&q, None).unwrap();
    assert_eq!(fusion.qualification.qualified_doc_count, Some(1));
    assert!(!fusion.qualification.effective_and_mode);
    // Both chunks of "both" are retrieved in OR mode inside the document
    assert_eq!(doc_ids(&fusion.fused), vec!["both", "both"]);

    q.document_scope = false; // match_scope = "chunk": no chunk holds both words
    let fusion = coll.hybrid_fuse(&q, None).unwrap();
    assert!(fusion.fused.is_empty());
}

#[test]
fn grouping_keeps_the_vector_lane_documents_and_scores_each_chunk() {
    let (_db, coll) = kb();
    let mut q = query("car maintenance", Some(vec![1.0, 0.0, 0.0, 0.0]));
    q.group_by_document = true;
    q.limit = 3;
    let fusion = coll.hybrid_fuse(&q, None).unwrap();
    let (groups, total) = coll
        .hybrid_group_documents(&fusion.fused, &q, None)
        .unwrap();
    let order: Vec<&str> = groups.iter().map(|g| g.doc_id.as_str()).collect();
    assert_eq!(order[0], "both");
    assert!(
        order.contains(&"sem"),
        "vector-only document dropped: {order:?}"
    );
    assert_eq!(total, groups.iter().map(|g| g.chunks.len()).sum::<usize>());
    let first_chunk = &groups[0].chunks[0];
    assert!(first_chunk.get("_rrf_score").is_some());
    assert!(first_chunk.get("_vector_rank").is_some());
}
