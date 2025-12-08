//! End-to-end tests for ironbase-backup
//!
//! These tests verify the complete backup/restore workflow including:
//! - Full and incremental backups
//! - Chain management and discovery
//! - Integrity verification
//! - Corruption detection
//! - Multiple database handling

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

/// Path to the backup binary
fn backup_binary() -> PathBuf {
    let mut path = std::env::current_exe().unwrap();
    path.pop(); // Remove test binary name
    path.pop(); // Remove deps
    path.push("ironbase-backup");
    path
}

/// Run backup CLI command
fn run_backup(args: &[&str]) -> (bool, String, String) {
    let output = Command::new(backup_binary())
        .args(args)
        .output()
        .expect("Failed to execute backup command");

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    (output.status.success(), stdout, stderr)
}

/// Create a test database with realistic JSON-like content
fn create_test_db(path: &Path, size_mb: usize) {
    let mut file = File::create(path).unwrap();

    // Write IronBase-like header (256 bytes)
    let mut header = vec![0u8; 256];
    header[0..8].copy_from_slice(b"MONGOLTE");
    file.write_all(&header).unwrap();

    // Write JSON documents (compressible content)
    let doc_template = r#"{"_id":%d,"name":"User %d","email":"user%d@example.com","data":{"score":%d,"active":true,"tags":["test","user","data"]}}"#;

    let mut written = 256;
    let target = size_mb * 1024 * 1024;
    let mut id = 1;

    while written < target {
        let doc = doc_template.replace("%d", &id.to_string());
        let doc_bytes = doc.as_bytes();

        // Write length prefix (4 bytes) + document
        let len = doc_bytes.len() as u32;
        file.write_all(&len.to_le_bytes()).unwrap();
        file.write_all(doc_bytes).unwrap();

        written += 4 + doc_bytes.len();
        id += 1;
    }

    file.flush().unwrap();
}

/// Append data to existing database
fn append_to_db(path: &Path, size_kb: usize) {
    let mut file = OpenOptions::new().append(true).open(path).unwrap();

    let doc_template =
        r#"{"_id":%d,"type":"append","timestamp":%d,"content":"Additional data entry"}"#;

    let mut written = 0;
    let target = size_kb * 1024;
    let mut id = 1_000_000; // Start from high ID to avoid conflicts

    while written < target {
        let doc = doc_template.replace("%d", &id.to_string());
        let doc_bytes = doc.as_bytes();

        let len = doc_bytes.len() as u32;
        file.write_all(&len.to_le_bytes()).unwrap();
        file.write_all(doc_bytes).unwrap();

        written += 4 + doc_bytes.len();
        id += 1;
    }

    file.flush().unwrap();
}

/// Get file size
fn file_size(path: &Path) -> u64 {
    fs::metadata(path).unwrap().len()
}

/// Compare two files byte-by-byte
fn files_equal(path1: &Path, path2: &Path) -> bool {
    let mut f1 = File::open(path1).unwrap();
    let mut f2 = File::open(path2).unwrap();

    let mut buf1 = Vec::new();
    let mut buf2 = Vec::new();

    f1.read_to_end(&mut buf1).unwrap();
    f2.read_to_end(&mut buf2).unwrap();

    buf1 == buf2
}

/// Corrupt a backup file at a specific offset
fn corrupt_file(path: &Path, offset: u64) {
    let mut file = OpenOptions::new().write(true).open(path).unwrap();

    use std::io::{Seek, SeekFrom};
    file.seek(SeekFrom::Start(offset)).unwrap();
    file.write_all(b"CORRUPTED").unwrap();
}

// =============================================================================
// E2E Tests
// =============================================================================

