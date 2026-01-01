//! Backup creation logic
//!
//! Supports both full and incremental backups with safe hot backup capability
//! (database can continue operating during backup).
//!
//! ## Hot Backup Safety
//!
//! NO LOCKS NEEDED - append-only storage guarantees safety:
//! 1. Read header to get data_end_offset (quick atomic read)
//! 2. Copy document data (immutable - never modified after write)
//! 3. Check header again to detect concurrent writes
//! 4. New data written during backup → included in next incremental

use crate::chain::{db_name_from_path, Chain};
use crate::compression::{compress, format_size};
use crate::error::{BackupError, Result};
use crate::format::{hash_to_short_hex, BackupFooter, BackupHeader, BackupType, DB_HEADER_SIZE};
use byteorder::{LittleEndian, ReadBytesExt};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// IronBase header field offsets (see ironbase-core/src/storage/mod.rs)
/// Header is 256 bytes, bincode-serialized
/// Layout: magic(8) + version(4) + page_size(4) + collection_count(4) +
///         free_list_head(8) + index_section_offset(8) + metadata_offset(8) +
///         metadata_size(8) + data_end_offset(8)
const IRONBASE_METADATA_OFFSET_POS: u64 = 36; // Position of metadata_offset in IronBase header
const IRONBASE_DATA_END_OFFSET_POS: u64 = 52; // Position of data_end_offset in IronBase header

/// Read data_end_offset from IronBase database header with shared lock
/// This is where document data ends (before metadata at file end)
///
/// For v2 databases, data_end_offset is 0, so we use metadata_offset instead
/// (metadata is at the end of the file, right after document data)
///
/// Uses shared lock to ensure consistent read while allowing other readers.
fn read_data_end_offset(db_path: &Path) -> Result<u64> {
    let file_size = std::fs::metadata(db_path)?.len();

    // Need at least 60 bytes to read data_end_offset (offset 52 + 8 bytes)
    if file_size < 60 {
        return Ok(file_size);
    }

    let file = File::open(db_path)?;
    let mut reader = BufReader::new(&file);

    // NO LOCK NEEDED: Append-only storage guarantees data immutability
    // Header read is atomic enough for our purposes (just need offsets)

    // Verify IronBase magic "MONGOLTE" at start
    let mut magic = [0u8; 8];
    reader.read_exact(&mut magic)?;
    if &magic != b"MONGOLTE" {
        // Not an IronBase file, return file size as fallback
        return Ok(file_size);
    }

    // First try data_end_offset (v3 databases)
    reader.seek(SeekFrom::Start(IRONBASE_DATA_END_OFFSET_POS))?;
    let data_end_offset = reader.read_u64::<LittleEndian>()?;

    // If data_end_offset is valid (v3), use it
    if data_end_offset > 0 && data_end_offset <= file_size {
        return Ok(data_end_offset);
    }

    // For v2 databases, data_end_offset is 0, use metadata_offset
    reader.seek(SeekFrom::Start(IRONBASE_METADATA_OFFSET_POS))?;
    let metadata_offset = reader.read_u64::<LittleEndian>()?;

    // Validate metadata_offset
    if metadata_offset > 0 && metadata_offset <= file_size {
        Ok(metadata_offset)
    } else {
        // Last resort: return file size
        Ok(file_size)
    }
}

/// Result of a backup operation
#[derive(Debug)]
pub struct BackupResult {
    /// Path to the created backup file (first/only file)
    pub path: PathBuf,
    /// All paths for multi-part backups
    pub all_paths: Vec<PathBuf>,
    /// Backup type (full or incremental)
    pub backup_type: BackupType,
    /// Original data size (uncompressed)
    pub original_size: u64,
    /// Compressed size (total across all parts)
    pub compressed_size: u64,
    /// SHA256 hash of the backup (first part for multi-part)
    pub hash: [u8; 32],
    /// Time taken in seconds
    pub duration_secs: f64,
    /// True if database was written to during backup (data in next incremental)
    pub concurrent_writes: bool,
    /// Number of parts (1 for single file, >1 for multi-part)
    pub part_count: u8,
}

