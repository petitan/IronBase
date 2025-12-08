//! Backup file format definitions
//!
//! File structure:
//! ```text
//! ┌─────────────────────────────────────────────────────────┐
//! │ HEADER (128 bytes, fixed)                               │
//! ├─────────────────────────────────────────────────────────┤
//! │ Magic: "IRONBAK\0" (8 bytes)                            │
//! │ Version: u16 (2 bytes)                                  │
//! │ Type: u8 (0=Full, 1=Incremental)                        │
//! │ Compression: u8 (0=None, 1=Zstd)                        │
//! │ Timestamp: i64 (8 bytes, Unix epoch)                    │
//! │ Parent Hash: [u8; 32] (SHA256, zeros for Full)          │
//! │ Original DB Size: u64 (8 bytes)                         │
//! │ Start Offset: u64 (8 bytes, 0 for Full)                 │
//! │ Data Length: u64 (8 bytes, uncompressed)                │
//! │ Compressed Length: u64 (8 bytes)                        │
//! │ DB Name: [u8; 32] (database identifier)                 │
//! │ Reserved: [u8; 14] (future use)                         │
//! ├─────────────────────────────────────────────────────────┤
//! │ PAYLOAD (variable)                                      │
//! │ - Compressed data bytes (zstd level 3)                  │
//! ├─────────────────────────────────────────────────────────┤
//! │ FOOTER (40 bytes)                                       │
//! │ - Content Hash: [u8; 32] (SHA256 of header+payload)     │
//! │ - Footer Magic: "BAKEND\0\0" (8 bytes)                  │
//! └─────────────────────────────────────────────────────────┘
//! ```

use crate::error::{BackupError, Result};
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use std::io::{Read, Write};

/// Magic number at start of backup file
pub const MAGIC: &[u8; 8] = b"IRONBAK\0";

/// Magic number at end of backup file (footer)
pub const FOOTER_MAGIC: &[u8; 8] = b"BAKEND\0\0";

/// Current backup format version
pub const VERSION: u16 = 1;

/// Fixed header size in bytes
pub const HEADER_SIZE: usize = 128;

/// Fixed footer size in bytes
pub const FOOTER_SIZE: usize = 40;

/// IronBase database header size (for incremental backups that include it)
pub const DB_HEADER_SIZE: usize = 256;

/// Type of backup
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum BackupType {
    /// Full database backup (offset = 0)
    Full = 0,
    /// Incremental backup (only new bytes since last backup)
    Incremental = 1,
}

impl BackupType {
    pub fn from_u8(v: u8) -> Result<Self> {
        match v {
            0 => Ok(BackupType::Full),
            1 => Ok(BackupType::Incremental),
            _ => Err(BackupError::InvalidBackupFile {
                reason: format!("Invalid backup type: {}", v),
            }),
        }
    }
}

/// Compression algorithm used
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Compression {
    /// No compression
    None = 0,
    /// Zstd compression
    Zstd = 1,
}

impl Compression {
    pub fn from_u8(v: u8) -> Result<Self> {
        match v {
            0 => Ok(Compression::None),
            1 => Ok(Compression::Zstd),
            _ => Err(BackupError::InvalidBackupFile {
                reason: format!("Invalid compression type: {}", v),
            }),
        }
    }
}

/// Backup file header (128 bytes)
#[derive(Debug, Clone)]
pub struct BackupHeader {
    /// Backup type (full or incremental)
    pub backup_type: BackupType,
    /// Compression algorithm
    pub compression: Compression,
    /// Unix timestamp when backup was created
    pub timestamp: i64,
    /// SHA256 hash of parent backup (zeros for full backup)
    pub parent_hash: [u8; 32],
    /// Original database file size at backup time
    pub original_db_size: u64,
    /// Start offset in database file (0 for full backup)
    pub start_offset: u64,
    /// Uncompressed data length
    pub data_length: u64,
    /// Compressed data length
    pub compressed_length: u64,
    /// Database name (null-terminated, max 31 chars)
    pub db_name: [u8; 32],
    /// Whether the payload includes the 256-byte DB header (for incremental backups)
    /// When true, payload = [db_header(256)] + [incremental_data]
    pub includes_db_header: bool,
    /// End of document data in database (from IronBase v3 header.data_end_offset)
    /// This is where incremental backups should start, NOT original_db_size
    /// because IronBase stores metadata at the END of the file
    pub data_end_offset: u64,
}

