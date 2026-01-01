//! Restore operations for backup chains
//!
//! Note: IronBase automatically rebuilds indexes from documents when the
//! database is first opened after restore. The .idx cache files are not
//! included in backups and will be regenerated on first use.

use crate::chain::{detect_db_name, Chain};
use crate::color::{green, red};
use crate::compression::{decompress, format_size};
use crate::error::{BackupError, Result};
use crate::format::{hash_to_short_hex, DB_HEADER_SIZE, FOOTER_SIZE, HEADER_SIZE};
use crate::verify::verify_backup;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// Result of a restore operation
#[derive(Debug)]
pub struct RestoreResult {
    /// Path to the restored database file
    pub path: PathBuf,
    /// Final size of restored database
    pub size: u64,
    /// Number of backups applied
    pub backups_applied: usize,
    /// Number of stale .idx files cleaned up
    pub idx_files_cleaned: usize,
    /// Time taken in seconds
    pub duration_secs: f64,
}

impl RestoreResult {
    /// Print summary of the restore
    pub fn print_summary(&self) {
        println!("Restore completed successfully!");
        println!("  Output:    {}", self.path.display());
        println!("  Size:      {}", format_size(self.size));
        println!("  Backups:   {} applied", self.backups_applied);
        if self.idx_files_cleaned > 0 {
            println!(
                "  Cleanup:   {} stale .idx files removed",
                self.idx_files_cleaned
            );
        }
        println!("  Time:      {:.2}s", self.duration_secs);
        println!();
        println!("Note: Indexes will be automatically rebuilt on first database open.");
    }
}

/// Clean up stale .idx files that might exist from a previous database
///
/// IronBase stores index caches in separate .idx files. After restoring,
/// any existing .idx files would be stale and inconsistent with the
/// restored data. IronBase will automatically rebuild indexes on first open.
fn cleanup_stale_idx_files(db_path: &Path) -> usize {
    let mut cleaned = 0;

    // Get the directory and stem of the database file
    let Some(parent) = db_path.parent() else {
        return 0;
    };
    let Some(stem) = db_path.file_stem().and_then(|s| s.to_str()) else {
        return 0;
    };

    // Pattern: {stem}_*.idx
    let prefix = format!("{}_", stem);

    if let Ok(entries) = fs::read_dir(parent) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                if name.starts_with(&prefix)
                    && name.ends_with(".idx")
                    && fs::remove_file(&path).is_ok()
                {
                    cleaned += 1;
                }
            }
        }
    }

    cleaned
}

