//! Backup creation logic
//!
//! Supports both full and incremental backups with safe hot backup capability
//! (database can continue operating during backup).
//!
//! ## Hot Backup Safety
//!
//! Uses snapshot isolation for safe hot backups:
//! 1. Shared lock (brief) to read metadata consistently
//! 2. Lock released - DB can continue writing
//! 3. Document data copied (immutable due to append-only storage)
//! 4. Final header check to detect concurrent changes

use crate::chain::{db_name_from_path, Chain};
use crate::compression::{compress, format_size};
use crate::error::{BackupError, Result};
use crate::format::{hash_to_short_hex, BackupFooter, BackupHeader, BackupType, DB_HEADER_SIZE};
use byteorder::{LittleEndian, ReadBytesExt};
#[allow(unused_imports)] // Required for lock_shared/unlock trait methods
use fs2::FileExt;
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

    // Acquire shared lock for consistent header read
    // This blocks writers but allows other readers
    file.lock_shared().map_err(|e| {
        BackupError::Io(std::io::Error::new(
            std::io::ErrorKind::WouldBlock,
            format!(
                "Cannot acquire lock on database (is it being written?): {}",
                e
            ),
        ))
    })?;

    let mut reader = BufReader::new(&file);

    // Verify IronBase magic "MONGOLTE" at start
    let mut magic = [0u8; 8];
    reader.read_exact(&mut magic)?;
    if &magic != b"MONGOLTE" {
        // Not an IronBase file, release lock and return file size as fallback
        let _ = file.unlock();
        return Ok(file_size);
    }

    // First try data_end_offset (v3 databases)
    reader.seek(SeekFrom::Start(IRONBASE_DATA_END_OFFSET_POS))?;
    let data_end_offset = reader.read_u64::<LittleEndian>()?;

    // Release lock - we have what we need
    let _ = file.unlock();

    // If data_end_offset is valid (v3), use it
    if data_end_offset > 0 && data_end_offset <= file_size {
        return Ok(data_end_offset);
    }

    // For v2 databases, data_end_offset is 0
    // Re-acquire lock to read metadata_offset
    file.lock_shared().map_err(|e| {
        BackupError::Io(std::io::Error::new(
            std::io::ErrorKind::WouldBlock,
            format!("Cannot acquire lock on database: {}", e),
        ))
    })?;

    let mut reader = BufReader::new(&file);
    reader.seek(SeekFrom::Start(IRONBASE_METADATA_OFFSET_POS))?;
    let metadata_offset = reader.read_u64::<LittleEndian>()?;

    let _ = file.unlock();

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
    /// Path to the created backup file
    pub path: PathBuf,
    /// Backup type (full or incremental)
    pub backup_type: BackupType,
    /// Original data size (uncompressed)
    pub original_size: u64,
    /// Compressed size
    pub compressed_size: u64,
    /// SHA256 hash of the backup
    pub hash: [u8; 32],
    /// Time taken in seconds
    pub duration_secs: f64,
    /// True if database was written to during backup (data in next incremental)
    pub concurrent_writes: bool,
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
        println!("  File: {}", self.path.display());
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
///
/// # Returns
/// BackupResult with details about the created backup
pub fn create_backup(db_path: &Path, output_dir: &Path, force_full: bool) -> Result<BackupResult> {
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
    let db_file = File::open(db_path)?;

    // SNAPSHOT ISOLATION: Acquire shared lock for consistent read
    // This allows other readers but blocks writers during our read
    db_file.lock_shared().map_err(|e| {
        BackupError::Io(std::io::Error::new(
            std::io::ErrorKind::WouldBlock,
            format!("Cannot acquire lock on database for backup: {}", e),
        ))
    })?;

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

    // Release lock - data is copied, DB can continue writing
    // Note: Due to append-only storage, the data we read is immutable
    let _ = db_file.unlock();

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

    // Calculate content hash (header + compressed payload)
    let content_hash = calculate_hash(&header, &compressed);

    // Generate filename
    let filename = generate_filename(&header, &chain);
    let backup_path = output_dir.join(&filename);

    // Write backup file
    write_backup_file(&backup_path, &header, &compressed, content_hash)?;

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
        backup_type,
        original_size: data_length,
        compressed_size: compressed.len() as u64,
        hash: content_hash,
        duration_secs: duration,
        concurrent_writes,
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

        // Create backup
        let result = create_backup(&db_path, &backup_dir, false).unwrap();

        assert_eq!(result.backup_type, BackupType::Full);
        assert!(result.path.exists());
        assert!(result.compressed_size < result.original_size);
    }

    #[test]
    fn test_incremental_backup() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");
        let backup_dir = temp_dir.path().join("backups");

        // Create test database
        fs::write(&db_path, b"initial content".repeat(100)).unwrap();

        // Create full backup
        let _full = create_backup(&db_path, &backup_dir, false).unwrap();

        // Append to database
        let mut file = fs::OpenOptions::new().append(true).open(&db_path).unwrap();
        file.write_all(&b"additional content".repeat(50)).unwrap();

        // Create incremental backup
        let incr = create_backup(&db_path, &backup_dir, false).unwrap();

        assert_eq!(incr.backup_type, BackupType::Incremental);
        assert!(incr.original_size < fs::metadata(&db_path).unwrap().len());
    }
}
