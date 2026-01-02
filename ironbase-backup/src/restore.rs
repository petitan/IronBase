//! Restore operations for backup chains
//!
//! Note: IronBase automatically rebuilds indexes from documents when the
//! database is first opened after restore. The .idx cache files are not
//! included in backups and will be regenerated on first use.

use crate::chain::{detect_db_name, BackupInfo, Chain};
use crate::color::{green, red};
use crate::compression::{decompress, decompress_stream, format_size};
use crate::error::{BackupError, Result};
use crate::format::{hash_to_short_hex, BackupHeader, DB_HEADER_SIZE, FOOTER_SIZE, HEADER_SIZE};
use crate::verify::verify_backup;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// Threshold for using streaming decompression (1 GB compressed)
const STREAMING_THRESHOLD: u64 = 1024 * 1024 * 1024;

/// Result of a restore operation
#[derive(Debug)]
pub struct RestoreResult {
    /// Path to the restored database file
    pub path: PathBuf,
    /// Final size of restored database
    pub size: u64,
    /// Number of backups applied
    pub backups_applied: usize,
    /// Number of stale index cache files cleaned up (.idx, .ftidx, .fzidx)
    pub index_files_cleaned: usize,
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
        if self.index_files_cleaned > 0 {
            println!(
                "  Cleanup:   {} stale index cache files removed",
                self.index_files_cleaned
            );
        }
        println!("  Time:      {:.2}s", self.duration_secs);
        println!();
        println!("Note: Indexes will be automatically rebuilt on first database open.");
    }
}

