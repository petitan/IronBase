// batch_drop_rename_tests.rs
// Batch mode: inserts buffered for a collection must not outlive a drop or
// a rename of it (audit 2026-10-07 T7, follow-up).

use ironbase_core::durability::DurabilityMode;
use ironbase_core::storage::StorageEngine;
use ironbase_core::DatabaseCore;
use serde_json::{json, Value};
use std::collections::HashMap;
use tempfile::TempDir;

fn doc(n: i64) -> HashMap<String, Value> {
    HashMap::from([("n".to_string(), json!(n))])
}

fn open(path: &std::path::Path) -> DatabaseCore<StorageEngine> {
    DatabaseCore::<StorageEngine>::open_with_durability(path, DurabilityMode::Batch { batch_size: 1000 })
        .unwrap()
}

/// A drop takes the collection's buffered inserts with it.
#[test]
fn drop_discards_buffered_inserts() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("b.mlite");
    {
        let db = open(&path);
        db.insert_one("c", doc(1)).unwrap();
        db.flush().unwrap();
        db.insert_one("c", doc(2)).unwrap(); // buffered
        db.drop_collection("c").unwrap();
        db.flush().unwrap();
        assert!(
            !db.list_collections().contains(&"c".to_string()),
            "dropped collection came back: {:?}",
            db.list_collections()
        );
        db.close().unwrap();
    }
    let db = open(&path);
    assert!(!db.list_collections().contains(&"c".to_string()));
}

/// A rename takes the buffered inserts to the new name.
#[test]
fn rename_keeps_buffered_inserts_under_the_new_name() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("r.mlite");
    let db = open(&path);
    db.insert_one("a", doc(1)).unwrap();
    db.flush().unwrap();
    db.insert_one("a", doc(2)).unwrap(); // buffered
    db.rename_collection("a", "b").unwrap();
    db.flush().unwrap();
    assert!(!db.list_collections().contains(&"a".to_string()), "{:?}", db.list_collections());
    assert_eq!(db.find("b", &json!({})).unwrap().len(), 2);
}