impl BackupHeader {
    /// Create a new header for a full backup
    pub fn new_full(
        db_name: &str,
        db_size: u64,
        data_end_offset: u64,
        data_length: u64,
        compressed_length: u64,
    ) -> Self {
        let mut name_bytes = [0u8; 32];
        let name = db_name.as_bytes();
        let len = name.len().min(31);
        name_bytes[..len].copy_from_slice(&name[..len]);

        Self {
            backup_type: BackupType::Full,
            compression: Compression::Zstd,
            timestamp: chrono::Utc::now().timestamp(),
            parent_hash: [0u8; 32],
            original_db_size: db_size,
            start_offset: 0,
            data_length,
            compressed_length,
            db_name: name_bytes,
            includes_db_header: false, // Full backup already includes header at offset 0
            data_end_offset,           // Where document data ends (before metadata)
        }
    }

    /// Create a new header for an incremental backup
    /// Note: includes_db_header is always true for incremental backups (v1.0.10+)
    /// because IronBase's dynamic metadata requires the updated header
    pub fn new_incremental(
        db_name: &str,
        parent_hash: [u8; 32],
        db_size: u64,
        start_offset: u64,
        data_end_offset: u64,
        data_length: u64,
        compressed_length: u64,
    ) -> Self {
        let mut name_bytes = [0u8; 32];
        let name = db_name.as_bytes();
        let len = name.len().min(31);
        name_bytes[..len].copy_from_slice(&name[..len]);

        Self {
            backup_type: BackupType::Incremental,
            compression: Compression::Zstd,
            timestamp: chrono::Utc::now().timestamp(),
            parent_hash,
            original_db_size: db_size,
            start_offset,
            data_length,
            compressed_length,
            db_name: name_bytes,
            includes_db_header: true, // Always include DB header for incremental backups
            data_end_offset,          // Where document data ends (before metadata)
        }
    }

    /// Get database name as string
    pub fn db_name_str(&self) -> &str {
        let end = self.db_name.iter().position(|&b| b == 0).unwrap_or(32);
        std::str::from_utf8(&self.db_name[..end]).unwrap_or("unknown")
    }

    /// Write header to a writer
    pub fn write_to<W: Write>(&self, writer: &mut W) -> Result<()> {
        // Magic (8 bytes)
        writer.write_all(MAGIC)?;

        // Version (2 bytes)
        writer.write_u16::<LittleEndian>(VERSION)?;

        // Type (1 byte)
        writer.write_u8(self.backup_type as u8)?;

        // Compression (1 byte)
        writer.write_u8(self.compression as u8)?;

        // Timestamp (8 bytes)
        writer.write_i64::<LittleEndian>(self.timestamp)?;

        // Parent hash (32 bytes)
        writer.write_all(&self.parent_hash)?;

        // Original DB size (8 bytes)
        writer.write_u64::<LittleEndian>(self.original_db_size)?;

        // Start offset (8 bytes)
        writer.write_u64::<LittleEndian>(self.start_offset)?;

        // Data length (8 bytes)
        writer.write_u64::<LittleEndian>(self.data_length)?;

        // Compressed length (8 bytes)
        writer.write_u64::<LittleEndian>(self.compressed_length)?;

        // DB name (32 bytes)
        writer.write_all(&self.db_name)?;

        // Flags (1 byte) - uses first reserved byte
        // Bit 0: includes_db_header
        let flags: u8 = if self.includes_db_header { 1 } else { 0 };
        writer.write_u8(flags)?;

        // Data end offset (8 bytes) - where document data ends in DB file
        // This is critical for incremental backups - tells us where new data starts
        writer.write_u64::<LittleEndian>(self.data_end_offset)?;

        // Reserved (3 bytes to reach 128 total)
        // 8+2+1+1+8+32+8+8+8+8+32+1+8 = 125, so 3 more bytes needed
        writer.write_all(&[0u8; 3])?;

        Ok(())
    }

