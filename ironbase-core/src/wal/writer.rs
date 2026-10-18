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
    /// Repair a failed operation left pending: until it succeeds, no entry
    /// may be appended (it would follow a partial record, or be overwritten
    /// by the retained entries at the next open)
    pending_repair: Option<Repair>,
    /// Test hook: the next append writes only this many bytes, then fails
    #[cfg(test)]
    fail_next_append_after: Option<usize>,
}

/// Work a failed WAL operation still owes the file.
#[derive(Debug, Clone, Copy)]
enum Repair {
    /// Cut the WAL back to this length (a failed append's partial bytes)
    Truncate(u64),
    /// Finish replacing the WAL with the published retained entries
    FinishRestore,
}

/// Retained entries being written by `retain_after`; not yet trusted
const RETAIN_TEMP_EXT: &str = "wal.retain.tmp";
/// Retained entries, complete and fsynced: they replace the WAL
const RETAIN_EXT: &str = "wal.retain";
/// Written by releases before v1.0.602 without fsync or rename: it may be
/// partial, so it only replaces an empty WAL
const LEGACY_KEEP_EXT: &str = "wal.keep";

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

        // A `retain_after` that did not finish (WAL/recovery audit
        // 2026-10-14 W7): an unpublished file is discarded (the WAL was not
        // touched yet); failing to remove it does not block the open
        let retain_temp_path = path.with_extension(RETAIN_TEMP_EXT);
        if retain_temp_path.exists() {
            if let Err(e) = std::fs::remove_file(&retain_temp_path) {
                tracing::warn!(error = %e, "Failed to remove an unpublished WAL retain file");
            }
        }

        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(&path)?;

        let mut wal = WriteAheadLog {
            file,
            path,
            pending_repair: None,
            #[cfg(test)]
            fail_next_append_after: None,
        };

        // A published file holds every entry the WAL must keep: it replaces
        // the WAL
        if wal.path.with_extension(RETAIN_EXT).exists() {
            tracing::warn!("Restoring the WAL from an interrupted checkpoint");
            wal.restore_retained()?;
        }

        // An older release's keep file may be partial: it is only the copy to
        // use when that release had already emptied the WAL
        let legacy_path = wal.path.with_extension(LEGACY_KEEP_EXT);
        if legacy_path.exists() {
            if wal.file_size()? == 0 {
                tracing::warn!("Restoring the WAL from an older release's interrupted checkpoint");
                crate::fs_utils::atomic_rename_and_sync(
                    &legacy_path,
                    wal.path.with_extension(RETAIN_EXT),
                )?;
                wal.restore_retained()?;
            } else {
                std::fs::remove_file(&legacy_path)?;
                sync_parent_dir(&legacy_path)?;
            }
        }
        Ok(wal)
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
    ///
    /// A failed write (e.g. disk full) is cut off again: left in the file,
    /// its partial bytes would sit in front of every later entry, and
    /// recovery reports an invalid record followed by valid ones as
    /// corruption (WAL/recovery audit 2026-10-14 W6).
    /// If the cut fails too, appends are refused until it succeeds.
    pub fn append(&mut self, entry: &WALEntry) -> Result<u64> {
        self.repair()?;
        let serialized = entry.serialize();
        let offset = self.file.seek(SeekFrom::End(0))?;
        if let Err(e) = self.write_entry_bytes(&serialized) {
            if let Err(truncate_err) = self.truncate_to(offset) {
                tracing::error!(
                    error = %truncate_err,
                    offset,
                    "Failed to cut a partial WAL entry; appends wait until it is cut"
                );
                self.pending_repair = Some(Repair::Truncate(offset));
            }
            return Err(e.into());
        }
        Ok(offset)
    }

    /// Do the work a failed operation left pending, if any.
    fn repair(&mut self) -> Result<()> {
        match self.pending_repair {
            None => return Ok(()),
            Some(Repair::Truncate(len)) => self.truncate_to(len)?,
            Some(Repair::FinishRestore) => self.restore_retained()?,
        }
        self.pending_repair = None;
        Ok(())
    }

    #[cfg(not(test))]
    fn write_entry_bytes(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.file.write_all(bytes)
    }

    #[cfg(test)]
    fn write_entry_bytes(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        match self.fail_next_append_after.take() {
            Some(n) => {
                self.file.write_all(&bytes[..n.min(bytes.len())])?;
                Err(std::io::Error::other("injected write failure"))
            }
            None => self.file.write_all(bytes),
        }
    }

    /// Cut the WAL back to `len` bytes. Through a second handle: Windows
    /// refuses `set_len` on the append-mode one (see [`clear`](Self::clear)).
    fn truncate_to(&mut self, len: u64) -> std::io::Result<()> {
        let file = OpenOptions::new().write(true).open(&self.path)?;
        file.set_len(len)?;
        file.sync_all()
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
    /// OOM PROTECTION: aborts when the entries held exceed
    /// [`recovery_memory_budget`](Self::recovery_memory_budget), a share of
    /// the available RAM. An entry-count cap (1 000 000) refused WALs that
    /// fit in memory many times over, and the database could not be opened
    /// without deleting committed transactions (WAL/recovery audit
    /// 2026-10-14 W5).
    pub fn recover(&mut self) -> Result<Vec<Vec<WALEntry>>> {
        use std::collections::HashMap;
        use std::io::BufReader;

        // Reopen file for reading
        let file = File::open(&self.path)?;
        let reader = BufReader::new(file);
        let iter = WALEntryIterator::new(reader)?;

        // Group entries by transaction ID
        let mut txs: HashMap<TransactionId, Vec<WALEntry>> = HashMap::new();
        let mut committed = Vec::new();
        let budget = Self::recovery_memory_budget();
        // Bytes of the entries still held (`txs` + `committed`)
        let mut held: u64 = 0;
        let group_bytes = |group: &[WALEntry]| group.iter().map(Self::held_bytes).sum::<u64>();

        for entry_result in iter {
            let entry = entry_result?;
            held += Self::held_bytes(&entry);

            // OOM protection: abort if the entries no longer fit in memory
            if held > budget {
                return Err(crate::error::IronBaseError::OutOfMemory(format!(
                    "WAL recovery aborted: the WAL needs more than {} MB of memory \
                     to replay (budget: a quarter of the available RAM). Open the \
                     database on a machine with more free memory",
                    budget / (1024 * 1024)
                )));
            }

            // Emit a transaction when its COMMIT is read, so the result is in
            // log (commit) order. Replay applies full-image ops, so the order
            // decides which version of a document wins (audit 2026-10-06 #7:
            // iterating the HashMap returned transactions in random order).
            let tx_id = entry.transaction_id;
            let entry_type = entry.entry_type;
            if entry_type == WALEntryType::Begin {
                // A BEGIN starts a new transaction: drop a torn group left
                // under the same id, so it cannot merge into this one
                // (audit 2026-10-06 #34).
                if let Some(torn) = txs.remove(&tx_id) {
                    held -= group_bytes(&torn);
                }
            }
            txs.entry(tx_id).or_default().push(entry);
            match entry_type {
                WALEntryType::Commit => {
                    if let Some(tx_entries) = txs.remove(&tx_id) {
                        committed.push(tx_entries);
                    }
                }
                WALEntryType::Abort => {
                    // An ABORT after COMMIT is written when the persist phase
                    // failed (`abort_committed_transaction`): the caller got
                    // an error, so the already emitted transaction must not
                    // be replayed.
                    // The ABORT itself was just pushed: a group holding only
                    // it means no open transaction had this id.
                    let group = txs.remove(&tx_id).unwrap_or_default();
                    held -= group_bytes(&group);
                    if group.len() <= 1 {
                        if let Some(pos) = committed.iter().rposition(|tx: &Vec<WALEntry>| {
                            tx.first().map(|e| e.transaction_id) == Some(tx_id)
                        }) {
                            held -= group_bytes(&committed.remove(pos));
                        }
                    }
                }
                _ => {}
            }
        }

        // Whatever is left in `txs` is uncommitted - discard

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

    /// Clear the WAL but keep every entry of a transaction with id
    /// `> keep_after`; returns the number of kept entries.
    ///
    /// Used by checkpoints: the index files were flushed with watermark
    /// `keep_after`, so later transactions may be missing from them and their
    /// WAL entries are still needed for index replay after a crash
    /// (`tx_id > last_flushed_tx_id`). Clearing the whole WAL lost them.
    ///
    /// Streams the kept entries through a temp file (O(1) memory) that is
    /// fsynced and renamed to `.wal.retain` before the WAL is touched, then
    /// truncates the WAL the same way as [`clear`](Self::clear) (no rename
    /// over the open file, which Windows refuses) and appends them back. A
    /// crash at any point after the rename leaves the retained file, and
    /// [`open`](Self::open) finishes the job; before, a crash between the
    /// truncate and the copy lost the kept entries (WAL/recovery audit
    /// 2026-10-14 W7).
    pub fn retain_after(&mut self, keep_after: TransactionId) -> Result<usize> {
        use std::io::{BufReader, BufWriter};

        self.repair()?;
        if self.file_size()? == 0 {
            return Ok(0);
        }
        let keep_temp_path = self.path.with_extension(RETAIN_TEMP_EXT);
        let mut kept = 0usize;
        {
            let iter = WALEntryIterator::new(BufReader::new(File::open(&self.path)?))?;
            let mut temp = BufWriter::new(
                OpenOptions::new()
                    .create(true)
                    .write(true)
                    .truncate(true)
                    .open(&keep_temp_path)?,
            );
            for entry in iter {
                let entry = entry?;
                if entry.transaction_id > keep_after {
                    temp.write_all(&entry.serialize())?;
                    kept += 1;
                }
            }
            temp.into_inner().map_err(|e| e.into_error())?.sync_all()?;
        }

        if kept == 0 {
            std::fs::remove_file(&keep_temp_path)?;
            self.clear()?;
            return Ok(0);
        }
        let keep_path = self.path.with_extension(RETAIN_EXT);
        crate::fs_utils::atomic_rename_and_sync(&keep_temp_path, &keep_path)?;
        // The rename helper only logs a failed directory fsync: the WAL must
        // not be truncated before the published file is durable
        sync_parent_dir(&keep_path)?;
        self.restore_retained()?;
        Ok(kept)
    }

    /// Replace the WAL with the published retained entries, then delete them.
    ///
    /// Idempotent: run again after a crash or an error it gives the same WAL.
    /// Until it completes no entry may be appended: the next open would
    /// restore the retained entries over it. The file's removal is made
    /// durable before returning, so a later open cannot restore it over
    /// entries appended since.
    fn restore_retained(&mut self) -> Result<()> {
        let keep_path = self.path.with_extension(RETAIN_EXT);
        self.pending_repair = Some(Repair::FinishRestore);
        if keep_path.exists() {
            self.clear()?;
            let mut keep = std::io::BufReader::new(File::open(&keep_path)?);
            std::io::copy(&mut keep, &mut self.file)?;
            self.file.sync_all()?;
            drop(keep);
            std::fs::remove_file(&keep_path)?;
        }
        sync_parent_dir(&keep_path)?;
        self.pending_repair = None;
        Ok(())
    }

    /// Maximum WAL entries for `checkpoint` (OOM protection)
    ///
    /// If WAL has more entries than this, something is very wrong
    /// (WAL clear should run on every flush/checkpoint).
    /// With checkpoint every 120s and ~5K ops/sec Safe mode,
    /// worst case is ~600K entries between checkpoints.
    const MAX_WAL_ENTRIES: usize = 1_000_000;

    /// Recovery budget when the available RAM cannot be detected (Windows)
    const RECOVERY_FALLBACK_BUDGET: u64 = 1024 * 1024 * 1024;

    /// Memory one entry held by `recover` takes.
    fn held_bytes(entry: &WALEntry) -> u64 {
        (std::mem::size_of::<WALEntry>() + entry.data.len()) as u64
    }

    /// Bytes of WAL entries `recover` may hold: a quarter of the available
    /// RAM (the replay also parses every operation and keeps it for index
    /// replay, roughly doubling it), 1 GB when the RAM is unknown.
    fn recovery_memory_budget() -> u64 {
        crate::aggregation::memory_info::get_available_memory_bytes()
            .map(|bytes| bytes / 4)
            .unwrap_or(Self::RECOVERY_FALLBACK_BUDGET)
    }

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

        // Atomic rename + parent directory fsync for durability on POSIX
        crate::fs_utils::atomic_rename_and_sync(&temp_path, &self.path)?;

        // Reopen file
        self.file = OpenOptions::new()
            .read(true)
            .append(true)
            .open(&self.path)?;

        Ok(())
    }
}

