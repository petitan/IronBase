// transaction_custom_id_tests.rs
// insert_one_tx keeps a caller-supplied _id, like insert_one (it always
// assigned an auto id).

use ironbase_core::{DatabaseCore, StorageEngine};
use serde_json::{json, Value};
use std::collections::HashMap;

fn fields(v: Value) -> HashMap<String, Value> {
    v.as_object().unwrap().clone().into_iter().collect()
}

#[test]
fn insert_one_tx_keeps_supplied_id() {
    let tmp = tempfile::tempdir().unwrap();
    let db = DatabaseCore::<StorageEngine>::open(tmp.path().join("tx_id.mlite")).unwrap();

    let tx = db.begin_transaction();
    let id = db
        .insert_one_tx("c", fields(json!({"_id": "custom", "v": 1})), tx)
        .unwrap();
    db.insert_one_tx("c", fields(json!({"_id": 50, "v": 2})), tx)
        .unwrap();
    db.commit_transaction(tx).unwrap();
    assert_eq!(serde_json::to_value(&id).unwrap(), json!("custom"));

    let c = db.collection("c").unwrap();
    assert_eq!(c.find(&json!({"_id": "custom"})).unwrap().len(), 1);
    assert_eq!(c.find(&json!({"_id": 50})).unwrap().len(), 1);

    // A later auto id does not collide with the numeric one
    let auto = db.insert_one("c", fields(json!({"v": 3}))).unwrap();
    assert_ne!(serde_json::to_value(&auto).unwrap(), json!(50));

    // Duplicates are rejected: against a live document, and within the tx
    let tx = db.begin_transaction();
    assert!(db
        .insert_one_tx("c", fields(json!({"_id": "custom"})), tx)
        .is_err());
    db.insert_one_tx("c", fields(json!({"_id": "fresh"})), tx)
        .unwrap();
    assert!(db
        .insert_one_tx("c", fields(json!({"_id": "fresh"})), tx)
        .is_err());
    db.rollback_transaction(tx).unwrap();
    assert_eq!(c.find(&json!({"_id": "fresh"})).unwrap().len(), 0);
}