    /// Read header from a reader
    pub fn read_from<R: Read>(reader: &mut R) -> Result<Self> {
        // Magic (8 bytes)
        let mut magic = [0u8; 8];
        reader.read_exact(&mut magic)?;
        if &magic != MAGIC {
            return Err(BackupError::InvalidMagic);
        }

        // Version (2 bytes)
        let version = reader.read_u16::<LittleEndian>()?;
        if version != VERSION {
            return Err(BackupError::UnsupportedVersion { version });
        }

        // Type (1 byte)
        let backup_type = BackupType::from_u8(reader.read_u8()?)?;

        // Compression (1 byte)
        let compression = Compression::from_u8(reader.read_u8()?)?;

        // Timestamp (8 bytes)
        let timestamp = reader.read_i64::<LittleEndian>()?;

        // Parent hash (32 bytes)
        let mut parent_hash = [0u8; 32];
        reader.read_exact(&mut parent_hash)?;

        // Original DB size (8 bytes)
        let original_db_size = reader.read_u64::<LittleEndian>()?;

        // Start offset (8 bytes)
        let start_offset = reader.read_u64::<LittleEndian>()?;

        // Data length (8 bytes)
        let data_length = reader.read_u64::<LittleEndian>()?;

        // Compressed length (8 bytes)
        let compressed_length = reader.read_u64::<LittleEndian>()?;

        // DB name (32 bytes)
        let mut db_name = [0u8; 32];
        reader.read_exact(&mut db_name)?;

        // Flags (1 byte)
        let flags = reader.read_u8()?;
        let includes_db_header = (flags & 1) != 0;

        // Data end offset (8 bytes) - where document data ends in DB file
        let data_end_offset = reader.read_u64::<LittleEndian>()?;

        // Reserved (3 bytes)
        let mut _reserved = [0u8; 3];
        reader.read_exact(&mut _reserved)?;

        Ok(Self {
            backup_type,
            compression,
            timestamp,
            parent_hash,
            original_db_size,
            start_offset,
            data_length,
            compressed_length,
            db_name,
            includes_db_header,
            data_end_offset,
        })
    }

    /// Serialize header to bytes (for hashing)
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(HEADER_SIZE);
        self.write_to(&mut buf).expect("write to vec cannot fail");
        buf
    }
}

/// Backup file footer (40 bytes)
#[derive(Debug, Clone)]
pub struct BackupFooter {
    /// SHA256 hash of header + payload
    pub content_hash: [u8; 32],
}

impl BackupFooter {
    /// Create a new footer with the given content hash
    pub fn new(content_hash: [u8; 32]) -> Self {
        Self { content_hash }
    }

    /// Write footer to a writer
    pub fn write_to<W: Write>(&self, writer: &mut W) -> Result<()> {
        writer.write_all(&self.content_hash)?;
        writer.write_all(FOOTER_MAGIC)?;
        Ok(())
    }

    /// Read footer from a reader
    pub fn read_from<R: Read>(reader: &mut R) -> Result<Self> {
        let mut content_hash = [0u8; 32];
        reader.read_exact(&mut content_hash)?;

        let mut magic = [0u8; 8];
        reader.read_exact(&mut magic)?;

        if &magic != FOOTER_MAGIC {
            return Err(BackupError::InvalidBackupFile {
                reason: "Invalid footer magic".to_string(),
            });
        }

        Ok(Self { content_hash })
    }
}

/// Format hash as hex string (first 8 chars)
pub fn hash_to_short_hex(hash: &[u8; 32]) -> String {
    hex::encode(&hash[..4])
}

/// Format hash as full hex string
pub fn hash_to_hex(hash: &[u8; 32]) -> String {
    hex::encode(hash)
}

// Simple hex encoding (no external dependency)
mod hex {
    pub fn encode(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{:02x}", b)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_header_roundtrip() {
        // new_full now takes: db_name, db_size, data_end_offset, data_length, compressed_length
        let header = BackupHeader::new_full("test_db", 1024 * 1024, 900000, 1000000, 250000);

        let mut buf = Vec::new();
        header.write_to(&mut buf).unwrap();

        assert_eq!(buf.len(), HEADER_SIZE);

        let mut cursor = Cursor::new(buf);
        let read_header = BackupHeader::read_from(&mut cursor).unwrap();

        assert_eq!(read_header.backup_type, BackupType::Full);
        assert_eq!(read_header.db_name_str(), "test_db");
        assert_eq!(read_header.original_db_size, 1024 * 1024);
        assert_eq!(read_header.data_end_offset, 900000);
        assert_eq!(read_header.data_length, 1000000);
        assert_eq!(read_header.compressed_length, 250000);
    }

    #[test]
    fn test_footer_roundtrip() {
        let hash = [0xABu8; 32];
        let footer = BackupFooter::new(hash);

        let mut buf = Vec::new();
        footer.write_to(&mut buf).unwrap();

        assert_eq!(buf.len(), FOOTER_SIZE);

        let mut cursor = Cursor::new(buf);
        let read_footer = BackupFooter::read_from(&mut cursor).unwrap();

        assert_eq!(read_footer.content_hash, hash);
    }
}