#[test]
fn e2e_full_backup_and_restore() {
    let temp = TempDir::new().unwrap();
    let db_path = temp.path().join("test.mlite");
    let backup_dir = temp.path().join("backups");
    let restore_path = temp.path().join("restored.mlite");

    // Create test database (1 MB)
    create_test_db(&db_path, 1);
    let original_size = file_size(&db_path);

    // Create full backup
    let (success, stdout, _) = run_backup(&[
        "backup",
        "--db",
        db_path.to_str().unwrap(),
        "--output",
        backup_dir.to_str().unwrap(),
    ]);

    assert!(success, "Backup failed");
    assert!(stdout.contains("Full"), "Should be full backup");
    assert!(stdout.contains("compression"), "Should show compression");

    // Restore
    let (success, stdout, _) = run_backup(&[
        "restore",
        "--dir",
        backup_dir.to_str().unwrap(),
        "--output",
        restore_path.to_str().unwrap(),
    ]);

    assert!(success, "Restore failed");
    assert!(
        stdout.contains("Restore completed"),
        "Should complete restore"
    );

    // Verify restored file matches original
    assert_eq!(file_size(&restore_path), original_size);
    assert!(
        files_equal(&db_path, &restore_path),
        "Restored file should match original"
    );
}

#[test]
fn e2e_incremental_backup_chain() {
    let temp = TempDir::new().unwrap();
    let db_path = temp.path().join("test.mlite");
    let backup_dir = temp.path().join("backups");
    let restore_path = temp.path().join("restored.mlite");

    // Create initial database (500 KB)
    create_test_db(&db_path, 1);

    // Full backup
    let (success, stdout, _) = run_backup(&[
        "backup",
        "--db",
        db_path.to_str().unwrap(),
        "--output",
        backup_dir.to_str().unwrap(),
    ]);
    assert!(success);
    assert!(stdout.contains("Full"));

    // Append data (100 KB)
    append_to_db(&db_path, 100);

    // First incremental
    let (success, stdout, _) = run_backup(&[
        "backup",
        "--db",
        db_path.to_str().unwrap(),
        "--output",
        backup_dir.to_str().unwrap(),
    ]);
    assert!(success);
    assert!(stdout.contains("Incremental"));

    // Append more data (50 KB)
    append_to_db(&db_path, 50);

    // Second incremental
    let (success, stdout, _) = run_backup(&[
        "backup",
        "--db",
        db_path.to_str().unwrap(),
        "--output",
        backup_dir.to_str().unwrap(),
    ]);
    assert!(success);
    assert!(stdout.contains("Incremental"));

    let final_size = file_size(&db_path);

    // List backups - should show 3
    let (success, stdout, _) = run_backup(&["list", "--dir", backup_dir.to_str().unwrap()]);
    assert!(success);
    assert!(stdout.contains("3 backups"));
    assert!(stdout.contains("[FULL]"));
    assert!(stdout.matches("[INCR]").count() == 2);

    // Restore from chain
    let (success, stdout, _) = run_backup(&[
        "restore",
        "--dir",
        backup_dir.to_str().unwrap(),
        "--output",
        restore_path.to_str().unwrap(),
    ]);
    assert!(success);
    assert!(stdout.contains("3 applied"));

    // Verify
    assert_eq!(file_size(&restore_path), final_size);
    assert!(files_equal(&db_path, &restore_path));
}

#[test]
fn e2e_verify_chain_integrity() {
    let temp = TempDir::new().unwrap();
    let db_path = temp.path().join("test.mlite");
    let backup_dir = temp.path().join("backups");

    create_test_db(&db_path, 1);

    // Create backups
    run_backup(&[
        "backup",
        "--db",
        db_path.to_str().unwrap(),
        "--output",
        backup_dir.to_str().unwrap(),
    ]);

    append_to_db(&db_path, 50);
    run_backup(&[
        "backup",
        "--db",
        db_path.to_str().unwrap(),
        "--output",
        backup_dir.to_str().unwrap(),
    ]);

    // Verify chain
    let (success, stdout, _) = run_backup(&["verify", "--dir", backup_dir.to_str().unwrap()]);

    assert!(success);
    assert!(stdout.contains("Chain valid"));
    assert!(stdout.contains("2/2"));
}

