// recovery/mod.rs
// WAL recovery module
//
// This module provides:
// - `RecoveryCoordinator`: Central orchestrator for WAL recovery
// - `OperationReplay`: Replays operations to storage
// - `IndexReplay`: Parses index change entries
// - `RecoveryStats`: Combined statistics from recovery

mod index_replay;
mod operation_replay;

pub use index_replay::{IndexOperation, IndexReplay, IndexReplayStats, RecoveredIndexChange};
pub use operation_replay::{OperationReplay, ReplayStats};

use std::path::Path;

use crate::error::Result;
use crate::storage::{RawStorage, Storage};
use crate::wal::{TransactionGrouper, WALEntryIterator, WriteAheadLog};

/// Combined statistics from WAL recovery
#[derive(Debug, Default, Clone)]
pub struct RecoveryStats {
    /// Number of committed transactions recovered
    pub transactions_recovered: usize,
    /// Number of operations replayed
    pub operations_replayed: usize,
    /// Number of insert operations
    pub inserts: usize,
    /// Number of update operations
    pub updates: usize,
    /// Number of delete operations
    pub deletes: usize,
    /// Number of index changes parsed
    pub index_changes: usize,
}

impl RecoveryStats {
    /// Merge replay stats into recovery stats
    pub fn merge_replay_stats(&mut self, replay: &ReplayStats) {
        self.operations_replayed += replay.operations_replayed;
        self.inserts += replay.inserts;
        self.updates += replay.updates;
        self.deletes += replay.deletes;
    }

    /// Merge index replay stats
    pub fn merge_index_stats(&mut self, index: &IndexReplayStats) {
        self.index_changes += index.changes_parsed;
    }
}

/// Central orchestrator for WAL recovery
///
/// Coordinates streaming WAL reading, transaction grouping,
/// operation replay, and index change extraction.
pub struct RecoveryCoordinator;

impl RecoveryCoordinator {
    /// Perform full WAL recovery
    ///
    /// This method:
    /// 1. Opens the WAL file and creates a streaming iterator
    /// 2. Groups entries by transaction (only committed transactions)
    /// 3. Replays operations to storage
    /// 4. Extracts index changes for later application
    /// 5. Returns stats and index changes
    ///
    /// Memory usage: O(active transactions + single entry) instead of O(entire WAL)
    pub fn recover<S: Storage + RawStorage>(
        wal_path: &Path,
        storage: &mut S,
    ) -> Result<(RecoveryStats, Vec<RecoveredIndexChange>)> {
        use std::fs::File;
        use std::io::BufReader;

        let mut stats = RecoveryStats::default();
        let mut all_index_changes = Vec::new();

        // Check if WAL file exists
        if !wal_path.exists() {
            return Ok((stats, all_index_changes));
        }

        // Open WAL and create streaming iterator
        let file = File::open(wal_path)?;
        let reader = BufReader::new(file);
        let entry_iter = WALEntryIterator::new(reader)?;

        // Create transaction grouper for streaming aggregation
        let grouper = TransactionGrouper::new(entry_iter);

        // Process each committed transaction
        for tx_result in grouper {
            let committed_tx = tx_result?;
            stats.transactions_recovered += 1;

            // Replay operations to storage
            let replay_stats = OperationReplay::replay(storage, &committed_tx.entries)?;
            stats.merge_replay_stats(&replay_stats);

            // Extract index changes
            let index_changes = IndexReplay::parse_entries(&committed_tx.entries)?;
            let index_stats = IndexReplayStats::from_changes(&index_changes);
            stats.merge_index_stats(&index_stats);
            all_index_changes.extend(index_changes);
        }

        Ok((stats, all_index_changes))
    }

