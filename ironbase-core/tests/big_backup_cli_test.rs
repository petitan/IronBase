// 1.1GB Backup/Restore Stress Test - Pure Rust + CLI
// - 10 collections with indexes
// - Full backup + Incremental backup + Restore via CLI

use ironbase_core::{DatabaseCore, StorageEngine};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::process::Command;
use std::time::Instant;

const TEST_DIR: &str = "/tmp/rust_big_backup_test";
const DB_PATH: &str = "/tmp/rust_big_backup_test/bigtest.mlite";
const BACKUP_DIR: &str = "/tmp/rust_big_backup_test/backups";
const BACKUP_CLI: &str = "/home/petitan/MongoLite/target/release/ironbase-backup";

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.2} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.2} MB", bytes as f64 / 1024.0 / 1024.0)
    } else {
        format!("{:.2} GB", bytes as f64 / 1024.0 / 1024.0 / 1024.0)
    }
}

fn format_speed(bytes: u64, seconds: f64) -> String {
    if seconds == 0.0 {
        return "∞".to_string();
    }
    let mb_per_sec = (bytes as f64 / 1024.0 / 1024.0) / seconds;
    format!("{:.1} MB/s", mb_per_sec)
}

fn json_to_hashmap(v: Value) -> HashMap<String, Value> {
    match v {
        Value::Object(map) => map.into_iter().collect(),
        _ => HashMap::new(),
    }
}