/// fsync the directory holding `path`, so a rename or removal in it is
/// durable. Unlike [`crate::fs_utils::atomic_rename_and_sync`] the error is
/// returned: the WAL steps that depend on it must not run without it. Skipped
/// on non-Unix platforms (NTFS journals renames).
fn sync_parent_dir(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        File::open(parent)?.sync_all()?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
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

    /// The WAL after a checkpoint kept tx 2 (watermark 1), crashed at
    /// `stage` of `retain_after` (WAL/recovery audit 2026-10-14 W7).
    fn wal_after_interrupted_retain(stage: &str) -> Vec<TransactionId> {
        let temp_dir = tempfile::tempdir().unwrap();
        let wal_path = temp_dir.path().join("w7.wal");
        let entries = |tx| {
            vec![
                WALEntry::new(tx, WALEntryType::Begin, vec![]),
                WALEntry::new(tx, WALEntryType::Operation, b"op".to_vec()),
                WALEntry::new(tx, WALEntryType::Commit, vec![]),
            ]
        };
        {
            let mut wal = WriteAheadLog::open(&wal_path).unwrap();
            for e in entries(1).iter().chain(entries(2).iter()) {
                wal.append(e).unwrap();
            }
            wal.flush().unwrap();
        }
        let keep: Vec<u8> = entries(2).iter().flat_map(|e| e.serialize()).collect();
        match stage {
            // Retained file published, WAL truncated, entries not copied back yet
            "truncated" => {
                std::fs::write(wal_path.with_extension(RETAIN_EXT), &keep).unwrap();
                std::fs::write(&wal_path, b"").unwrap();
            }
            // Retained file published, WAL not touched yet
            "published" => {
                std::fs::write(wal_path.with_extension(RETAIN_EXT), &keep).unwrap();
            }
            // Retained file still being written: the WAL is intact
            "writing" => {
                std::fs::write(wal_path.with_extension(RETAIN_TEMP_EXT), &keep[..10]).unwrap();
            }
            // An older release emptied the WAL: its keep file is the only copy
            "legacy_truncated" => {
                std::fs::write(wal_path.with_extension(LEGACY_KEEP_EXT), &keep).unwrap();
                std::fs::write(&wal_path, b"").unwrap();
            }
            // An older release was still writing its keep file: the WAL wins
            "legacy_writing" => {
                std::fs::write(wal_path.with_extension(LEGACY_KEEP_EXT), &keep[..10]).unwrap();
            }
            _ => unreachable!(),
        }

        let mut wal = WriteAheadLog::open(&wal_path).unwrap();
        for ext in [RETAIN_EXT, RETAIN_TEMP_EXT, LEGACY_KEEP_EXT] {
            assert!(!wal_path.with_extension(ext).exists(), "{ext} left behind");
        }
        wal.recover()
            .unwrap()
            .iter()
            .map(|tx| tx[0].transaction_id)
            .collect()
    }

    #[test]
    fn interrupted_retain_after_keeps_the_kept_entries() {
        assert_eq!(wal_after_interrupted_retain("truncated"), vec![2]);
        assert_eq!(wal_after_interrupted_retain("published"), vec![2]);
        assert_eq!(wal_after_interrupted_retain("writing"), vec![1, 2]);
        assert_eq!(wal_after_interrupted_retain("legacy_truncated"), vec![2]);
        assert_eq!(wal_after_interrupted_retain("legacy_writing"), vec![1, 2]);
    }

    #[test]
    fn pending_repair_runs_before_the_next_append() {
        let temp_dir = tempfile::tempdir().unwrap();
        let wal_path = temp_dir.path().join("repair.wal");
        let begin = |tx| WALEntry::new(tx, WALEntryType::Begin, vec![]);
        let commit = |tx| WALEntry::new(tx, WALEntryType::Commit, vec![]);

        // A partial append whose cut failed: cut before appending again
        let mut wal = WriteAheadLog::open(&wal_path).unwrap();
        wal.append(&begin(1)).unwrap();
        let size = wal.file_size().unwrap();
        wal.file.write_all(b"partial").unwrap();
        wal.pending_repair = Some(Repair::Truncate(size));
        wal.append(&commit(1)).unwrap();
        assert!(wal.pending_repair.is_none());
        assert_eq!(wal.recover().unwrap().len(), 1);

        // A restore that failed half-way: finish it before appending again
        let retained: Vec<u8> = [begin(2), commit(2)]
            .iter()
            .flat_map(|e| e.serialize())
            .collect();
        std::fs::write(wal_path.with_extension(RETAIN_EXT), retained).unwrap();
        wal.pending_repair = Some(Repair::FinishRestore);
        wal.append(&begin(3)).unwrap();
        wal.append(&commit(3)).unwrap();
        assert!(!wal_path.with_extension(RETAIN_EXT).exists());
        let ids: Vec<_> = wal
            .recover()
            .unwrap()
            .iter()
            .map(|t| t[0].transaction_id)
            .collect();
        assert_eq!(ids, vec![2, 3]);
    }

    #[test]
    fn retain_after_leaves_no_keep_file() {
        let temp_dir = tempfile::tempdir().unwrap();
        let wal_path = temp_dir.path().join("w7b.wal");
        let mut wal = WriteAheadLog::open(&wal_path).unwrap();
        for tx in [1, 2] {
            wal.append(&WALEntry::new(tx, WALEntryType::Begin, vec![]))
                .unwrap();
            wal.append(&WALEntry::new(tx, WALEntryType::Commit, vec![]))
                .unwrap();
        }
        assert_eq!(wal.retain_after(1).unwrap(), 2);
        assert!(!wal_path.with_extension(RETAIN_EXT).exists());
        assert!(!wal_path.with_extension(RETAIN_TEMP_EXT).exists());
        let ids: Vec<_> = wal
            .recover()
            .unwrap()
            .iter()
            .map(|t| t[0].transaction_id)
            .collect();
        assert_eq!(ids, vec![2]);

        // Nothing to keep: the WAL is just cleared
        assert_eq!(wal.retain_after(5).unwrap(), 0);
        assert_eq!(wal.file_size().unwrap(), 0);
    }

    #[test]
    fn failed_append_is_cut_off() {
        // A partial write left in the WAL would sit in front of every later
        // entry, and recovery reports that as corruption (audit W6)
        let temp_dir = tempfile::tempdir().unwrap();
        let wal_path = temp_dir.path().join("w6.wal");
        let mut wal = WriteAheadLog::open(&wal_path).unwrap();
        wal.append(&WALEntry::new(1, WALEntryType::Begin, vec![]))
            .unwrap();
        let size = wal.file_size().unwrap();

        wal.fail_next_append_after = Some(7);
        assert!(wal
            .append(&WALEntry::new(1, WALEntryType::Commit, vec![]))
            .is_err());
        assert_eq!(wal.file_size().unwrap(), size);

        wal.append(&WALEntry::new(1, WALEntryType::Commit, vec![]))
            .unwrap();
        let recovered = wal.recover().unwrap();
        assert_eq!(recovered.len(), 1);
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

    /// Audit 2026-10-06 #34 / #7: an ABORT written after COMMIT (persist
    /// failure) retracts the transaction.
    #[test]
    fn commit_then_abort_is_not_recovered() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mut wal = WriteAheadLog::open(temp_dir.path().join("a.wal")).unwrap();
        for (tx, kind) in [
            (1, WALEntryType::Begin),
            (1, WALEntryType::Commit),
            (2, WALEntryType::Begin),
            (2, WALEntryType::Commit),
            (1, WALEntryType::Abort),
        ] {
            wal.append(&WALEntry::new(tx, kind, vec![])).unwrap();
        }
        wal.flush().unwrap();
        let recovered = wal.recover().unwrap();
        let ids: Vec<_> = recovered.iter().map(|t| t[0].transaction_id).collect();
        assert_eq!(ids, vec![2]);
    }

    /// Audit 2026-10-06 #34: a torn group (BEGIN + OP, no COMMIT) must not
    /// merge into a later transaction that reuses its id.
    #[test]
    fn torn_group_does_not_merge_into_reused_id() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mut wal = WriteAheadLog::open(temp_dir.path().join("t.wal")).unwrap();
        wal.append(&WALEntry::new(7, WALEntryType::Begin, vec![]))
            .unwrap();
        wal.append(&WALEntry::new(7, WALEntryType::Operation, b"torn".to_vec()))
            .unwrap();
        wal.append(&WALEntry::new(7, WALEntryType::Begin, vec![]))
            .unwrap();
        wal.append(&WALEntry::new(7, WALEntryType::Operation, b"new".to_vec()))
            .unwrap();
        wal.append(&WALEntry::new(7, WALEntryType::Commit, vec![]))
            .unwrap();
        wal.flush().unwrap();
        let recovered = wal.recover().unwrap();
        assert_eq!(recovered.len(), 1);
        let ops: Vec<_> = recovered[0]
            .iter()
            .filter(|e| e.entry_type == WALEntryType::Operation)
            .map(|e| e.data.clone())
            .collect();
        assert_eq!(ops, vec![b"new".to_vec()]);
    }

    /// `retain_after` keeps exactly the entries of later transactions.
    #[test]
    fn retain_after_keeps_later_transactions() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mut wal = WriteAheadLog::open(temp_dir.path().join("r.wal")).unwrap();
        for tx in [0u64, 3, 4, 5, 6] {
            wal.append(&WALEntry::new(tx, WALEntryType::Begin, vec![]))
                .unwrap();
            wal.append(&WALEntry::new(tx, WALEntryType::Commit, vec![]))
                .unwrap();
        }
        assert_eq!(wal.retain_after(4).unwrap(), 4);
        let ids: Vec<_> = wal
            .recover()
            .unwrap()
            .iter()
            .map(|t| t[0].transaction_id)
            .collect();
        assert_eq!(ids, vec![5, 6]);
        // appends still work after the rewrite
        wal.append(&WALEntry::new(7, WALEntryType::Begin, vec![]))
            .unwrap();
        wal.append(&WALEntry::new(7, WALEntryType::Commit, vec![]))
            .unwrap();
        assert_eq!(wal.recover().unwrap().len(), 3);
        assert_eq!(wal.retain_after(100).unwrap(), 0);
        assert_eq!(wal.file_size().unwrap(), 0);
    }
}