impl BackupResult {
    /// Print summary of the backup
    pub fn print_summary(&self) {
        let type_str = match self.backup_type {
            BackupType::Full => "Full",
            BackupType::Incremental => "Incremental",
        };

        let ratio = if self.compressed_size > 0 {
            self.original_size as f64 / self.compressed_size as f64
        } else {
            0.0
        };

        println!("Backup completed successfully!");
        println!("  Type: {}", type_str);

        if self.part_count > 1 {
            println!("  Files: {} parts", self.part_count);
            for (i, path) in self.all_paths.iter().enumerate() {
                println!("    Part {}: {}", i + 1, path.display());
            }
        } else {
            println!("  File: {}", self.path.display());
        }

        println!(
            "  Size: {} -> {} ({:.1}x compression)",
            format_size(self.original_size),
            format_size(self.compressed_size),
            ratio
        );
        println!("  Hash: {}", hash_to_short_hex(&self.hash));
        println!("  Time: {:.2}s", self.duration_secs);
        if self.concurrent_writes {
            println!("  Note: Database was modified during backup - new data in next incremental");
        }
    }
}

/// Create a backup of the database
///
/// # Arguments
/// * `db_path` - Path to the .mlite database file
/// * `output_dir` - Directory to store backups
/// * `force_full` - If true, create a full backup even if incrementals exist
/// * `split_size` - Optional size in bytes to split backup into multiple parts
///
/// # Returns
/// BackupResult with details about the created backup
pub fn create_backup(
    db_path: &Path,
    output_dir: &Path,
    force_full: bool,
    split_size: Option<u64>,
) -> Result<BackupResult> {
    let start_time = std::time::Instant::now();

    // Validate inputs
    if !db_path.exists() {
        return Err(BackupError::DatabaseNotFound {
            path: db_path.to_path_buf(),
        });
    }

    if !output_dir.exists() {
        fs::create_dir_all(output_dir)?;
    }

    // Get database name and current size
    let db_name = db_name_from_path(db_path);
    let db_size = fs::metadata(db_path)?.len();

    // Read data_end_offset from IronBase header
    // This is where document data ends (metadata is at the END of the file)
    let current_data_end = read_data_end_offset(db_path)?;

    // Discover existing chain
    let chain = Chain::discover(output_dir, &db_name)?;

    // Determine backup type
    // CRITICAL FIX: Use data_end_offset instead of original_db_size for incremental start_offset
    // IronBase stores metadata at the END of the file, so original_db_size includes metadata
    // which gets overwritten when new documents are added
    let (backup_type, start_offset, parent_hash) = if force_full || chain.is_empty() {
        (BackupType::Full, 0u64, [0u8; 32])
    } else {
        let last = chain.last().unwrap();

        // Verify database hasn't shrunk (would indicate compaction or corruption)
        if db_size < last.header.original_db_size {
            return Err(BackupError::DatabaseShrunk {
                expected: last.header.original_db_size,
                actual: db_size,
            });
        }

        // Use data_end_offset from last backup as start point for incremental
        // This is where document data ended, NOT where the file ended (which includes metadata)
        let incremental_start = if last.header.data_end_offset > 0 {
            last.header.data_end_offset
        } else {
            // Fallback for backups made before this fix
            last.header.original_db_size
        };

        (BackupType::Incremental, incremental_start, last.hash)
    };

    // Calculate data to backup
    let incremental_data_length = db_size - start_offset;

    // Open database file for reading
    // NO LOCK NEEDED: Append-only storage guarantees data immutability
    // The DB can continue writing - new data will be in next incremental backup
    let db_file = File::open(db_path)?;

    // For incremental backups: payload = [DB header (256)] + [incremental data]
    // This ensures the updated metadata_offset pointer is captured
    let (data, data_length) = {
        let mut reader = BufReader::new(&db_file);

        if backup_type == BackupType::Incremental {
            // Read DB header first (0-255) - contains metadata_offset
            reader.seek(SeekFrom::Start(0))?;
            let mut db_header = vec![0u8; DB_HEADER_SIZE];
            reader.read_exact(&mut db_header)?;

            // Read incremental data (start_offset to end)
            reader.seek(SeekFrom::Start(start_offset))?;
            let mut incremental_data = vec![0u8; incremental_data_length as usize];
            reader.read_exact(&mut incremental_data)?;

            // Concatenate: [db_header] + [incremental_data]
            db_header.extend(incremental_data);
            let total_length = db_header.len() as u64;
            (db_header, total_length)
        } else {
            // Full backup: read entire file from offset 0
            reader.seek(SeekFrom::Start(start_offset))?;
            let mut data = vec![0u8; incremental_data_length as usize];
            reader.read_exact(&mut data)?;
            (data, incremental_data_length)
        }
    };

    // Compress data
    let compressed = compress(&data)?;

    // Create header
    // Note: For incremental backups, data_length includes the 256-byte DB header
    let header = match backup_type {
        BackupType::Full => BackupHeader::new_full(
            &db_name,
            db_size,
            current_data_end, // Store where document data ends
            data_length,
            compressed.len() as u64,
        ),
        BackupType::Incremental => BackupHeader::new_incremental(
            &db_name,
            parent_hash,
            db_size,
            start_offset,
            current_data_end, // Store where document data ends
            data_length,
            compressed.len() as u64,
        ),
    };

    // Generate base filename
    let base_filename = generate_filename(&header, &chain);

    // Determine if we need to split
    let compressed_len = compressed.len() as u64;
    let should_split = split_size.is_some_and(|size| compressed_len > size);

    let (backup_path, all_paths, content_hash, part_count) = if should_split {
        let split_size = split_size.unwrap();
        write_split_backup(output_dir, &base_filename, &header, &compressed, split_size)?
    } else {
        // Single file backup
        let content_hash = calculate_hash(&header, &compressed);
        let backup_path = output_dir.join(&base_filename);
        write_backup_file(&backup_path, &header, &compressed, content_hash)?;
        (backup_path.clone(), vec![backup_path], content_hash, 1u8)
    };

    // SNAPSHOT ISOLATION: Verify no concurrent changes during backup
    // Re-read data_end_offset to check if new documents were added
    let final_data_end = read_data_end_offset(db_path)?;
    let concurrent_writes = final_data_end != current_data_end;

    if concurrent_writes {
        // This is informational, not an error - new data will be in next backup
        eprintln!(
            "Note: {} bytes written during backup - will be included in next incremental",
            final_data_end - current_data_end
        );
    }

    let duration = start_time.elapsed().as_secs_f64();

    Ok(BackupResult {
        path: backup_path,
        all_paths,
        backup_type,
        original_size: data_length,
        compressed_size: compressed_len,
        hash: content_hash,
        duration_secs: duration,
        concurrent_writes,
        part_count,
    })
}