#[test]
#[ignore] // Run with: cargo test -p ironbase-core --release big_backup_cli_test -- --ignored --nocapture
fn big_backup_cli_test() {
    println!("\n{}", "=".repeat(70));
    println!("IronBase 1.1GB Backup/Restore Test - Pure Rust + CLI");
    println!("{}", "=".repeat(70));

    // Clean up
    let _ = fs::remove_dir_all(TEST_DIR);
    fs::create_dir_all(BACKUP_DIR).unwrap();

    // Collections with their index fields
    let collections = vec![
        ("users", vec!["email", "username"]),
        ("products", vec!["sku", "category"]),
        ("orders", vec!["order_id", "customer_id"]),
        ("logs", vec!["timestamp", "level"]),
        ("sessions", vec!["session_id", "user_id"]),
        ("analytics", vec!["event_type", "date"]),
        ("inventory", vec!["product_id", "warehouse"]),
        ("payments", vec!["transaction_id", "status"]),
        ("notifications", vec!["recipient", "type"]),
        ("audit", vec!["action", "entity_id"]),
    ];

    // Target: ~1.1GB, ~400 bytes per doc
    let target_size: u64 = 1_100 * 1024 * 1024;
    let doc_size: u64 = 400;
    let total_docs = target_size / doc_size;
    let docs_per_collection = total_docs / collections.len() as u64;
    let batch_size: u64 = 5000;

    // ========== PHASE 1: Create Database ==========
    println!("\n[1] Creating 1.1GB database with 10 collections + indexes...");
    println!(
        "    Target: {} docs per collection ({} total)",
        docs_per_collection, total_docs
    );

    let start = Instant::now();

    {
        let db = DatabaseCore::<StorageEngine>::open(DB_PATH).unwrap();

        // Create collections and indexes
        println!("\n    Creating indexes...");
        for (coll_name, index_fields) in &collections {
            let coll = db.collection(coll_name).unwrap();
            for field in index_fields {
                coll.create_index(field.to_string(), false, false).unwrap();
            }
            println!("      {}: indexes on {:?}", coll_name, index_fields);
        }

        // Insert documents using insert_many with batches
        println!("\n    Inserting documents...");
        for (coll_name, _) in &collections {
            let coll_start = Instant::now();
            let mut inserted: u64 = 0;

            while inserted < docs_per_collection {
                let batch_count = std::cmp::min(batch_size, docs_per_collection - inserted);
                let mut batch: Vec<HashMap<String, Value>> =
                    Vec::with_capacity(batch_count as usize);

                for i in 0..batch_count {
                    let doc_id = inserted + i;
                    batch.push(json_to_hashmap(json!({
                        "_id": format!("{}_{}", coll_name, doc_id),
                        "name": format!("item_{}", doc_id),
                        "email": format!("user{}@example.com", doc_id),
                        "category": format!("cat_{}", doc_id % 100),
                        "data": "x".repeat(300),
                        "value": doc_id as f64 * 1.5,
                        "active": doc_id.is_multiple_of(2)
                    })));
                }

                db.insert_many(coll_name, batch).unwrap();
                inserted += batch_count;

                if inserted.is_multiple_of(50000) {
                    print!(".");
                    use std::io::Write;
                    std::io::stdout().flush().unwrap();
                }
            }

            let coll_elapsed = coll_start.elapsed().as_secs_f64();
            println!(
                "\n      {}: {} docs in {:.1}s",
                coll_name, docs_per_collection, coll_elapsed
            );
        }
    }

    let elapsed = start.elapsed().as_secs_f64();
    let db_size = fs::metadata(DB_PATH).map(|m| m.len()).unwrap_or(0);

    println!(
        "\n    Database created: {} in {:.1}s ({})",
        format_size(db_size),
        elapsed,
        format_speed(db_size, elapsed)
    );

    // ========== PHASE 2: Full Backup via CLI ==========
    println!("\n{}", "=".repeat(70));
    println!("[2] FULL BACKUP (CLI)");
    println!("{}", "=".repeat(70));

    let start = Instant::now();
    let output = Command::new(BACKUP_CLI)
        .args(["backup", "--db", DB_PATH, "--output", BACKUP_DIR, "--full"])
        .output()
        .expect("Failed to run backup CLI");

    let elapsed_full = start.elapsed().as_secs_f64();

    println!("{}", String::from_utf8_lossy(&output.stdout));
    if !output.stderr.is_empty() {
        eprintln!("{}", String::from_utf8_lossy(&output.stderr));
    }

    // Find backup file
    let full_backup = fs::read_dir(BACKUP_DIR)
        .unwrap()
        .filter_map(|e| e.ok())
        .find(|e| e.path().extension().map(|x| x == "ibak").unwrap_or(false))
        .map(|e| e.path());

    let full_size = full_backup
        .as_ref()
        .and_then(|p| fs::metadata(p).ok())
        .map(|m| m.len())
        .unwrap_or(0);

    println!("    Backup time: {:.2}s", elapsed_full);
    println!("    Speed: {}", format_speed(db_size, elapsed_full));

    // ========== PHASE 3: Add More Data ==========
    println!("\n{}", "=".repeat(70));
    println!("[3] Adding ~100MB more data (simulating daily changes)...");
    println!("{}", "=".repeat(70));

    let extra_docs_per_coll = 25000u64;
    let start = Instant::now();

    {
        let db = DatabaseCore::<StorageEngine>::open(DB_PATH).unwrap();

        for (coll_name, _) in &collections {
            let mut batch: Vec<HashMap<String, Value>> =
                Vec::with_capacity(extra_docs_per_coll as usize);

            for i in 0..extra_docs_per_coll {
                let doc_id = 10_000_000 + i;
                batch.push(json_to_hashmap(json!({
                    "_id": format!("{}_new_{}", coll_name, doc_id),
                    "name": format!("new_item_{}", doc_id),
                    "email": format!("new{}@example.com", doc_id),
                    "data": "y".repeat(300),
                    "value": doc_id as f64 * 2.0
                })));
            }

            db.insert_many(coll_name, batch).unwrap();
            print!(".");
            use std::io::Write;
            std::io::stdout().flush().unwrap();
        }
    }

    let elapsed = start.elapsed().as_secs_f64();
    let new_db_size = fs::metadata(DB_PATH).map(|m| m.len()).unwrap_or(0);
    let delta = new_db_size - db_size;

    println!("\n    Added {} in {:.1}s", format_size(delta), elapsed);
    println!("    New DB size: {}", format_size(new_db_size));

    // ========== PHASE 4: Incremental Backup via CLI ==========
    println!("\n{}", "=".repeat(70));
    println!("[4] INCREMENTAL BACKUP (CLI)");
    println!("{}", "=".repeat(70));

    let start = Instant::now();
    let output = Command::new(BACKUP_CLI)
        .args(["backup", "--db", DB_PATH, "--output", BACKUP_DIR])
        .output()
        .expect("Failed to run backup CLI");

    let elapsed_inc = start.elapsed().as_secs_f64();

    println!("{}", String::from_utf8_lossy(&output.stdout));

    println!("    Incremental backup time: {:.2}s", elapsed_inc);

    // ========== PHASE 5: Restore via CLI ==========
    println!("\n{}", "=".repeat(70));
    println!("[5] RESTORE FROM BACKUPS (CLI)");
    println!("{}", "=".repeat(70));

    let restore_path = format!("{}/restored.mlite", TEST_DIR);

    let start = Instant::now();
    let output = Command::new(BACKUP_CLI)
        .args(["restore", "--dir", BACKUP_DIR, "--output", &restore_path])
        .output()
        .expect("Failed to run restore CLI");

    let elapsed_restore = start.elapsed().as_secs_f64();

    println!("{}", String::from_utf8_lossy(&output.stdout));

    let restored_size = fs::metadata(&restore_path).map(|m| m.len()).unwrap_or(0);
    println!("    Restore time: {:.2}s", elapsed_restore);
    println!("    Restored size: {}", format_size(restored_size));
    println!(
        "    Speed: {}",
        format_speed(restored_size, elapsed_restore)
    );

    // ========== PHASE 6: Verify (strict) ==========
    println!("\n{}", "=".repeat(70));
    println!("[6] VERIFICATION (strict)");
    println!("{}", "=".repeat(70));

    let mut total_docs_restored: u64 = 0;
    let mut errors: Vec<String> = Vec::new();
    let mut docs_compared: u64 = 0;
    let mut indexes_checked: u64 = 0;
    let expected_per_coll = docs_per_collection + extra_docs_per_coll;

    {
        let db = DatabaseCore::<StorageEngine>::open(&restore_path).unwrap();

        for (coll_name, index_fields) in &collections {
            let coll = db.collection(coll_name).unwrap();

            // (a) Strict per-collection count check (no tolerance)
            let count = coll.count_documents(&json!({})).unwrap();
            total_docs_restored += count;
            if count != expected_per_coll {
                errors.push(format!(
                    "{}: count {} != expected {}",
                    coll_name, count, expected_per_coll
                ));
            }

            // (b) Index existence check (auto-named: "{collection}_{field}")
            let listed = coll.list_indexes().unwrap_or_default();
            for field in index_fields {
                let expected_idx = format!("{}_{}", coll_name, field);
                if listed.contains(&expected_idx) {
                    indexes_checked += 1;
                } else {
                    errors.push(format!(
                        "{}: index '{}' missing after restore (have: {:?})",
                        coll_name, expected_idx, listed
                    ));
                }
            }

            // (c) Doc-level deep equality on samples
            // Phase 1 docs: first 20, last 20 (boundary coverage)
            let mut phase1_ids: Vec<u64> = (0..20).collect();
            phase1_ids.extend((docs_per_collection - 20)..docs_per_collection);
            for doc_id in &phase1_ids {
                let id = format!("{}_{}", coll_name, doc_id);
                let expected = json!({
                    "_id": id,
                    "name": format!("item_{}", doc_id),
                    "email": format!("user{}@example.com", doc_id),
                    "category": format!("cat_{}", doc_id % 100),
                    "data": "x".repeat(300),
                    "value": *doc_id as f64 * 1.5,
                    "active": doc_id.is_multiple_of(2),
                });
                match coll.find_one(&json!({"_id": id.clone()})).unwrap() {
                    Some(actual) => {
                        if actual != expected {
                            errors.push(format!(
                                "{}: doc {} content mismatch\n  got:      {}\n  expected: {}",
                                coll_name, id, actual, expected
                            ));
                        }
                        docs_compared += 1;
                    }
                    None => errors.push(format!("{}: doc {} missing", coll_name, id)),
                }
            }

            // Phase 3 (extra) docs: first 10
            for i in 0..10u64 {
                let doc_id = 10_000_000 + i;
                let id = format!("{}_new_{}", coll_name, doc_id);
                let expected = json!({
                    "_id": id,
                    "name": format!("new_item_{}", doc_id),
                    "email": format!("new{}@example.com", doc_id),
                    "data": "y".repeat(300),
                    "value": doc_id as f64 * 2.0,
                });
                match coll.find_one(&json!({"_id": id.clone()})).unwrap() {
                    Some(actual) => {
                        if actual != expected {
                            errors
                                .push(format!("{}: extra doc {} content mismatch", coll_name, id));
                        }
                        docs_compared += 1;
                    }
                    None => errors.push(format!("{}: extra doc {} missing", coll_name, id)),
                }
            }

            println!(
                "    {}: count={} (ok), {} indexes verified",
                coll_name,
                count,
                index_fields.len()
            );
        }

        // (d) Indexed lookup smoke test: query via 'email' index on 'users'
        // Verifies the custom B+ tree index actually works post-restore (not just _id catalog).
        let users = db.collection("users").unwrap();
        let probe_email = "user100@example.com";
        match users.find_one(&json!({"email": probe_email})).unwrap() {
            Some(doc) => {
                let got_id = doc.get("_id").and_then(|v| v.as_str()).unwrap_or("");
                if got_id != "users_100" {
                    errors.push(format!(
                        "indexed email lookup returned wrong _id: {} != users_100",
                        got_id
                    ));
                }
            }
            None => errors.push(format!(
                "indexed email lookup for {} returned None on 'users'",
                probe_email
            )),
        }
    }

    let expected_total = expected_per_coll * collections.len() as u64;

    // ========== SUMMARY ==========
    println!("\n{}", "=".repeat(70));
    println!("SUMMARY");
    println!("{}", "=".repeat(70));
    println!("Original DB size:     {}", format_size(new_db_size));
    println!("Full backup size:     {}", format_size(full_size));
    println!("Restored DB size:     {}", format_size(restored_size));
    println!();
    println!(
        "Full backup time:     {:.2}s ({})",
        elapsed_full,
        format_speed(db_size, elapsed_full)
    );
    println!("Incremental time:     {:.2}s", elapsed_inc);
    println!(
        "Restore time:         {:.2}s ({})",
        elapsed_restore,
        format_speed(restored_size, elapsed_restore)
    );
    println!();
    println!(
        "Total docs restored:  {} (expected {})",
        total_docs_restored, expected_total
    );
    println!("Docs deep-compared:   {}", docs_compared);
    println!("Indexes verified:     {}", indexes_checked);
    println!("{}", "=".repeat(70));

    // Strict assert: every check must pass
    if !errors.is_empty() {
        eprintln!("\n✗ {} verification error(s):", errors.len());
        for (i, e) in errors.iter().enumerate() {
            eprintln!("  [{}] {}", i + 1, e);
        }
        panic!("Backup/restore round-trip failed strict verification");
    }
    assert_eq!(
        total_docs_restored, expected_total,
        "Total document count mismatch"
    );
    println!("\n✓ All strict verifications passed!");

    // Cleanup
    // let _ = fs::remove_dir_all(TEST_DIR);
}