#[test]
fn e2e_detect_corruption() {
    let temp = TempDir::new().unwrap();
    let db_path = temp.path().join("test.mlite");
    let backup_dir = temp.path().join("backups");

    create_test_db(&db_path, 1);

    // Create backup
    run_backup(&[
        "backup",
        "--db",
        db_path.to_str().unwrap(),
        "--output",
        backup_dir.to_str().unwrap(),
    ]);

    // Find the backup file and corrupt it
    let backup_file = fs::read_dir(&backup_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .find(|e| e.path().extension().map(|s| s == "ibak").unwrap_or(false))
        .unwrap()
        .path();

    // Corrupt the payload (after header at offset 200)
    corrupt_file(&backup_file, 200);

    // Verify should fail
    let (success, stdout, _) = run_backup(&["verify", "--dir", backup_dir.to_str().unwrap()]);

    assert!(!success);
    assert!(stdout.contains("FAIL") || stdout.contains("invalid"));
}

#[test]
fn e2e_restore_to_specific_point() {
    let temp = TempDir::new().unwrap();
    let db_path = temp.path().join("test.mlite");
    let backup_dir = temp.path().join("backups");
    let restore_path = temp.path().join("restored.mlite");

    // Create initial database
    create_test_db(&db_path, 1);
    let size_after_full = file_size(&db_path);

    // Full backup
    run_backup(&[
        "backup",
        "--db",
        db_path.to_str().unwrap(),
        "--output",
        backup_dir.to_str().unwrap(),
    ]);

    // Append and create incremental
    append_to_db(&db_path, 100);
    run_backup(&[
        "backup",
        "--db",
        db_path.to_str().unwrap(),
        "--output",
        backup_dir.to_str().unwrap(),
    ]);

    // Get the full backup filename
    let full_backup = fs::read_dir(&backup_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .find(|e| e.file_name().to_str().unwrap().contains("full"))
        .unwrap();

    // Restore to full backup only (before incremental)
    let (success, stdout, _) = run_backup(&[
        "restore",
        "--dir",
        backup_dir.to_str().unwrap(),
        "--output",
        restore_path.to_str().unwrap(),
        "--target",
        full_backup.file_name().to_str().unwrap(),
    ]);

    assert!(success);
    assert!(stdout.contains("1 applied")); // Only full backup

    // Should match size after full backup, not after incremental
    assert_eq!(file_size(&restore_path), size_after_full);
}

#[test]
fn e2e_force_full_backup() {
    let temp = TempDir::new().unwrap();
    let db_path = temp.path().join("test.mlite");
    let backup_dir = temp.path().join("backups");

    create_test_db(&db_path, 1);

    // First backup (auto full)
    run_backup(&[
        "backup",
        "--db",
        db_path.to_str().unwrap(),
        "--output",
        backup_dir.to_str().unwrap(),
    ]);

    // Force another full backup
    let (success, stdout, _) = run_backup(&[
        "backup",
        "--db",
        db_path.to_str().unwrap(),
        "--output",
        backup_dir.to_str().unwrap(),
        "--full",
    ]);

    assert!(success);
    assert!(stdout.contains("Full")); // Should be full, not incremental
}

#[test]
fn e2e_multiple_databases() {
    let temp = TempDir::new().unwrap();
    let db1_path = temp.path().join("users.mlite");
    let db2_path = temp.path().join("orders.mlite");
    let backup_dir = temp.path().join("backups");

    // Create two databases
    create_test_db(&db1_path, 1);
    create_test_db(&db2_path, 1);

    // Backup both to same directory
    run_backup(&[
        "backup",
        "--db",
        db1_path.to_str().unwrap(),
        "--output",
        backup_dir.to_str().unwrap(),
    ]);
    run_backup(&[
        "backup",
        "--db",
        db2_path.to_str().unwrap(),
        "--output",
        backup_dir.to_str().unwrap(),
    ]);

    // List should show both chains
    let (success, stdout, _) = run_backup(&["list", "--dir", backup_dir.to_str().unwrap()]);

    assert!(success);
    assert!(stdout.contains("users"));
    assert!(stdout.contains("orders"));
}

#[test]
fn e2e_backup_info() {
    let temp = TempDir::new().unwrap();
    let db_path = temp.path().join("test.mlite");
    let backup_dir = temp.path().join("backups");

    create_test_db(&db_path, 1);
    run_backup(&[
        "backup",
        "--db",
        db_path.to_str().unwrap(),
        "--output",
        backup_dir.to_str().unwrap(),
    ]);

    // Find backup file
    let backup_file = fs::read_dir(&backup_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .find(|e| e.path().extension().map(|s| s == "ibak").unwrap_or(false))
        .unwrap()
        .path();

    // Get info
    let (success, stdout, _) = run_backup(&["info", "--file", backup_file.to_str().unwrap()]);

    assert!(success);
    assert!(stdout.contains("Database:"));
    assert!(stdout.contains("Type:"));
    assert!(stdout.contains("Full"));
    assert!(stdout.contains("Content hash:"));
    assert!(stdout.contains("OK")); // Verification passed
}

#[test]
fn e2e_compression_ratio() {
    let temp = TempDir::new().unwrap();
    let db_path = temp.path().join("test.mlite");
    let backup_dir = temp.path().join("backups");

    // Create database with compressible JSON content
    create_test_db(&db_path, 2);
    let original_size = file_size(&db_path);

    // Backup
    let (success, stdout, _) = run_backup(&[
        "backup",
        "--db",
        db_path.to_str().unwrap(),
        "--output",
        backup_dir.to_str().unwrap(),
    ]);

    assert!(success);

    // Find backup file and check it's smaller
    let backup_file = fs::read_dir(&backup_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .find(|e| e.path().extension().map(|s| s == "ibak").unwrap_or(false))
        .unwrap()
        .path();

    let backup_size = file_size(&backup_file);

    // JSON data should compress well (at least 2x)
    let ratio = original_size as f64 / backup_size as f64;
    assert!(
        ratio > 1.5,
        "Compression ratio {} should be > 1.5 for JSON data",
        ratio
    );

    // Output should mention compression
    assert!(stdout.contains("compression"));
}

#[test]
fn e2e_empty_incremental() {
    let temp = TempDir::new().unwrap();
    let db_path = temp.path().join("test.mlite");
    let backup_dir = temp.path().join("backups");

    create_test_db(&db_path, 1);

    // Full backup
    run_backup(&[
        "backup",
        "--db",
        db_path.to_str().unwrap(),
        "--output",
        backup_dir.to_str().unwrap(),
    ]);

    // Incremental without changes (should still work)
    let (success, stdout, _) = run_backup(&[
        "backup",
        "--db",
        db_path.to_str().unwrap(),
        "--output",
        backup_dir.to_str().unwrap(),
    ]);

    assert!(success);
    assert!(stdout.contains("Incremental"));
    assert!(stdout.contains("0 B") || stdout.contains("0.00")); // Zero or very small size
}

#[test]
fn e2e_missing_database() {
    let temp = TempDir::new().unwrap();
    let db_path = temp.path().join("nonexistent.mlite");
    let backup_dir = temp.path().join("backups");

    let (success, _, stderr) = run_backup(&[
        "backup",
        "--db",
        db_path.to_str().unwrap(),
        "--output",
        backup_dir.to_str().unwrap(),
    ]);

    assert!(!success);
    assert!(stderr.contains("Error") || stderr.contains("not found"));
}

#[test]
fn e2e_restore_no_backups() {
    let temp = TempDir::new().unwrap();
    let backup_dir = temp.path().join("empty_backups");
    let restore_path = temp.path().join("restored.mlite");

    fs::create_dir_all(&backup_dir).unwrap();

    let (success, _, stderr) = run_backup(&[
        "restore",
        "--dir",
        backup_dir.to_str().unwrap(),
        "--output",
        restore_path.to_str().unwrap(),
    ]);

    assert!(!success);
    assert!(stderr.contains("Error") || stderr.contains("No backups"));
}