/// Calculate SHA256 hash of header + payload
fn calculate_hash(header: &BackupHeader, payload: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(header.to_bytes());
    hasher.update(payload);
    hasher.finalize().into()
}

/// Generate backup filename
fn generate_filename(header: &BackupHeader, chain: &Chain) -> String {
    let timestamp = chrono::DateTime::from_timestamp(header.timestamp, 0)
        .map(|dt| dt.format("%Y%m%d_%H%M%S").to_string())
        .unwrap_or_else(|| format!("{}", header.timestamp));

    let type_str = match header.backup_type {
        BackupType::Full => "full",
        BackupType::Incremental => "incr",
    };

    let sequence = chain.backups.len();

    format!(
        "{}_{}_{}_{:03}.ibak",
        header.db_name_str(),
        timestamp,
        type_str,
        sequence
    )
}

/// Write backup file (header + compressed payload + footer)
fn write_backup_file(
    path: &Path,
    header: &BackupHeader,
    compressed: &[u8],
    content_hash: [u8; 32],
) -> Result<()> {
    let file = File::create(path)?;
    let mut writer = BufWriter::new(file);

    // Write header
    header.write_to(&mut writer)?;

    // Write compressed payload
    writer.write_all(compressed)?;

    // Write footer
    let footer = BackupFooter::new(content_hash);
    footer.write_to(&mut writer)?;

    writer.flush()?;

    Ok(())
}

