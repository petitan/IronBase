// wal/entry.rs
// WAL entry types and serialization

use crate::error::{MongoLiteError, Result};
use crate::transaction::TransactionId;

/// Entry type in the WAL
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum WALEntryType {
    /// Transaction begin marker
    Begin = 0x01,
    /// Operation entry (insert/update/delete)
    Operation = 0x02,
    /// Transaction commit marker
    Commit = 0x03,
    /// Transaction abort marker
    Abort = 0x04,
    /// Index change entry (for atomic index updates)
    IndexChange = 0x05,
}

impl WALEntryType {
    pub fn from_u8(value: u8) -> Result<Self> {
        match value {
            0x01 => Ok(WALEntryType::Begin),
            0x02 => Ok(WALEntryType::Operation),
            0x03 => Ok(WALEntryType::Commit),
            0x04 => Ok(WALEntryType::Abort),
            0x05 => Ok(WALEntryType::IndexChange),
            _ => Err(MongoLiteError::WALCorruption),
        }
    }
}

/// A single entry in the Write-Ahead Log
///
/// Binary format:
/// - transaction_id: 8 bytes (u64 LE)
/// - entry_type: 1 byte
/// - data_len: 4 bytes (u32 LE)
/// - data: variable (JSON payload)
/// - checksum: 4 bytes (CRC32)
#[derive(Debug, Clone)]
pub struct WALEntry {
    pub transaction_id: TransactionId,
    pub entry_type: WALEntryType,
    pub data: Vec<u8>,
    pub checksum: u32,
}

/// Header size: 8 (tx_id) + 1 (type) + 4 (len) = 13 bytes
pub const WAL_HEADER_SIZE: usize = 13;

/// Maximum WAL entry size: 64MB (security limit)
pub const MAX_WAL_ENTRY_SIZE: usize = 64 * 1024 * 1024;

impl WALEntry {
    /// Create a new WAL entry with computed checksum
    pub fn new(transaction_id: TransactionId, entry_type: WALEntryType, data: Vec<u8>) -> Self {
        let mut entry = WALEntry {
            transaction_id,
            entry_type,
            data,
            checksum: 0,
        };
        entry.checksum = entry.compute_checksum();
        entry
    }

    /// Serialize entry to bytes
    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(WAL_HEADER_SIZE + self.data.len() + 4);

        // Transaction ID (8 bytes)
        buf.extend_from_slice(&self.transaction_id.to_le_bytes());

        // Entry Type (1 byte)
        buf.push(self.entry_type as u8);

        // Data Length (4 bytes)
        let data_len = self.data.len() as u32;
        buf.extend_from_slice(&data_len.to_le_bytes());

        // Data
        buf.extend_from_slice(&self.data);

        // Checksum (4 bytes)
        buf.extend_from_slice(&self.checksum.to_le_bytes());

        buf
    }

    /// Deserialize entry from bytes
    pub fn deserialize(data: &[u8]) -> Result<Self> {
        if data.len() < WAL_HEADER_SIZE + 4 {
            // Minimum: header + checksum
            return Err(MongoLiteError::WALCorruption);
        }

        let mut offset = 0;

        // Transaction ID
        let tx_id = u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap());
        offset += 8;

        // Entry Type
        let entry_type = WALEntryType::from_u8(data[offset])?;
        offset += 1;

        // Data Length
        let data_len = u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap()) as usize;
        offset += 4;

        // SECURITY: Prevent OOM from malformed WAL with huge data_len
        if data_len > MAX_WAL_ENTRY_SIZE {
            return Err(MongoLiteError::WALCorruption);
        }

        // Data
        if data.len() < offset + data_len + 4 {
            return Err(MongoLiteError::WALCorruption);
        }
        let entry_data = data[offset..offset + data_len].to_vec();
        offset += data_len;

        // Checksum
        let checksum = u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap());

        let entry = WALEntry {
            transaction_id: tx_id,
            entry_type,
            data: entry_data,
            checksum,
        };

        // Verify checksum
        if entry.compute_checksum() != checksum {
            return Err(MongoLiteError::WALCorruption);
        }

        Ok(entry)
    }

    /// Compute CRC32 checksum
    pub fn compute_checksum(&self) -> u32 {
        let mut hasher = crc32fast::Hasher::new();

        hasher.update(&self.transaction_id.to_le_bytes());
        hasher.update(&[self.entry_type as u8]);
        hasher.update(&(self.data.len() as u32).to_le_bytes());
        hasher.update(&self.data);

        hasher.finalize()
    }

    /// Verify entry checksum
    pub fn verify(&self) -> bool {
        self.compute_checksum() == self.checksum
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wal_entry_type_conversion() {
        assert_eq!(WALEntryType::from_u8(0x01).unwrap(), WALEntryType::Begin);
        assert_eq!(
            WALEntryType::from_u8(0x02).unwrap(),
            WALEntryType::Operation
        );
        assert_eq!(WALEntryType::from_u8(0x03).unwrap(), WALEntryType::Commit);
        assert_eq!(WALEntryType::from_u8(0x04).unwrap(), WALEntryType::Abort);
        assert_eq!(
            WALEntryType::from_u8(0x05).unwrap(),
            WALEntryType::IndexChange
        );
        assert!(WALEntryType::from_u8(0xFF).is_err());
    }

    #[test]
    fn test_wal_entry_serialize_deserialize() {
        let data = b"test data".to_vec();
        let entry = WALEntry::new(1, WALEntryType::Operation, data.clone());

        let serialized = entry.serialize();
        let deserialized = WALEntry::deserialize(&serialized).unwrap();

        assert_eq!(deserialized.transaction_id, 1);
        assert_eq!(deserialized.entry_type, WALEntryType::Operation);
        assert_eq!(deserialized.data, data);
        assert_eq!(deserialized.checksum, entry.checksum);
    }

    #[test]
    fn test_wal_entry_checksum_validation() {
        let entry = WALEntry::new(1, WALEntryType::Begin, vec![]);
        let mut serialized = entry.serialize();

        // Corrupt checksum
        let len = serialized.len();
        serialized[len - 1] ^= 0xFF;

        assert!(matches!(
            WALEntry::deserialize(&serialized),
            Err(MongoLiteError::WALCorruption)
        ));
    }

    #[test]
    fn test_wal_entry_verify() {
        let entry = WALEntry::new(42, WALEntryType::Commit, b"commit".to_vec());
        assert!(entry.verify());
    }
}
