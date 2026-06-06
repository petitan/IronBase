//! Review follow-up regression — streaming/cursor numeric IndexRangeScan (finding D).
//!
//! PR5 applied the Int/Float two-bucket scan to the non-streaming
//! collect_doc_ids_from_plan path only. The streaming cursor
//! (FindCursor::new_index_scan_from_plan, used by find_streaming and aggregation
//! $match) kept a single contiguous range scan and silently dropped Float-keyed
//! docs for bounded numeric ranges. The cursor now falls back to the
//! bucket-aware non-streaming path for numeric ranges.
//!
//! Requires file-backed storage: MemoryStorage early-returns find_streaming to
//! the (already-fixed) doc_ids path, so the cursor bug only surfaces on a real
//! .mlite file.

use ironbase_core::StorageEngine;
use serde_json::json;
use std::collections::HashMap;
use tempfile::TempDir;

type DatabaseCore = ironbase_core::DatabaseCore<StorageEngine>;

fn insert(db: &DatabaseCore, coll: &str, doc: serde_json::Value) {
    if let serde_json::Value::Object(m) = doc {
        db.insert_one(coll, m.into_iter().collect::<HashMap<_, _>>())
            .unwrap();
    }
}

#[test]
fn streaming_numeric_range_spans_int_and_float() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("stream_numeric.mlite");
    let db = DatabaseCore::open(&path).unwrap();
    for v in [json!(20), json!(30.5), json!(40), json!(50.0)] {
        insert(&db, "items", json!({ "age": v }));
    }
    let coll = db.collection("items").unwrap();
    coll.create_index("age".to_string(), false, false).unwrap();

    let q = json!({"age": {"$gte": 18, "$lt": 65}});

    // Non-streaming find (already-fixed path) sees all 4.
    let find_n = coll.find(&q).unwrap().len();
    assert_eq!(find_n, 4, "find() should see all 4 (incl. Float docs)");

    // Streaming cursor must agree — previously dropped 30.5 and 50.0.
    let mut cursor = coll.find_streaming(&q).unwrap();
    let mut stream_n = 0;
    while cursor.next().unwrap().is_some() {
        stream_n += 1;
    }
    assert_eq!(
        stream_n, find_n,
        "streaming cursor must match find() — Float-keyed docs not dropped"
    );
}

#[test]
fn streaming_half_bounded_and_exclusive() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("stream_numeric2.mlite");
    let db = DatabaseCore::open(&path).unwrap();
    for v in [json!(-5), json!(-2.5), json!(0), json!(3.5), json!(10)] {
        insert(&db, "items", json!({ "age": v }));
    }
    let coll = db.collection("items").unwrap();
    coll.create_index("age".to_string(), false, false).unwrap();

    for q in [
        json!({"age": {"$gte": 0}}),
        json!({"age": {"$lt": 3.5}}),
        json!({"age": {"$gt": -5, "$lte": 3.5}}),
    ] {
        let find_n = coll.find(&q).unwrap().len();
        let mut cursor = coll.find_streaming(&q).unwrap();
        let mut stream_n = 0;
        while cursor.next().unwrap().is_some() {
            stream_n += 1;
        }
        assert_eq!(stream_n, find_n, "streaming must match find for {q}");
    }
}
