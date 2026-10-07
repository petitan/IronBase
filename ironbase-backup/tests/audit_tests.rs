//! Regression tests for audit 2026-10-06 #31, #38, #39, #40 against real
//! IronBase databases (library API).

use ironbase_backup::{create_backup, restore, verify_backup, BackupError};
use ironbase_core::storage::StorageEngine;
use ironbase_core::DatabaseCore;
use serde_json::json;
use std::collections::HashMap;
use std::path::Path;
use tempfile::TempDir;

fn doc(i: i64) -> HashMap<String, serde_json::Value> {
    HashMap::from([
        ("_id".to_string(), json!(i)),
        ("pad".to_string(), json!("x".repeat(200))),
    ])
}

fn insert(db: &DatabaseCore<StorageEngine>, ids: std::ops::Range<i64>) {
    for i in ids {
        db.insert_one("c", doc(i)).unwrap();
    }
}

fn count_in(path: &Path) -> u64 {
    let db = DatabaseCore::<StorageEngine>::open(path).unwrap();
    db.count_documents("c", &json!({})).unwrap()
}

/// #31: after a compaction the next backup must not be an incremental on top
/// of the pre-compaction chain.
#[test]
fn incremental_after_compaction_is_rejected() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("db.mlite");
    let backups = dir.path().join("backups");
    std::fs::create_dir(&backups).unwrap();

    let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
    insert(&db, 0..300);
    db.delete_many("c", &json!({"_id": {"$lt": 200}})).unwrap();
    db.checkpoint().unwrap();
    create_backup(&db_path, &backups, false, None).unwrap();

    db.compact().unwrap();
    insert(&db, 1000..1600); // grows past the previous size again
    db.checkpoint().unwrap();

    match create_backup(&db_path, &backups, false, None) {
        Err(BackupError::LayoutChanged) => {}
        other => panic!("expected LayoutChanged, got {:?}", other.map(|r| r.path)),
    }
    // A forced full backup still works and restores correctly. Chains pick
    // the newest full backup by its (whole-second) timestamp.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    create_backup(&db_path, &backups, true, None).unwrap();
    drop(db);
    let restored = dir.path().join("restored.mlite");
    restore(&backups, &restored, None, Some("db")).unwrap();
    assert_eq!(count_in(&restored), 700);
}

/// #38 + #40: a chain of full + incremental (one without new data) restores
/// to a consistent database.
#[test]
fn full_and_incremental_restore_consistent_database() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("db.mlite");
    let backups = dir.path().join("backups");
    std::fs::create_dir(&backups).unwrap();

    let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
    insert(&db, 0..100);
    db.checkpoint().unwrap();
    create_backup(&db_path, &backups, false, None).unwrap();
    insert(&db, 100..150);
    db.checkpoint().unwrap();
    create_backup(&db_path, &backups, false, None).unwrap();
    // No new data: payload is just the DB header
    create_backup(&db_path, &backups, false, None).unwrap();
    let source_header = std::fs::read(&db_path).unwrap()[..256].to_vec();
    db.close().unwrap();
    drop(db);

    let restored = dir.path().join("restored.mlite");
    restore(&backups, &restored, None, Some("db")).unwrap();
    let copy = std::fs::read(&restored).unwrap();
    assert_eq!(
        copy[..256],
        source_header[..],
        "restored header must be the last backup's header"
    );
    assert_eq!(count_in(&restored), 150);
}

/// #39: a corrupt later part of a multi-part backup must fail verification.
#[test]
fn corrupt_later_part_fails_verification() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("db.mlite");
    let backups = dir.path().join("backups");
    std::fs::create_dir(&backups).unwrap();

    let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    for i in 0..400 {
        // Incompressible padding (xorshift) so the backup really splits
        let pad: String = (0..300)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                char::from_digit((state % 16) as u32, 16).unwrap()
            })
            .collect();
        db.insert_one(
            "c",
            HashMap::from([
                ("_id".to_string(), json!(i)),
                ("pad".to_string(), json!(pad)),
            ]),
        )
        .unwrap();
    }
    db.checkpoint().unwrap();
    let result = create_backup(&db_path, &backups, false, Some(16 * 1024)).unwrap();
    assert!(
        result.part_count >= 2,
        "backup must be split: {}",
        result.part_count
    );

    assert!(verify_backup(&result.all_paths[0]).unwrap().valid);
    // Flip a byte in the payload of part 2
    let part2 = &result.all_paths[1];
    let mut bytes = std::fs::read(part2).unwrap();
    let mid = bytes.len() / 2;
    bytes[mid] ^= 0xFF;
    std::fs::write(part2, bytes).unwrap();

    let check = verify_backup(&result.all_paths[0]).unwrap();
    assert!(!check.valid, "corrupt part 2 passed verification");
}