/// Restore database from backup chain
///
/// # Arguments
/// * `backup_dir` - Directory containing backup files
/// * `output_path` - Path where restored database will be written
/// * `target` - Optional specific backup to restore to (defaults to latest)
/// * `db_name` - Optional database name (auto-detected if not provided)
///
/// # Returns
/// RestoreResult with details about the restored database
pub fn restore(
    backup_dir: &Path,
    output_path: &Path,
    target: Option<&str>,
    db_name: Option<&str>,
) -> Result<RestoreResult> {
    let start_time = std::time::Instant::now();

    // Determine database name
    let db_name = match db_name {
        Some(name) => name.to_string(),
        None => detect_db_name(backup_dir)?,
    };

    // Discover backup chain
    let chain = Chain::discover(backup_dir, &db_name)?;

    if chain.is_empty() {
        return Err(BackupError::NoBackupsFound { db_name });
    }

    // Find target backup index
    let target_idx = match target {
        Some(name) => chain.find_by_name(name)?,
        None => chain.backups.len() - 1, // Latest
    };

    println!(
        "Restoring database '{}' from {} backup(s)...",
        db_name,
        target_idx + 1
    );

    // Clean up any stale .idx files from previous database
    let idx_cleaned = cleanup_stale_idx_files(output_path);
    if idx_cleaned > 0 {
        println!("Cleaned up {} stale .idx file(s)", idx_cleaned);
    }

    // Verify chain integrity up to target
    println!("Verifying backup integrity...");
    for (i, backup) in chain.backups[..=target_idx].iter().enumerate() {
        print!(
            "  [{}/{}] {} ... ",
            i + 1,
            target_idx + 1,
            backup.filename()
        );
        std::io::stdout().flush()?;

        let result = verify_backup(&backup.path)?;
        if !result.valid {
            println!("{}", red("FAILED"));
            return Err(BackupError::ChecksumMismatch {
                expected: hash_to_short_hex(&result.expected_hash),
                actual: hash_to_short_hex(&result.actual_hash),
            });
        }
        println!("{}", green("OK"));
    }

    // Create output file
    let output_file = File::create(output_path)?;
    let mut writer = BufWriter::new(output_file);

    // Apply backups in order
    println!("Applying backups...");
    for (i, backup) in chain.backups[..=target_idx].iter().enumerate() {
        print!(
            "  [{}/{}] {} ({}) ... ",
            i + 1,
            target_idx + 1,
            backup.filename(),
            format_size(backup.header.compressed_length)
        );
        std::io::stdout().flush()?;

        // Read and decompress backup data
        let data = read_and_decompress(&backup.path)?;

        // Handle includes_db_header flag for incremental backups
        // When true, payload = [DB header (256)] + [incremental data]
        if backup.header.includes_db_header && data.len() > DB_HEADER_SIZE {
            // Extract DB header (first 256 bytes) and write at position 0
            let db_header = &data[..DB_HEADER_SIZE];
            writer.seek(SeekFrom::Start(0))?;
            writer.write_all(db_header)?;

            // Write incremental data at start_offset
            let incremental_data = &data[DB_HEADER_SIZE..];
            writer.seek(SeekFrom::Start(backup.header.start_offset))?;
            writer.write_all(incremental_data)?;
        } else {
            // Full backup or legacy incremental without DB header
            writer.seek(SeekFrom::Start(backup.header.start_offset))?;
            writer.write_all(&data)?;
        }

        println!("done");
    }

    writer.flush()?;
    drop(writer);

    // Get final size
    let final_size = std::fs::metadata(output_path)?.len();
    let duration = start_time.elapsed().as_secs_f64();

    Ok(RestoreResult {
        path: output_path.to_path_buf(),
        size: final_size,
        backups_applied: target_idx + 1,
        idx_files_cleaned: idx_cleaned,
        duration_secs: duration,
    })
}

/// Read backup file and decompress payload
/// Handles both single-file and multi-part backups
fn read_and_decompress(path: &Path) -> Result<Vec<u8>> {
    use crate::format::BackupHeader;

    let file = File::open(path)?;
    let mut reader = BufReader::new(file);

    // Read header to check if multi-part
    let header = BackupHeader::read_from(&mut reader)?;

    if header.is_multipart() {
        // Multi-part backup: read and concatenate all parts
        read_multipart_backup(path, &header)
    } else {
        // Single-file backup
        let file = File::open(path)?;
        let file_size = file.metadata()?.len();
        let mut reader = BufReader::new(file);

        // Skip header
        reader.seek(SeekFrom::Start(HEADER_SIZE as u64))?;

        // Read compressed payload
        let payload_size = file_size - HEADER_SIZE as u64 - FOOTER_SIZE as u64;
        let mut compressed = vec![0u8; payload_size as usize];
        reader.read_exact(&mut compressed)?;

        // Decompress
        decompress(&compressed)
    }
}

