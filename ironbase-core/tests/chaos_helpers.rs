// chaos_helpers.rs
// Utility functions for chaos/corruption testing

#![allow(dead_code)]

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

/// Overwrite bytes at specific offset in a file
pub fn corrupt_bytes_at(path: &Path, offset: u64, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = OpenOptions::new().write(true).open(path)?;
    file.seek(SeekFrom::Start(offset))?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// Flip a specific bit in a file at given offset
pub fn corrupt_bit(path: &Path, offset: u64, bit: u8) -> std::io::Result<()> {
    let mut file = OpenOptions::new().read(true).write(true).open(path)?;
    file.seek(SeekFrom::Start(offset))?;
    let mut byte = [0u8; 1];
    file.read_exact(&mut byte)?;
    byte[0] ^= 1 << bit;
    file.seek(SeekFrom::Start(offset))?;
    file.write_all(&byte)?;
    file.sync_all()
}

/// Truncate file to specified length
pub fn truncate_file(path: &Path, len: u64) -> std::io::Result<()> {
    let file = OpenOptions::new().write(true).open(path)?;
    file.set_len(len)?;
    file.sync_all()
}

/// Append garbage bytes to end of file
pub fn append_garbage(path: &Path, garbage: &[u8]) -> std::io::Result<u64> {
    let mut file = OpenOptions::new().append(true).open(path)?;
    let offset = file.seek(SeekFrom::End(0))?;
    file.write_all(garbage)?;
    file.sync_all()?;
    Ok(offset)
}

/// Read bytes from file at specific offset
pub fn read_bytes_at(path: &Path, offset: u64, len: usize) -> std::io::Result<Vec<u8>> {
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(offset))?;
    let mut buffer = vec![0u8; len];
    file.read_exact(&mut buffer)?;
    Ok(buffer)
}

/// Get file length
pub fn file_len(path: &Path) -> std::io::Result<u64> {
    Ok(std::fs::metadata(path)?.len())
}

/// Write a partial WAL entry (simulating crash mid-write)
pub fn write_partial_wal_entry(
    wal_path: &Path,
    tx_id: u64,
    entry_type: u8,
    data: &[u8],
    truncate_at: usize, // How many bytes to write before "crash"
) -> std::io::Result<u64> {
    let mut file = OpenOptions::new().append(true).open(wal_path)?;
    let offset = file.seek(SeekFrom::End(0))?;

    // Build full entry
    let mut entry = Vec::new();
    entry.extend_from_slice(&tx_id.to_le_bytes()); // 8 bytes
    entry.push(entry_type); // 1 byte
    entry.extend_from_slice(&(data.len() as u32).to_le_bytes()); // 4 bytes
    entry.extend_from_slice(data); // variable

    // Compute CRC32 (same algorithm as WAL)
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(&tx_id.to_le_bytes());
    hasher.update(&[entry_type]);
    hasher.update(&(data.len() as u32).to_le_bytes());
    hasher.update(data);
    let checksum = hasher.finalize();
    entry.extend_from_slice(&checksum.to_le_bytes()); // 4 bytes

    // Only write partial entry (simulating crash)
    let bytes_to_write = truncate_at.min(entry.len());
    file.write_all(&entry[..bytes_to_write])?;
    file.sync_all()?;

    Ok(offset)
}

/// Write a complete WAL entry with corrupted checksum
pub fn write_wal_entry_bad_crc(
    wal_path: &Path,
    tx_id: u64,
    entry_type: u8,
    data: &[u8],
) -> std::io::Result<u64> {
    let mut file = OpenOptions::new().append(true).open(wal_path)?;
    let offset = file.seek(SeekFrom::End(0))?;

    // Build entry
    file.write_all(&tx_id.to_le_bytes())?;
    file.write_all(&[entry_type])?;
    file.write_all(&(data.len() as u32).to_le_bytes())?;
    file.write_all(data)?;
    // Write bad checksum
    file.write_all(&0xDEADBEEFu32.to_le_bytes())?;
    file.sync_all()?;

    Ok(offset)
}

/// Write a document with partial data (simulating crash during write)
pub fn write_partial_document(
    path: &Path,
    data: &[u8],
    truncate_at: usize,
) -> std::io::Result<u64> {
    let mut file = OpenOptions::new().append(true).open(path)?;
    let offset = file.seek(SeekFrom::End(0))?;

    // Build full document block: [u32 len][data]
    let mut block = Vec::new();
    block.extend_from_slice(&(data.len() as u32).to_le_bytes());
    block.extend_from_slice(data);

    // Only write partial block
    let bytes_to_write = truncate_at.min(block.len());
    file.write_all(&block[..bytes_to_write])?;
    file.sync_all()?;

    Ok(offset)
}

/// Write a document length header only (no data - simulating crash after length write)
pub fn write_length_header_only(path: &Path, claimed_length: u32) -> std::io::Result<u64> {
    let mut file = OpenOptions::new().append(true).open(path)?;
    let offset = file.seek(SeekFrom::End(0))?;
    file.write_all(&claimed_length.to_le_bytes())?;
    file.sync_all()?;
    Ok(offset)
}

/// Integrity verification result
#[derive(Debug, Default)]
pub struct IntegrityReport {
    pub readable_documents: usize,
    pub corrupted_documents: usize,
    pub orphan_offsets: Vec<u64>,
    pub catalog_mismatches: usize,
    pub errors: Vec<String>,
}

impl IntegrityReport {
    pub fn is_clean(&self) -> bool {
        self.corrupted_documents == 0
            && self.orphan_offsets.is_empty()
            && self.catalog_mismatches == 0
            && self.errors.is_empty()
    }
}

/// Constants for file format
pub mod format {
    pub const HEADER_SIZE: u64 = 256;
    pub const MAGIC: &[u8; 8] = b"MONGOLTE";

    // WAL entry types
    pub const WAL_BEGIN: u8 = 0x01;
    pub const WAL_OPERATION: u8 = 0x02;
    pub const WAL_COMMIT: u8 = 0x03;
    pub const WAL_ABORT: u8 = 0x04;
    pub const WAL_INDEX_CHANGE: u8 = 0x05;
}