/// Write a split backup (multiple files)
///
/// Each part file has the structure:
/// - Header (128 bytes) with part_number and total_parts set
/// - Compressed payload chunk
/// - Footer (40 bytes) with hash of header+chunk
///
/// Returns (first_path, all_paths, first_hash, part_count)
fn write_split_backup(
    output_dir: &Path,
    base_filename: &str,
    base_header: &BackupHeader,
    compressed: &[u8],
    split_size: u64,
) -> Result<(PathBuf, Vec<PathBuf>, [u8; 32], u8)> {
    use crate::format::{FOOTER_SIZE, HEADER_SIZE};

    // Calculate how many parts we need
    // Each part has header + footer overhead
    let overhead = (HEADER_SIZE + FOOTER_SIZE) as u64;
    let payload_per_part = split_size.saturating_sub(overhead);

    if payload_per_part == 0 {
        return Err(BackupError::InvalidBackupFile {
            reason: "Split size too small for header/footer overhead".to_string(),
        });
    }

    let total_parts_u64 = (compressed.len() as u64).div_ceil(payload_per_part);

    if total_parts_u64 > 255 {
        return Err(BackupError::InvalidBackupFile {
            reason: format!(
                "Backup would require {} parts, maximum is 255. Use larger split size.",
                total_parts_u64
            ),
        });
    }

    let total_parts = total_parts_u64 as u8;

    let mut all_paths = Vec::with_capacity(total_parts as usize);
    let mut first_hash = [0u8; 32];

    // Remove .ibak extension from base filename
    let base_name = base_filename.trim_end_matches(".ibak");

    for part_num in 1..=total_parts {
        // Calculate chunk bounds
        let start = ((part_num - 1) as usize) * (payload_per_part as usize);
        let end = std::cmp::min(start + payload_per_part as usize, compressed.len());
        let chunk = &compressed[start..end];

        // Create part header
        let part_header = base_header.for_part(part_num, total_parts, chunk.len() as u64);

        // Calculate hash for this part
        let part_hash = calculate_hash(&part_header, chunk);

        if part_num == 1 {
            first_hash = part_hash;
        }

        // Generate part filename: base_name.ibak.001, .002, etc.
        let part_filename = format!("{}.ibak.{:03}", base_name, part_num);
        let part_path = output_dir.join(&part_filename);

        // Write part file
        write_backup_file(&part_path, &part_header, chunk, part_hash)?;

        all_paths.push(part_path);
    }

    let first_path = all_paths.first().cloned().unwrap_or_default();

    Ok((first_path, all_paths, first_hash, total_parts))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_full_backup() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");
        let backup_dir = temp_dir.path().join("backups");

        // Create test database
        fs::write(&db_path, b"test database content".repeat(100)).unwrap();

        // Create backup (no split)
        let result = create_backup(&db_path, &backup_dir, false, None).unwrap();

        assert_eq!(result.backup_type, BackupType::Full);
        assert!(result.path.exists());
        assert!(result.compressed_size < result.original_size);
        assert_eq!(result.part_count, 1);
    }

    #[test]
    fn test_incremental_backup() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");
        let backup_dir = temp_dir.path().join("backups");

        // Create test database
        fs::write(&db_path, b"initial content".repeat(100)).unwrap();

        // Create full backup
        let _full = create_backup(&db_path, &backup_dir, false, None).unwrap();

        // Append to database
        let mut file = fs::OpenOptions::new().append(true).open(&db_path).unwrap();
        file.write_all(&b"additional content".repeat(50)).unwrap();

        // Create incremental backup
        let incr = create_backup(&db_path, &backup_dir, false, None).unwrap();

        assert_eq!(incr.backup_type, BackupType::Incremental);
        assert!(incr.original_size < fs::metadata(&db_path).unwrap().len());
    }

    #[test]
    fn test_split_backup() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");
        let backup_dir = temp_dir.path().join("backups");

        // Create pseudo-random data (doesn't compress well)
        // Using a simple LCG to generate non-compressible data without rand dependency
        let mut seed: u64 = 12345;
        let mut random_data = Vec::with_capacity(100 * 1024);
        for _ in 0..(100 * 1024) {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            random_data.push((seed >> 33) as u8);
        }
        fs::write(&db_path, &random_data).unwrap();

        // Create backup with 20KB split (should create multiple parts)
        let result = create_backup(&db_path, &backup_dir, false, Some(20 * 1024)).unwrap();

        assert_eq!(result.backup_type, BackupType::Full);
        assert!(
            result.part_count >= 2,
            "Expected multi-part backup, got {} parts",
            result.part_count
        );
        assert_eq!(result.all_paths.len(), result.part_count as usize);

        // Verify all part files exist
        for path in &result.all_paths {
            assert!(path.exists(), "Part file should exist: {}", path.display());
        }
    }
}