/// Read all parts of a multi-part backup and concatenate them
fn read_multipart_backup(
    first_part_path: &Path,
    first_header: &crate::format::BackupHeader,
) -> Result<Vec<u8>> {
    let total_parts = first_header.total_parts;

    // Derive base path from first part (remove .ibak.001 extension)
    let path_str = first_part_path.to_string_lossy();
    let base_path = if path_str.ends_with(".001") {
        path_str.trim_end_matches(".001").to_string()
    } else {
        // Fallback: assume it's already the base path with .ibak extension
        path_str.trim_end_matches(".ibak").to_string()
    };

    let mut all_compressed = Vec::new();

    for part_num in 1..=total_parts {
        let part_path = PathBuf::from(format!("{}.{:03}", base_path, part_num));

        if !part_path.exists() {
            return Err(BackupError::InvalidBackupFile {
                reason: format!(
                    "Missing part {} of {}: {}",
                    part_num,
                    total_parts,
                    part_path.display()
                ),
            });
        }

        // Read this part's compressed payload
        let file = File::open(&part_path)?;
        let file_size = file.metadata()?.len();
        let mut reader = BufReader::new(file);

        // Skip header
        reader.seek(SeekFrom::Start(HEADER_SIZE as u64))?;

        // Read compressed payload
        let payload_size = file_size - HEADER_SIZE as u64 - FOOTER_SIZE as u64;
        let mut part_data = vec![0u8; payload_size as usize];
        reader.read_exact(&mut part_data)?;

        all_compressed.extend_from_slice(&part_data);
    }

    // Decompress concatenated data
    decompress(&all_compressed)
}

/// List available restore points
pub fn list_restore_points(backup_dir: &Path) -> Result<()> {
    let chains = Chain::discover_all(backup_dir)?;

    if chains.is_empty() {
        println!("No backups found in {}", backup_dir.display());
        return Ok(());
    }

    println!("Available restore points:\n");

    for chain in chains {
        chain.print_summary();
        println!();
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::create_backup;
    use tempfile::TempDir;

    #[test]
    fn test_restore_full_backup() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");
        let backup_dir = temp_dir.path().join("backups");
        let restore_path = temp_dir.path().join("restored.mlite");

        // Create test database with known content
        let original_content = b"Hello, World! This is test data.".repeat(100);
        std::fs::write(&db_path, &original_content).unwrap();

        // Create backup (no split)
        create_backup(&db_path, &backup_dir, false, None).unwrap();

        // Restore
        let result = restore(&backup_dir, &restore_path, None, None).unwrap();

        // Verify restored content matches original
        let restored_content = std::fs::read(&restore_path).unwrap();
        assert_eq!(original_content.as_slice(), restored_content.as_slice());
        assert_eq!(result.backups_applied, 1);
    }

    #[test]
    fn test_restore_with_incremental() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");
        let backup_dir = temp_dir.path().join("backups");
        let restore_path = temp_dir.path().join("restored.mlite");

        // Create initial database
        let initial = b"Initial content.".repeat(100);
        std::fs::write(&db_path, &initial).unwrap();

        // Create full backup
        create_backup(&db_path, &backup_dir, false, None).unwrap();

        // Append to database
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&db_path)
            .unwrap();
        let additional = b"Additional content.".repeat(50);
        file.write_all(&additional).unwrap();
        drop(file);

        // Create incremental backup
        create_backup(&db_path, &backup_dir, false, None).unwrap();

        // Restore
        let result = restore(&backup_dir, &restore_path, None, None).unwrap();

        // Verify restored content
        let restored = std::fs::read(&restore_path).unwrap();
        let mut expected = initial.to_vec();
        expected.extend_from_slice(&additional);
        assert_eq!(expected, restored);
        assert_eq!(result.backups_applied, 2);
    }

    #[test]
    fn test_restore_split_backup() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");
        let backup_dir = temp_dir.path().join("backups");
        let restore_path = temp_dir.path().join("restored.mlite");

        // Create pseudo-random data (doesn't compress well)
        let mut seed: u64 = 12345;
        let mut original_content = Vec::with_capacity(100 * 1024);
        for _ in 0..(100 * 1024) {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            original_content.push((seed >> 33) as u8);
        }
        std::fs::write(&db_path, &original_content).unwrap();

        // Create split backup with 20KB parts
        let backup_result = create_backup(&db_path, &backup_dir, false, Some(20 * 1024)).unwrap();
        assert!(backup_result.part_count >= 2, "Expected multi-part backup");

        // Restore from split backup
        let result = restore(&backup_dir, &restore_path, None, None).unwrap();

        // Verify restored content matches original
        let restored_content = std::fs::read(&restore_path).unwrap();
        assert_eq!(original_content.as_slice(), restored_content.as_slice());
        assert_eq!(result.backups_applied, 1);
    }
}
