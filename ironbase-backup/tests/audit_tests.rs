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