    /// Recover and clear WAL
    ///
    /// Performs recovery and then clears the WAL file.
    /// This is the typical recovery flow on database open.
    pub fn recover_and_clear<S: Storage + RawStorage>(
        wal_path: &Path,
        storage: &mut S,
    ) -> Result<(RecoveryStats, Vec<RecoveredIndexChange>)> {
        let result = Self::recover(wal_path, storage)?;

        // Clear WAL after successful recovery
        if wal_path.exists() {
            let mut wal = WriteAheadLog::open(wal_path)?;
            wal.clear()?;
        }

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::MemoryStorage;
    use crate::transaction::Operation;
    use crate::wal::{WALEntry, WALEntryType, WriteAheadLog};
    use serde_json::json;

    #[test]
    fn test_recovery_empty_wal() {
        let temp_dir = tempfile::tempdir().unwrap();
        let wal_path = temp_dir.path().join("test.wal");

        // Create empty WAL
        {
            let _wal = WriteAheadLog::open(&wal_path).unwrap();
        }

        let mut storage = MemoryStorage::new();
        let (stats, index_changes) = RecoveryCoordinator::recover(&wal_path, &mut storage).unwrap();

        assert_eq!(stats.transactions_recovered, 0);
        assert_eq!(stats.operations_replayed, 0);
        assert!(index_changes.is_empty());
    }

    #[test]
    fn test_recovery_single_transaction() {
        let temp_dir = tempfile::tempdir().unwrap();
        let wal_path = temp_dir.path().join("test.wal");

        // Create WAL with one committed transaction
        {
            let mut wal = WriteAheadLog::open(&wal_path).unwrap();

            // Begin
            wal.append(&WALEntry::new(1, WALEntryType::Begin, vec![]))
                .unwrap();

            // Insert operation
            let op = Operation::Insert {
                collection: "users".to_string(),
                doc_id: crate::document::DocumentId::Int(1),
                doc: json!({"_id": 1, "name": "Alice"}),
            };
            let op_data = serde_json::to_vec(&op).unwrap();
            wal.append(&WALEntry::new(1, WALEntryType::Operation, op_data))
                .unwrap();

            // Commit
            wal.append(&WALEntry::new(1, WALEntryType::Commit, vec![]))
                .unwrap();

            wal.flush().unwrap();
        }

        let mut storage = MemoryStorage::new();
        let (stats, index_changes) = RecoveryCoordinator::recover(&wal_path, &mut storage).unwrap();

        assert_eq!(stats.transactions_recovered, 1);
        assert_eq!(stats.operations_replayed, 1);
        assert_eq!(stats.inserts, 1);
        assert!(index_changes.is_empty()); // No index changes in this test
    }

    #[test]
    fn test_recovery_uncommitted_ignored() {
        let temp_dir = tempfile::tempdir().unwrap();
        let wal_path = temp_dir.path().join("test.wal");

        // Create WAL with uncommitted transaction
        {
            let mut wal = WriteAheadLog::open(&wal_path).unwrap();

            // Begin but no commit
            wal.append(&WALEntry::new(1, WALEntryType::Begin, vec![]))
                .unwrap();

            let op = Operation::Insert {
                collection: "users".to_string(),
                doc_id: crate::document::DocumentId::Int(1),
                doc: json!({"_id": 1, "name": "Alice"}),
            };
            let op_data = serde_json::to_vec(&op).unwrap();
            wal.append(&WALEntry::new(1, WALEntryType::Operation, op_data))
                .unwrap();

            // No commit!
            wal.flush().unwrap();
        }

        let mut storage = MemoryStorage::new();
        let (stats, _) = RecoveryCoordinator::recover(&wal_path, &mut storage).unwrap();

        // Uncommitted transaction should be ignored
        assert_eq!(stats.transactions_recovered, 0);
        assert_eq!(stats.operations_replayed, 0);
    }

    #[test]
    fn test_recovery_stats_merge() {
        let mut stats = RecoveryStats::default();

        let replay = ReplayStats {
            operations_replayed: 5,
            inserts: 3,
            updates: 1,
            deletes: 1,
        };

        stats.merge_replay_stats(&replay);

        assert_eq!(stats.operations_replayed, 5);
        assert_eq!(stats.inserts, 3);
        assert_eq!(stats.updates, 1);
        assert_eq!(stats.deletes, 1);
    }

    #[test]
    fn test_recover_and_clear() {
        let temp_dir = tempfile::tempdir().unwrap();
        let wal_path = temp_dir.path().join("test.wal");

        // Create WAL with committed transaction
        {
            let mut wal = WriteAheadLog::open(&wal_path).unwrap();
            wal.append(&WALEntry::new(1, WALEntryType::Begin, vec![]))
                .unwrap();

            let op = Operation::Insert {
                collection: "test".to_string(),
                doc_id: crate::document::DocumentId::Int(1),
                doc: json!({"_id": 1}),
            };
            let op_data = serde_json::to_vec(&op).unwrap();
            wal.append(&WALEntry::new(1, WALEntryType::Operation, op_data))
                .unwrap();

            wal.append(&WALEntry::new(1, WALEntryType::Commit, vec![]))
                .unwrap();
            wal.flush().unwrap();
        }

        let mut storage = MemoryStorage::new();
        let (stats, _) = RecoveryCoordinator::recover_and_clear(&wal_path, &mut storage).unwrap();

        assert_eq!(stats.transactions_recovered, 1);

        // WAL should now be empty
        let mut wal = WriteAheadLog::open(&wal_path).unwrap();
        let recovered = wal.recover().unwrap();
        assert!(recovered.is_empty());
    }
}
