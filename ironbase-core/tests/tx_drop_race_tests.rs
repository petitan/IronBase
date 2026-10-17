// tx_drop_race_tests.rs
// A collection drop or rename must not run between an explicit transaction's
// buffered writes and its commit (WAL audit 2026-10-14 W4): the commit then
// logged its operations to the WAL, failed half-way while applying them, and
// crash recovery replayed them, bringing the dropped collection back.
// A crash is simulated by copying the database files (except the lock)
// while the database is still open.

use ironbase_core::DatabaseCore;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

fn doc(v: Value) -> HashMap<String, Value> {
    serde_json::from_value(v).unwrap()
}

fn crash_copy(src_dir: &Path, dst_dir: &Path) {
    for e in std::fs::read_dir(src_dir).unwrap() {
        let e = e.unwrap();
        let name = e.file_name().to_string_lossy().to_string();
        if !name.ends_with(".lock") {
            std::fs::copy(e.path(), dst_dir.join(&name)).unwrap();
        }
    }
}

#[test]
fn drop_waits_for_open_transaction() {
    let dir = tempfile::TempDir::new().unwrap();
    let crashed = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("w4.mlite");

    {
        let db = Arc::new(DatabaseCore::open(&path).unwrap());
        db.insert_one("users", doc(json!({"k": 0}))).unwrap();
        db.insert_one("logs", doc(json!({"k": 0}))).unwrap();

        let tx = db.begin_transaction();
        db.insert_one_tx("logs", doc(json!({"k": 1})), tx).unwrap();
        db.insert_one_tx("users", doc(json!({"k": 1})), tx).unwrap();

        let dropper = {
            let db = Arc::clone(&db);
            std::thread::spawn(move || db.drop_collection("users"))
        };
        // Give the drop time to run if nothing holds it off
        std::thread::sleep(Duration::from_millis(300));

        db.commit_transaction(tx)
            .expect("commit must not see the collection dropped under it");
        dropper.join().unwrap().unwrap();

        assert!(!db.list_collections().contains(&"users".to_string()));
        assert_eq!(
            db.collection("logs")
                .unwrap()
                .count_documents(&json!({}))
                .unwrap(),
            2
        );
        crash_copy(dir.path(), crashed.path());
    }

    let db = DatabaseCore::open(crashed.path().join("w4.mlite")).unwrap();
    assert!(
        !db.list_collections().contains(&"users".to_string()),
        "dropped collection came back after recovery"
    );
    assert_eq!(
        db.collection("logs")
            .unwrap()
            .count_documents(&json!({}))
            .unwrap(),
        2
    );
}

#[test]
fn rename_waits_for_open_transaction() {
    let dir = tempfile::TempDir::new().unwrap();
    let crashed = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("w4r.mlite");

    {
        let db = Arc::new(DatabaseCore::open(&path).unwrap());
        db.insert_one("users", doc(json!({"k": 0}))).unwrap();

        let tx = db.begin_transaction();
        db.insert_one_tx("users", doc(json!({"k": 1})), tx).unwrap();

        let renamer = {
            let db = Arc::clone(&db);
            std::thread::spawn(move || db.rename_collection("users", "people"))
        };
        std::thread::sleep(Duration::from_millis(300));

        db.commit_transaction(tx)
            .expect("commit must not see the collection renamed under it");
        renamer.join().unwrap().unwrap();
        crash_copy(dir.path(), crashed.path());
    }

    let db = DatabaseCore::open(crashed.path().join("w4r.mlite")).unwrap();
    assert!(
        !db.list_collections().contains(&"users".to_string()),
        "renamed collection came back under its old name after recovery"
    );
    assert_eq!(
        db.collection("people")
            .unwrap()
            .count_documents(&json!({}))
            .unwrap(),
        2
    );
}

#[test]
fn drop_rechecks_protection_after_waiting() {
    let dir = tempfile::TempDir::new().unwrap();
    let db = Arc::new(DatabaseCore::open(dir.path().join("w4p.mlite")).unwrap());
    db.insert_one("users", doc(json!({"k": 0}))).unwrap();

    let tx = db.begin_transaction();
    db.insert_one_tx("users", doc(json!({"k": 1})), tx).unwrap();

    let dropper = {
        let db = Arc::clone(&db);
        std::thread::spawn(move || db.drop_collection("users"))
    };
    std::thread::sleep(Duration::from_millis(300));

    // Protected while the drop waits for the transaction
    let flags = ironbase_core::storage::CollectionFlags {
        protected: true,
        ..Default::default()
    };
    db.set_collection_flags("users", flags).unwrap();
    db.commit_transaction(tx).unwrap();

    assert!(matches!(
        dropper.join().unwrap(),
        Err(ironbase_core::error::IronBaseError::OperationNotAllowed(_))
    ));
    assert_eq!(
        db.collection("users")
            .unwrap()
            .count_documents(&json!({}))
            .unwrap(),
        2
    );
}
