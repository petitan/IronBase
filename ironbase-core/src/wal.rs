// ironbase-core/src/wal.rs
// Write-Ahead Log (WAL) for transaction durability

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::error::{Result, MongoLiteError};
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
    fn from_u8(value: u8) -> Result<Self> {
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
#[derive(Debug, Clone)]
pub struct WALEntry {
    pub transaction_id: TransactionId,
    pub entry_type: WALEntryType,
    pub data: Vec<u8>,
    pub checksum: u32,
}

impl WALEntry {
    /// Create a new WAL entry
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
        let mut buf = Vec::new();

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
        if data.len() < 17 {
            // Minimum: 8 (tx_id) + 1 (type) + 4 (len) + 0 (data) + 4 (checksum)
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
    fn compute_checksum(&self) -> u32 {
        let mut hasher = crc32fast::Hasher::new();

        hasher.update(&self.transaction_id.to_le_bytes());
        hasher.update(&[self.entry_type as u8]);
        hasher.update(&(self.data.len() as u32).to_le_bytes());
        hasher.update(&self.data);

        hasher.finalize()
    }
}

/// Write-Ahead Log file manager
pub struct WriteAheadLog {
    file: File,
    path: PathBuf,
}

impl WriteAheadLog {
    /// Open or create a WAL file
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();

        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .append(true)
            .open(&path)?;

        Ok(WriteAheadLog { file, path })
    }

    /// Append an entry to the WAL
    pub fn append(&mut self, entry: &WALEntry) -> Result<u64> {
        let serialized = entry.serialize();
        let offset = self.file.seek(SeekFrom::End(0))?;
        self.file.write_all(&serialized)?;
        Ok(offset)
    }

    /// Flush WAL to disk (fsync)
    pub fn flush(&mut self) -> Result<()> {
        self.file.sync_all()?;
        Ok(())
    }

    /// Recover transactions from WAL
    /// Returns grouped transactions (only committed ones)
    pub fn recover(&mut self) -> Result<Vec<Vec<WALEntry>>> {
        self.file.seek(SeekFrom::Start(0))?;

        let mut entries = Vec::new();

        // Read all entries
        loop {
            match self.read_next_entry() {
                Ok(entry) => entries.push(entry),
                Err(MongoLiteError::Io(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    break;  // End of file
                }
                Err(e) => return Err(e),
            }
        }

        // Group entries by transaction ID
        use std::collections::HashMap;
        let mut txs: HashMap<TransactionId, Vec<WALEntry>> = HashMap::new();
        for entry in entries {
            txs.entry(entry.transaction_id)
                .or_insert_with(Vec::new)
                .push(entry);
        }

        // Filter to committed transactions only
        let mut committed = Vec::new();
        for (_tx_id, tx_entries) in txs {
            // Check if last entry is COMMIT
            if let Some(last) = tx_entries.last() {
                if last.entry_type == WALEntryType::Commit {
                    committed.push(tx_entries);
                }
            }
            // Else: uncommitted or aborted transaction, discard
        }

        Ok(committed)
    }

    /// Read next entry from current position
    fn read_next_entry(&mut self) -> Result<WALEntry> {
        // Read header: 8 (tx_id) + 1 (type) + 4 (len) = 13 bytes
        let mut header = [0u8; 13];
        self.file.read_exact(&mut header)?;

        let tx_id = u64::from_le_bytes(header[0..8].try_into().unwrap());
        let entry_type = WALEntryType::from_u8(header[8])?;
        let data_len = u32::from_le_bytes(header[9..13].try_into().unwrap()) as usize;

        // Read data
        let mut data = vec![0u8; data_len];
        self.file.read_exact(&mut data)?;

        // Read checksum
        let mut checksum_bytes = [0u8; 4];
        self.file.read_exact(&mut checksum_bytes)?;
        let checksum = u32::from_le_bytes(checksum_bytes);

        let entry = WALEntry {
            transaction_id: tx_id,
            entry_type,
            data,
            checksum,
        };

        // Verify checksum
        if entry.compute_checksum() != checksum {
            return Err(MongoLiteError::WALCorruption);
        }

        Ok(entry)
    }

    /// Clear WAL file (after successful recovery)
    pub fn clear(&mut self) -> Result<()> {
        self.file.set_len(0)?;
        self.file.seek(SeekFrom::Start(0))?;
        self.flush()?;
        Ok(())
    }

    /// Checkpoint: remove committed transactions from WAL
    pub fn checkpoint(&mut self, committed_tx_ids: &[TransactionId]) -> Result<()> {
        // Read all entries
        self.file.seek(SeekFrom::Start(0))?;
        let mut all_entries = Vec::new();

        loop {
            match self.read_next_entry() {
                Ok(entry) => all_entries.push(entry),
                Err(MongoLiteError::Io(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    break;
                }
                Err(e) => return Err(e),
            }
        }

        // Keep only uncommitted transactions
        let active_entries: Vec<_> = all_entries
            .into_iter()
            .filter(|e| !committed_tx_ids.contains(&e.transaction_id))
            .collect();

        // Rewrite WAL file
        let temp_path = self.path.with_extension("wal.tmp");
        let mut temp_file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&temp_path)?;

        for entry in active_entries {
            temp_file.write_all(&entry.serialize())?;
        }
        temp_file.sync_all()?;
        drop(temp_file);

        // Atomic rename
        std::fs::rename(&temp_path, &self.path)?;

        // Reopen file
        self.file = OpenOptions::new()
            .read(true)
            .write(true)
            .append(true)
            .open(&self.path)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wal_entry_type_conversion() {
        assert_eq!(WALEntryType::from_u8(0x01).unwrap(), WALEntryType::Begin);
        assert_eq!(WALEntryType::from_u8(0x02).unwrap(), WALEntryType::Operation);
        assert_eq!(WALEntryType::from_u8(0x03).unwrap(), WALEntryType::Commit);
        assert_eq!(WALEntryType::from_u8(0x04).unwrap(), WALEntryType::Abort);
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
    fn test_wal_append_and_recover() {
        let temp_dir = tempfile::tempdir().unwrap();
        let wal_path = temp_dir.path().join("test.wal");

        {
            let mut wal = WriteAheadLog::open(&wal_path).unwrap();

            // Write a complete transaction
            let begin = WALEntry::new(1, WALEntryType::Begin, vec![]);
            wal.append(&begin).unwrap();

            let op = WALEntry::new(1, WALEntryType::Operation, b"insert doc".to_vec());
            wal.append(&op).unwrap();

            let commit = WALEntry::new(1, WALEntryType::Commit, vec![]);
            wal.append(&commit).unwrap();

            wal.flush().unwrap();
        }

        // Recover
        {
            let mut wal = WriteAheadLog::open(&wal_path).unwrap();
            let recovered = wal.recover().unwrap();

            assert_eq!(recovered.len(), 1);  // One committed transaction
            assert_eq!(recovered[0].len(), 3);  // Begin + Operation + Commit
        }
    }

    #[test]
    fn test_wal_recover_filters_uncommitted() {
        let temp_dir = tempfile::tempdir().unwrap();
        let wal_path = temp_dir.path().join("test.wal");

        {
            let mut wal = WriteAheadLog::open(&wal_path).unwrap();

            // Committed transaction
            wal.append(&WALEntry::new(1, WALEntryType::Begin, vec![])).unwrap();
            wal.append(&WALEntry::new(1, WALEntryType::Operation, b"op1".to_vec())).unwrap();
            wal.append(&WALEntry::new(1, WALEntryType::Commit, vec![])).unwrap();

            // Uncommitted transaction
            wal.append(&WALEntry::new(2, WALEntryType::Begin, vec![])).unwrap();
            wal.append(&WALEntry::new(2, WALEntryType::Operation, b"op2".to_vec())).unwrap();
            // No commit

            wal.flush().unwrap();
        }

        // Recover
        {
            let mut wal = WriteAheadLog::open(&wal_path).unwrap();
            let recovered = wal.recover().unwrap();

            assert_eq!(recovered.len(), 1);  // Only committed transaction
            assert_eq!(recovered[0][0].transaction_id, 1);
        }
    }

    #[test]
    fn test_wal_clear() {
        let temp_dir = tempfile::tempdir().unwrap();
        let wal_path = temp_dir.path().join("test.wal");

        {
            let mut wal = WriteAheadLog::open(&wal_path).unwrap();
            wal.append(&WALEntry::new(1, WALEntryType::Begin, vec![])).unwrap();
            wal.flush().unwrap();

            wal.clear().unwrap();
        }

        // Verify empty
        {
            let mut wal = WriteAheadLog::open(&wal_path).unwrap();
            let recovered = wal.recover().unwrap();
            assert_eq!(recovered.len(), 0);
        }
    }
}