/// Clean up stale index cache files that might exist from a previous database
///
/// IronBase stores index caches in separate files:
/// - `.idx` - B+ tree index caches
/// - `.ftidx` - Fulltext index caches
/// - `.fzidx` - Fuzzy index caches
///
/// After restoring, any existing cache files would be stale and inconsistent
/// with the restored data. IronBase will automatically rebuild all indexes
/// from documents on first open.
fn cleanup_stale_index_files(db_path: &Path) -> usize {
    let mut cleaned = 0;

    // Get the directory and stem of the database file
    let Some(parent) = db_path.parent() else {
        return 0;
    };
    let Some(stem) = db_path.file_stem().and_then(|s| s.to_str()) else {
        return 0;
    };

    // Pattern: {stem}_*.{idx,ftidx,fzidx}
    let prefix = format!("{}_", stem);
    let extensions = [".idx", ".ftidx", ".fzidx"];

    if let Ok(entries) = fs::read_dir(parent) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                if name.starts_with(&prefix)
                    && extensions.iter().any(|ext| name.ends_with(ext))
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

    // Clean up any stale index cache files from previous database
    let idx_cleaned = cleanup_stale_index_files(output_path);
    if idx_cleaned > 0 {
        println!("Cleaned up {} stale index cache file(s)", idx_cleaned);
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

        // Use streaming for large backups to avoid OOM
        let use_streaming = backup.header.compressed_length > STREAMING_THRESHOLD;

        if use_streaming {
            // Streaming mode: decompress directly to output file
            apply_backup_streaming(backup, &mut writer)?;
        } else {
            // In-memory mode for smaller backups
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
        index_files_cleaned: idx_cleaned,
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

/// Apply a backup using streaming decompression (for large backups)
///
/// This function handles both single-file and multi-part backups,
/// streaming decompression directly to the output file to avoid OOM.
fn apply_backup_streaming<W: Write + Seek>(backup: &BackupInfo, writer: &mut W) -> Result<()> {
    // Check if multi-part
    let file = File::open(&backup.path)?;
    let mut reader = BufReader::new(file);
    let header = BackupHeader::read_from(&mut reader)?;

    if header.is_multipart() {
        // Multi-part backup: chain all parts together for streaming
        apply_multipart_streaming(backup, &header, writer)?;
    } else {
        // Single-file backup
        apply_single_file_streaming(&backup.path, &backup.header, writer)?;
    }

    Ok(())
}

/// Apply a single-file backup using streaming
fn apply_single_file_streaming<W: Write + Seek>(
    path: &Path,
    header: &BackupHeader,
    writer: &mut W,
) -> Result<()> {
    let file = File::open(path)?;
    let file_size = file.metadata()?.len();
    let mut reader = BufReader::new(file);

    // Skip backup header
    reader.seek(SeekFrom::Start(HEADER_SIZE as u64))?;

    // Create a limited reader for the compressed payload
    let payload_size = file_size - HEADER_SIZE as u64 - FOOTER_SIZE as u64;
    let limited_reader = reader.take(payload_size);

    if header.includes_db_header {
        // Decompress to temp buffer for DB header extraction
        // For streaming with DB header, we need a different approach:
        // decompress to a temp file, then process
        let temp_path = path.with_extension("tmp");
        {
            let temp_file = File::create(&temp_path)?;
            let temp_writer = BufWriter::new(temp_file);
            decompress_stream(limited_reader, temp_writer)?;
        }

        // Read from temp file
        let mut temp_reader = BufReader::new(File::open(&temp_path)?);

        // Read and write DB header at position 0
        let mut db_header = vec![0u8; DB_HEADER_SIZE];
        temp_reader.read_exact(&mut db_header)?;
        writer.seek(SeekFrom::Start(0))?;
        writer.write_all(&db_header)?;

        // Stream the rest at start_offset
        writer.seek(SeekFrom::Start(header.start_offset))?;
        let mut buffer = vec![0u8; 256 * 1024];
        loop {
            let n = temp_reader.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            writer.write_all(&buffer[..n])?;
        }

        // Clean up temp file
        let _ = fs::remove_file(&temp_path);
    } else {
        // Full backup: stream directly to start_offset
        writer.seek(SeekFrom::Start(header.start_offset))?;
        decompress_stream(limited_reader, writer)?;
    }

    Ok(())
}

/// Apply a multi-part backup using streaming
fn apply_multipart_streaming<W: Write + Seek>(
    backup: &BackupInfo,
    first_header: &BackupHeader,
    writer: &mut W,
) -> Result<()> {
    let total_parts = first_header.total_parts;

    // Derive base path from first part
    let path_str = backup.path.to_string_lossy();
    let base_path = if path_str.ends_with(".001") {
        path_str.trim_end_matches(".001").to_string()
    } else {
        path_str.trim_end_matches(".ibak").to_string()
    };

    // Create a chained reader from all parts
    // We'll decompress to a temp file first, then process
    let temp_path = backup.path.with_extension("tmp");
    {
        let temp_file = File::create(&temp_path)?;
        let mut temp_writer = BufWriter::new(temp_file);

        // Create zstd decoder with streaming reader
        let chained_reader = create_multipart_reader(&base_path, total_parts)?;
        decompress_stream(chained_reader, &mut temp_writer)?;
    }

    // Process decompressed data
    let temp_size = fs::metadata(&temp_path)?.len();
    let mut temp_reader = BufReader::new(File::open(&temp_path)?);

    if backup.header.includes_db_header && temp_size > DB_HEADER_SIZE as u64 {
        // Read and write DB header at position 0
        let mut db_header = vec![0u8; DB_HEADER_SIZE];
        temp_reader.read_exact(&mut db_header)?;
        writer.seek(SeekFrom::Start(0))?;
        writer.write_all(&db_header)?;

        // Stream the rest at start_offset
        writer.seek(SeekFrom::Start(backup.header.start_offset))?;
        let mut buffer = vec![0u8; 256 * 1024];
        loop {
            let n = temp_reader.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            writer.write_all(&buffer[..n])?;
        }
    } else {
        // Full backup: stream to start_offset
        writer.seek(SeekFrom::Start(backup.header.start_offset))?;
        let mut buffer = vec![0u8; 256 * 1024];
        loop {
            let n = temp_reader.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            writer.write_all(&buffer[..n])?;
        }
    }

    // Clean up temp file
    let _ = fs::remove_file(&temp_path);

    Ok(())
}

/// Create a chained reader from multiple backup parts
fn create_multipart_reader(base_path: &str, total_parts: u8) -> Result<impl Read> {
    // We need to collect all parts into a single reader
    // This is tricky because we can't return a dynamic chain of readers
    // Instead, we'll create a custom reader that chains the parts

    // First, collect all the file handles and their sizes
    let mut readers: Vec<Box<dyn Read>> = Vec::with_capacity(total_parts as usize);

    for part_num in 1..=total_parts {
        let part_path = PathBuf::from(format!("{}.{:03}", base_path, part_num));

        if !part_path.exists() {
            return Err(BackupError::InvalidBackupFile {
                reason: format!("Missing part {}: {}", part_num, part_path.display()),
            });
        }

        let file = File::open(&part_path)?;
        let file_size = file.metadata()?.len();
        let mut reader = BufReader::new(file);

        // Skip header
        reader.seek(SeekFrom::Start(HEADER_SIZE as u64))?;

        // Take only the payload
        let payload_size = file_size - HEADER_SIZE as u64 - FOOTER_SIZE as u64;
        readers.push(Box::new(reader.take(payload_size)));
    }

    // Chain all readers together
    Ok(ChainedReader::new(readers))
}

/// A reader that chains multiple readers together
struct ChainedReader {
    readers: Vec<Box<dyn Read>>,
    current: usize,
}

impl ChainedReader {
    fn new(readers: Vec<Box<dyn Read>>) -> Self {
        Self {
            readers,
            current: 0,
        }
    }
}

impl Read for ChainedReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        while self.current < self.readers.len() {
            let n = self.readers[self.current].read(buf)?;
            if n > 0 {
                return Ok(n);
            }
            self.current += 1;
        }
        Ok(0)
    }
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
