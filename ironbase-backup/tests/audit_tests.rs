//! Regression tests for audit 2026-10-06 #31, #38, #39, #40 and the P10
//! backup property against real IronBase databases (library API).

use ironbase_backup::{create_backup, restore, verify_backup, BackupError, BackupType};
use ironbase_core::storage::StorageEngine;
use ironbase_core::vector::VectorIndexConfig;
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
    // A forced full backup still works and restores correctly; in the same
    // second as the previous backup too (P10)
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

/// A checkpoint of another database interrupted while trimming its WAL left
/// its retained entries next to the target; opening the restored file must
/// not restore them over the backup's WAL (WAL/recovery audit 2026-10-14 W7).
#[test]
fn restore_removes_foreign_retained_wal_at_target() {
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

    let other = dir.path().join("other.mlite");
    {
        let db = DatabaseCore::<StorageEngine>::open(&other).unwrap();
        insert(&db, 0..3);
        db.checkpoint().unwrap();
        insert(&db, 99..100);
        std::mem::forget(db);
    }
    let restored = dir.path().join("restored.mlite");
    for ext in ["wal.retain", "wal.keep"] {
        std::fs::copy(other.with_extension("wal"), restored.with_extension(ext)).unwrap();
    }

    restore(&backups, &restored, None, Some("db")).unwrap();
    assert_eq!(ids_in(&restored), vec![0, 1, 2]);
}

fn insert_incompressible(db: &DatabaseCore<StorageEngine>, n: i64) {
    let mut state: u64 = 0x2545_F491_4F6C_DD1D;
    for i in 0..n {
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
}

/// Review follow-up on #39: a valid part of a different backup (own hash
/// fine) must not pass as part of this one; a later part verified on its
/// own is checked against its own hash only.
#[test]
fn substituted_part_from_other_backup_fails_verification() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("db.mlite");
    let backups_a = dir.path().join("a");
    let backups_b = dir.path().join("b");
    std::fs::create_dir(&backups_a).unwrap();
    std::fs::create_dir(&backups_b).unwrap();

    let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
    insert_incompressible(&db, 400);
    db.checkpoint().unwrap();
    let a = create_backup(&db_path, &backups_a, true, Some(16 * 1024)).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let b = create_backup(&db_path, &backups_b, true, Some(16 * 1024)).unwrap();
    assert!(a.part_count >= 2 && a.part_count == b.part_count);

    assert!(verify_backup(&a.all_paths[1]).unwrap().valid);
    assert!(verify_backup(&b.all_paths[0]).unwrap().valid);

    std::fs::copy(&a.all_paths[1], &b.all_paths[1]).unwrap();
    assert!(
        !verify_backup(&b.all_paths[0]).unwrap().valid,
        "part 2 of another backup passed verification"
    );
}

/// P10: a full backup taken in the same second as the previous one starts
/// the chain; the older chain must not be restored instead
#[test]
fn full_backups_in_one_second_newest_wins() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("db.mlite");
    let backups = dir.path().join("backups");
    let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
    insert(&db, 0..10);
    create_backup(&db_path, &backups, true, None).unwrap();
    insert(&db, 10..20);
    create_backup(&db_path, &backups, false, None).unwrap();
    insert(&db, 20..30);
    let newest = create_backup(&db_path, &backups, true, None).unwrap();
    db.close().unwrap();
    drop(db);

    let restored = dir.path().join("restored.mlite");
    let name = newest.path.file_name().unwrap().to_str().unwrap();
    restore(&backups, &restored, Some(name), Some("db")).unwrap();
    assert_eq!(count_in(&restored), 30);
    std::fs::remove_file(&restored).unwrap();
    restore(&backups, &restored, None, Some("db")).unwrap();
    assert_eq!(count_in(&restored), 30);
}

/// P10: an incremental backup after a split backup decoded the previous
/// backup's database header from its first part alone (a zstd block can
/// span parts) and failed with "incomplete frame"
#[test]
fn incremental_after_split_backup() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("db.mlite");
    let backups = dir.path().join("backups");
    let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
    insert_incompressible(&db, 300);
    db.checkpoint().unwrap();
    let full = create_backup(&db_path, &backups, true, Some(16 * 1024)).unwrap();
    assert!(full.part_count >= 2);
    db.insert_one("c", doc(1000)).unwrap();
    let incr = create_backup(&db_path, &backups, false, None).unwrap();
    assert_eq!(incr.backup_type, BackupType::Incremental);
    db.close().unwrap();
    drop(db);

    let restored = dir.path().join("restored.mlite");
    restore(&backups, &restored, None, Some("db")).unwrap();
    assert_eq!(count_in(&restored), 301);
}

/// P10: a backup taken right after a compaction (before any checkpoint)
/// stored the old `last_compact_size`; the next checkpoint wrote the new
/// one and every later incremental was refused as `LayoutChanged`. The
/// compaction count is written with the compacted file.
#[test]
fn incremental_after_backup_of_freshly_compacted_file() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("db.mlite");
    let backups = dir.path().join("backups");
    let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
    insert(&db, 0..50);
    db.delete_many("c", &json!({"_id": {"$lt": 20}})).unwrap();
    db.compact().unwrap();
    create_backup(&db_path, &backups, false, None).unwrap();
    db.close().unwrap();
    drop(db);

    let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
    insert(&db, 50..60);
    db.checkpoint().unwrap();
    let incr = create_backup(&db_path, &backups, false, None).unwrap();
    assert_eq!(incr.backup_type, BackupType::Incremental);
    db.close().unwrap();
    drop(db);

    let restored = dir.path().join("restored.mlite");
    restore(&backups, &restored, None, Some("db")).unwrap();
    assert_eq!(count_in(&restored), 40);
}

/// P10: restoring over a database left its vector caches (`.hnsw`) in
/// place, and the restored database searched the vectors of its later state
#[test]
fn restore_removes_every_index_cache_at_target() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("db.mlite");
    let backups = dir.path().join("backups");
    let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
    let c = db.collection("c").unwrap();
    c.create_index("pad".to_string(), false, false).unwrap();
    c.create_vector_index("v", VectorIndexConfig::new(2))
        .unwrap();
    drop(c);
    let vector_doc = |i: i64| {
        HashMap::from([
            ("_id".to_string(), json!(i)),
            ("v".to_string(), json!([1.0, i as f32 * 0.1])),
        ])
    };
    for i in 0..4 {
        db.insert_one("c", vector_doc(i)).unwrap();
    }
    create_backup(&db_path, &backups, false, None).unwrap();
    for i in 4..8 {
        db.insert_one("c", vector_doc(i)).unwrap();
    }
    db.close().unwrap();
    drop(db);
    assert!(!ironbase_core::index_cache_files(&db_path).is_empty());

    let result = restore(&backups, &db_path, None, Some("db")).unwrap();
    assert!(ironbase_core::index_cache_files(&db_path).is_empty());
    assert!(result.index_files_cleaned >= 2);
    let db = DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
    let hits = db
        .collection("c")
        .unwrap()
        .vector_search("v", &[1.0, 0.0], 4)
        .unwrap();
    assert_eq!(hits.len(), 4);
}
