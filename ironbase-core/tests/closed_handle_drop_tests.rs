// closed_handle_drop_tests.rs
// A handle that was closed and dropped only after the database was reopened
// must not write to the file (Python audit 2026-10-14 P1: the GC can free a
// closed `IronBase` after a new one opened the same path).

use ironbase_core::DatabaseCore;
use serde_json::{json, Value};
use std::collections::HashMap;

fn doc(v: Value) -> HashMap<String, Value> {
    serde_json::from_value(v).unwrap()
}

#[test]
fn dropping_closed_handle_after_reopen_keeps_new_writes() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("p1.mlite");

    let old = DatabaseCore::open(&path).unwrap();
    old.insert_one("c", doc(json!({"k": 1}))).unwrap();
    old.close().unwrap();

    // Reopened while the old (closed) handle is still alive
    let new = DatabaseCore::open(&path).unwrap();
    new.insert_one("c", doc(json!({"k": 2}))).unwrap();
    new.insert_one("d", doc(json!({"k": 3}))).unwrap();
    new.close().unwrap();
    drop(new);

    // The GC frees the old handle last
    drop(old);

    let db = DatabaseCore::open(&path).unwrap();
    let c = db.collection("c").unwrap();
    assert_eq!(
        c.count_documents(&json!({})).unwrap(),
        2,
        "writes of the reopened database lost"
    );
    assert_eq!(c.find(&json!({"k": 2})).unwrap().len(), 1);
    assert!(db.list_collections().contains(&"d".to_string()));
}
