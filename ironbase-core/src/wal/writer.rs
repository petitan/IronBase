//! # WAL Writer - Write-Ahead Log Fájl Kezelő
//!
//! Felelős a WAL fájl írásáért és életciklus kezeléséért.
//!
//! ## Műveletek
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────┐
//! │  WriteAheadLog                                          │
//! ├─────────────────────────────────────────────────────────┤
//! │  open(path)    → Megnyitás/létrehozás                   │
//! │  append(entry) → Bejegyzés hozzáadása (seek end + write)│
//! │  flush()       → fsync() - adatok lemezre írása         │
//! │  recover()     → Committed TX-ek visszaolvasása         │
//! │  clear()       → WAL törlése (recovery után)            │
//! │  checkpoint()  → Committed TX-ek eltávolítása           │
//! └─────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Windows Kompatibilitás
//!
//! `clear()` workaround: Windows-on append mode-ban `set_len(0)` "Access Denied".
//! Megoldás: close → truncate mode open → reopen append mode.

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
    ///
    /// Also cleans up any orphaned .wal.tmp files from interrupted checkpoint operations.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();

        // Clean up orphaned temp file from interrupted checkpoint (crash recovery)
        let temp_path = path.with_extension("wal.tmp");
        if temp_path.exists() {
            tracing::warn!(
                temp_path = %temp_path.display(),
                "Removing orphaned WAL temp file from interrupted checkpoint"
            );
            if let Err(e) = std::fs::remove_file(&temp_path) {
                tracing::error!(error = %e, "Failed to remove orphaned WAL temp file");
            }
        }

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

    /// Get the current WAL file size in bytes
    pub fn file_size(&self) -> Result<u64> {
        Ok(self.file.metadata()?.len())
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
    ///
    /// OOM PROTECTION: Aborts if WAL contains more than MAX_WAL_ENTRIES entries.
    /// This should never happen in normal operation (WAL cleared every flush/checkpoint).
    pub fn recover(&mut self) -> Result<Vec<Vec<WALEntry>>> {
        use std::collections::HashMap;
        use std::io::BufReader;

        // Reopen file for reading
        let file = File::open(&self.path)?;
        let reader = BufReader::new(file);
        let iter = WALEntryIterator::new(reader)?;

        // Group entries by transaction ID
        let mut txs: HashMap<TransactionId, Vec<WALEntry>> = HashMap::new();
        let mut entry_count: usize = 0;

        for entry_result in iter {
            let entry = entry_result?;
            entry_count += 1;

            // OOM protection: abort if WAL is abnormally large
            if entry_count > Self::MAX_WAL_ENTRIES {
                return Err(crate::error::IronBaseError::OutOfMemory(format!(
                    "WAL recovery aborted: too many entries (>{}) - \
                     WAL file may be corrupted or was not cleared. \
                     Consider deleting the .wal file (backup .mlite first)",
                    Self::MAX_WAL_ENTRIES
                )));
            }

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
    ///
    /// On Windows, `set_len(0)` fails with "Access Denied" when the file
    /// is opened in append mode. We work around this by closing the file
    /// and reopening it with truncate mode, then reopening in append mode.
    pub fn clear(&mut self) -> Result<()> {
        // Close current handle by reopening with truncate (clears the file)
        self.file = OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&self.path)?;
        self.file.sync_all()?;

        // Reopen in append mode for future writes
        self.file = OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(&self.path)?;

        Ok(())
    }

    /// Maximum WAL entries for recovery/checkpoint (OOM protection)
    ///
    /// If WAL has more entries than this, something is very wrong
    /// (WAL clear should run on every flush/checkpoint).
    /// With checkpoint every 120s and ~5K ops/sec Safe mode,
    /// worst case is ~600K entries between checkpoints.
    const MAX_WAL_ENTRIES: usize = 1_000_000;

    /// Checkpoint: remove committed transactions from WAL
    ///
    /// Rewrites the WAL file keeping only uncommitted transactions.
    /// Streams entries directly to temp file to minimize memory usage.
    pub fn checkpoint(&mut self, committed_tx_ids: &[TransactionId]) -> Result<()> {
        use std::io::BufReader;

        // Stream entries directly to temp file (no full collect into memory)
        let file = File::open(&self.path)?;
        let reader = BufReader::new(file);
        let iter = WALEntryIterator::new(reader)?;

        let temp_path = self.path.with_extension("wal.tmp");
        let mut temp_file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&temp_path)?;

        let mut entry_count: usize = 0;
        for entry_result in iter {
            let entry = entry_result?;
            entry_count += 1;

            // OOM protection: if WAL is absurdly large, abort checkpoint
            if entry_count > Self::MAX_WAL_ENTRIES {
                // Clean up temp file
                drop(temp_file);
                let _ = std::fs::remove_file(&temp_path);
                return Err(crate::error::IronBaseError::Io(std::io::Error::other(
                    format!(
                        "WAL checkpoint aborted: too many entries (>{}) - consider WAL clear instead",
                        Self::MAX_WAL_ENTRIES
                    ),
                )));
            }

            // Write only uncommitted entries to temp file
            if !committed_tx_ids.contains(&entry.transaction_id) {
                temp_file.write_all(&entry.serialize())?;
            }
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

impl Drop for WriteAheadLog {
    fn drop(&mut self) {
        if let Err(e) = self.file.sync_all() {
            tracing::error!(error = %e, "WriteAheadLog::drop sync_all failed");
        }
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