fn ids_in(path: &Path) -> Vec<i64> {
    let db = DatabaseCore::<StorageEngine>::open(path).unwrap();
    let mut ids: Vec<i64> = db
        .collection("c")
        .unwrap()
        .find(&json!({}))
        .unwrap()
        .iter()
        .map(|d| d["_id"].as_i64().unwrap())
        .collect();
    ids.sort();
    ids
}

/// #32: writes since the last checkpoint live only in the WAL; a hot backup
/// must carry it so the restored database has them (insert kept, delete not
/// undone).
#[test]
fn hot_backup_includes_writes_since_checkpoint() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("db.mlite");
    let backups = dir.path().join("backups");
    std::fs::create_dir(&backups).unwrap();

    let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
    insert(&db, 0..5);
    db.checkpoint().unwrap();
    insert(&db, 100..101);
    db.delete_one("c", &json!({"_id": 2})).unwrap();

    create_backup(&db_path, &backups, true, None).unwrap();
    let restored = dir.path().join("restored.mlite");
    restore(&backups, &restored, None, Some("db")).unwrap();
    assert_eq!(ids_in(&restored), vec![0, 1, 3, 4, 100]);

    // An incremental with no checkpoint in between copies no new file bytes;
    // the new writes come only from its WAL.
    insert(&db, 200..202);
    std::thread::sleep(std::time::Duration::from_millis(1100));
    create_backup(&db_path, &backups, false, None).unwrap();
    let restored2 = dir.path().join("restored2.mlite");
    restore(&backups, &restored2, None, Some("db")).unwrap();
    assert_eq!(ids_in(&restored2), vec![0, 1, 3, 4, 100, 200, 201]);
}

/// #32: the WAL section is stored in part 1 of a multi-part backup.
#[test]
fn multipart_hot_backup_restores_wal() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("db.mlite");
    let backups = dir.path().join("backups");
    std::fs::create_dir(&backups).unwrap();

    let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
    for i in 0..200 {
        let mut d = doc(i);
        // incompressible padding so the payload spans several parts
        let mut x = (i as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let noise: String = (0..300)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                char::from(b'a' + (x % 26) as u8)
            })
            .collect();
        d.insert("pad".to_string(), json!(noise));
        db.insert_one("c", d).unwrap();
    }
    db.checkpoint().unwrap();
    insert(&db, 500..510);

    let result = create_backup(&db_path, &backups, true, Some(16 * 1024)).unwrap();
    assert!(result.part_count >= 2);
    let restored = dir.path().join("restored.mlite");
    restore(&backups, &restored, None, Some("db")).unwrap();
    assert_eq!(count_in(&restored), 210);
}

/// #32: a WAL already at the restore target belongs to another database
/// state and must not be replayed into the restored file.
#[test]
fn restore_removes_foreign_wal_at_target() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("db.mlite");
    let backups = dir.path().join("backups");
    std::fs::create_dir(&backups).unwrap();
    {
        let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
        insert(&db, 0..3);
        db.close().unwrap();
    }
    create_backup(&db_path, &backups, true, None).unwrap();

    // A WAL from another database with a committed insert of _id 99
    let other = dir.path().join("other.mlite");
    {
        let db = DatabaseCore::<StorageEngine>::open(&other).unwrap();
        insert(&db, 0..3);
        db.checkpoint().unwrap();
        insert(&db, 99..100);
        // "Crash": keep the WAL (Drop would checkpoint and clear it)
        std::mem::forget(db);
    }
    let restored = dir.path().join("restored.mlite");
    std::fs::copy(other.with_extension("wal"), restored.with_extension("wal")).unwrap();

    restore(&backups, &restored, None, Some("db")).unwrap();
    assert_eq!(ids_in(&restored), vec![0, 1, 2]);
}
