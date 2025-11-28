// wal/writer.rs
// Write-Ahead Log file manager

use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::error::Result;
use crate::transaction::TransactionId;

use super::entry::{WALEntry, WALEntryType};
use super::reader::WALEntryIterator;

/// Write-Ahead Log file manager
///
/// Handles appending entries and managing the WAL file lifecycle.
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
            .append(true)
            .open(&path)?;

        Ok(WriteAheadLog { file, path })
    }

    /// Get the path to this WAL file
    pub fn path(&self) -> &Path {
        &self.path
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

    /// Recover transactions from WAL using streaming iterator
    ///
    /// Returns grouped transactions (only committed ones).
    /// This method uses the new streaming approach but returns the same
    /// format as the old method for backwards compatibility.
    pub fn recover(&mut self) -> Result<Vec<Vec<WALEntry>>> {
        use std::collections::HashMap;
        use std::io::BufReader;

        // Reopen file for reading
        let file = File::open(&self.path)?;
        let reader = BufReader::new(file);
        let iter = WALEntryIterator::new(reader)?;

        // Group entries by transaction ID
        let mut txs: HashMap<TransactionId, Vec<WALEntry>> = HashMap::new();

        for entry_result in iter {
            let entry = entry_result?;
            txs.entry(entry.transaction_id).or_default().push(entry);
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

    /// Clear WAL file (after successful recovery)
    pub fn clear(&mut self) -> Result<()> {
        self.file.set_len(0)?;
        self.file.seek(SeekFrom::Start(0))?;
        self.file.sync_all()?; // Ensure truncation is persisted to disk
        Ok(())
    }

    /// Checkpoint: remove committed transactions from WAL
    ///
    /// Rewrites the WAL file keeping only uncommitted transactions.
    pub fn checkpoint(&mut self, committed_tx_ids: &[TransactionId]) -> Result<()> {
        use std::io::BufReader;

        // Read all entries using streaming iterator
        let file = File::open(&self.path)?;
        let reader = BufReader::new(file);
        let iter = WALEntryIterator::new(reader)?;

        let mut all_entries = Vec::new();
        for entry_result in iter {
            all_entries.push(entry_result?);
        }

        // Keep only uncommitted transactions
        let active_entries: Vec<_> = all_entries
            .into_iter()
            .filter(|e| !committed_tx_ids.contains(&e.transaction_id))
            .collect();

        // Rewrite WAL file atomically
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
            .append(true)
            .open(&self.path)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

            assert_eq!(recovered.len(), 1); // One committed transaction
            assert_eq!(recovered[0].len(), 3); // Begin + Operation + Commit
        }
    }

    #[test]
    fn test_wal_recover_filters_uncommitted() {
        let temp_dir = tempfile::tempdir().unwrap();
        let wal_path = temp_dir.path().join("test.wal");

        {
            let mut wal = WriteAheadLog::open(&wal_path).unwrap();

            // Committed transaction
            wal.append(&WALEntry::new(1, WALEntryType::Begin, vec![]))
                .unwrap();
            wal.append(&WALEntry::new(1, WALEntryType::Operation, b"op1".to_vec()))
                .unwrap();
            wal.append(&WALEntry::new(1, WALEntryType::Commit, vec![]))
                .unwrap();

            // Uncommitted transaction
            wal.append(&WALEntry::new(2, WALEntryType::Begin, vec![]))
                .unwrap();
            wal.append(&WALEntry::new(2, WALEntryType::Operation, b"op2".to_vec()))
                .unwrap();
            // No commit

            wal.flush().unwrap();
        }

        // Recover
        {
            let mut wal = WriteAheadLog::open(&wal_path).unwrap();
            let recovered = wal.recover().unwrap();

            assert_eq!(recovered.len(), 1); // Only committed transaction
            assert_eq!(recovered[0][0].transaction_id, 1);
        }
    }

    #[test]
    fn test_wal_clear() {
        let temp_dir = tempfile::tempdir().unwrap();
        let wal_path = temp_dir.path().join("test.wal");

        {
            let mut wal = WriteAheadLog::open(&wal_path).unwrap();
            wal.append(&WALEntry::new(1, WALEntryType::Begin, vec![]))
                .unwrap();
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

    #[test]
    fn test_wal_checkpoint() {
        let temp_dir = tempfile::tempdir().unwrap();
        let wal_path = temp_dir.path().join("test.wal");

        {
            let mut wal = WriteAheadLog::open(&wal_path).unwrap();

            // Transaction 1 (will be checkpointed)
            wal.append(&WALEntry::new(1, WALEntryType::Begin, vec![]))
                .unwrap();
            wal.append(&WALEntry::new(1, WALEntryType::Commit, vec![]))
                .unwrap();

            // Transaction 2 (still active)
            wal.append(&WALEntry::new(2, WALEntryType::Begin, vec![]))
                .unwrap();
            wal.append(&WALEntry::new(
                2,
                WALEntryType::Operation,
                b"active".to_vec(),
            ))
            .unwrap();

            wal.flush().unwrap();

            // Checkpoint transaction 1
            wal.checkpoint(&[1]).unwrap();
        }

        // Verify only transaction 2 remains
        {
            let mut wal = WriteAheadLog::open(&wal_path).unwrap();
            let recovered = wal.recover().unwrap();
            // Transaction 2 has no commit, so it should not be recovered
            assert_eq!(recovered.len(), 0);
        }
    }
}
