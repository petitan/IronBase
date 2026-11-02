// vector_metadata_tests.rs
// The persisted vector index metadata follows the index (#51).

use ironbase_core::storage::StorageEngine;
use ironbase_core::vector::VectorIndexConfig;
use ironbase_core::DatabaseCore;
use serde_json::{json, Value};
use std::collections::HashMap;

/// `vector_count` was written once at index creation (0 for an empty
/// collection) and never again: only `list_vector_indexes` corrected it from
/// the in-memory index. The checkpoint and the close now store the count.
#[test]
fn persisted_vector_count_follows_the_index() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v.mlite");
    let persisted = || {
        let storage = StorageEngine::open(&path).unwrap();
        storage.get_collection_meta("kb").unwrap().vector_indexes[0].vector_count
    };
    {
        let db = DatabaseCore::<StorageEngine>::open(&path).unwrap();
        let coll = db.collection("kb").unwrap();
        coll.create_vector_index("embedding", VectorIndexConfig::new(4))
            .unwrap();
        for i in 0..10 {
            let doc: HashMap<String, Value> =
                serde_json::from_value(json!({"n": i, "embedding": [1.0, i as f32, 0.0, 0.0]}))
                    .unwrap();
            db.insert_one("kb", doc).unwrap();
        }
        assert_eq!(coll.list_vector_indexes().unwrap()[0].vector_count, 10);
        db.checkpoint().unwrap();
        db.delete_many("kb", &json!({"n": {"$lt": 3}})).unwrap();
        db.close().unwrap();
    }
    assert_eq!(persisted(), 7);
}
