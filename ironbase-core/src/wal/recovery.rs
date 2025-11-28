// wal/recovery.rs
// Transaction grouper for streaming WAL recovery

use std::collections::HashMap;

use crate::error::Result;
use crate::transaction::TransactionId;

use super::entry::{WALEntry, WALEntryType};

/// A committed transaction with all its entries
#[derive(Debug)]
pub struct CommittedTransaction {
    pub id: TransactionId,
    pub entries: Vec<WALEntry>,
}

impl CommittedTransaction {
    /// Get operation entries (excluding Begin/Commit markers)
    pub fn operations(&self) -> impl Iterator<Item = &WALEntry> {
        self.entries
            .iter()
            .filter(|e| e.entry_type == WALEntryType::Operation)
    }

    /// Get index change entries
    pub fn index_changes(&self) -> impl Iterator<Item = &WALEntry> {
        self.entries
            .iter()
            .filter(|e| e.entry_type == WALEntryType::IndexChange)
    }

    /// Count of operation entries
    pub fn operation_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| e.entry_type == WALEntryType::Operation)
            .count()
    }

    /// Count of index change entries
    pub fn index_change_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| e.entry_type == WALEntryType::IndexChange)
            .count()
    }
}

/// Streaming transaction grouper
///
/// Aggregates WAL entries into committed transactions.
/// Memory usage: O(active transactions) instead of O(all entries)
///
/// Example:
/// ```ignore
/// let iter = WALEntryIterator::new(reader)?;
/// let grouper = TransactionGrouper::new(iter);
///
/// for tx_result in grouper {
///     let tx = tx_result?;
///     // Process committed transaction
/// }
/// ```
pub struct TransactionGrouper<I: Iterator<Item = Result<WALEntry>>> {
    source: I,
    active: HashMap<TransactionId, Vec<WALEntry>>,
}

impl<I: Iterator<Item = Result<WALEntry>>> TransactionGrouper<I> {
    /// Create a new transaction grouper from a WAL entry iterator
    pub fn new(source: I) -> Self {
        Self {
            source,
            active: HashMap::new(),
        }
    }

    /// Get the number of currently active (uncommitted) transactions
    pub fn active_transaction_count(&self) -> usize {
        self.active.len()
    }
}

impl<I: Iterator<Item = Result<WALEntry>>> Iterator for TransactionGrouper<I> {
    type Item = Result<CommittedTransaction>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.source.next()? {
                Ok(entry) => match entry.entry_type {
                    WALEntryType::Begin => {
                        // Start tracking new transaction
                        self.active.entry(entry.transaction_id).or_default();
                    }
                    WALEntryType::Operation | WALEntryType::IndexChange => {
                        // Add to active transaction
                        if let Some(tx) = self.active.get_mut(&entry.transaction_id) {
                            tx.push(entry);
                        }
                        // If transaction not tracked, entry is orphaned - ignore
                    }
                    WALEntryType::Commit => {
                        // Transaction committed - yield it
                        if let Some(entries) = self.active.remove(&entry.transaction_id) {
                            return Some(Ok(CommittedTransaction {
                                id: entry.transaction_id,
                                entries,
                            }));
                        }
                        // If not found, orphaned commit - ignore
                    }
                    WALEntryType::Abort => {
                        // Transaction aborted - discard
                        self.active.remove(&entry.transaction_id);
                    }
                },
                Err(e) => return Some(Err(e)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    use crate::wal::reader::WALEntryIterator;

    fn create_test_iterator(entries: Vec<WALEntry>) -> WALEntryIterator<Cursor<Vec<u8>>> {
        let mut data = Vec::new();
        for entry in &entries {
            data.extend_from_slice(&entry.serialize());
        }
        WALEntryIterator::new(Cursor::new(data)).unwrap()
    }

    #[test]
    fn test_grouper_commits_only() {
        let entries = vec![
            WALEntry::new(1, WALEntryType::Begin, vec![]),
            WALEntry::new(1, WALEntryType::Operation, b"op1".to_vec()),
            WALEntry::new(1, WALEntryType::Commit, vec![]),
        ];

        let iter = create_test_iterator(entries);
        let mut grouper = TransactionGrouper::new(iter);

        let tx = grouper.next().unwrap().unwrap();
        assert_eq!(tx.id, 1);
        assert_eq!(tx.entries.len(), 1); // Only the operation, not Begin/Commit
        assert_eq!(tx.operation_count(), 1);

        assert!(grouper.next().is_none());
    }

    #[test]
    fn test_grouper_discards_aborted() {
        let entries = vec![
            WALEntry::new(1, WALEntryType::Begin, vec![]),
            WALEntry::new(1, WALEntryType::Operation, b"op1".to_vec()),
            WALEntry::new(1, WALEntryType::Abort, vec![]),
        ];

        let iter = create_test_iterator(entries);
        let grouper = TransactionGrouper::new(iter);
        let results: Vec<_> = grouper.collect();

        assert_eq!(results.len(), 0); // No committed transactions
    }

    #[test]
    fn test_grouper_handles_interleaved() {
        // Simulate real-world interleaved transactions
        let entries = vec![
            WALEntry::new(1, WALEntryType::Begin, vec![]),
            WALEntry::new(2, WALEntryType::Begin, vec![]),
            WALEntry::new(1, WALEntryType::Operation, b"tx1-op".to_vec()),
            WALEntry::new(2, WALEntryType::Operation, b"tx2-op".to_vec()),
            WALEntry::new(1, WALEntryType::Commit, vec![]),
            WALEntry::new(2, WALEntryType::Operation, b"tx2-op2".to_vec()),
            WALEntry::new(2, WALEntryType::Commit, vec![]),
        ];

        let iter = create_test_iterator(entries);
        let grouper = TransactionGrouper::new(iter);
        let results: Vec<_> = grouper.map(|r| r.unwrap()).collect();

        assert_eq!(results.len(), 2);

        // Transaction 1 commits first
        assert_eq!(results[0].id, 1);
        assert_eq!(results[0].operation_count(), 1);

        // Transaction 2 commits second
        assert_eq!(results[1].id, 2);
        assert_eq!(results[1].operation_count(), 2);
    }

    #[test]
    fn test_grouper_uncommitted_not_yielded() {
        let entries = vec![
            WALEntry::new(1, WALEntryType::Begin, vec![]),
            WALEntry::new(1, WALEntryType::Operation, b"op1".to_vec()),
            // No commit - simulates crash before commit
        ];

        let iter = create_test_iterator(entries);
        let grouper = TransactionGrouper::new(iter);
        let results: Vec<_> = grouper.collect();

        assert_eq!(results.len(), 0); // Uncommitted transaction not yielded
    }

    #[test]
    fn test_grouper_mixed_committed_aborted_uncommitted() {
        let entries = vec![
            // Transaction 1: committed
            WALEntry::new(1, WALEntryType::Begin, vec![]),
            WALEntry::new(1, WALEntryType::Operation, b"tx1".to_vec()),
            WALEntry::new(1, WALEntryType::Commit, vec![]),
            // Transaction 2: aborted
            WALEntry::new(2, WALEntryType::Begin, vec![]),
            WALEntry::new(2, WALEntryType::Operation, b"tx2".to_vec()),
            WALEntry::new(2, WALEntryType::Abort, vec![]),
            // Transaction 3: uncommitted (crash)
            WALEntry::new(3, WALEntryType::Begin, vec![]),
            WALEntry::new(3, WALEntryType::Operation, b"tx3".to_vec()),
        ];

        let iter = create_test_iterator(entries);
        let grouper = TransactionGrouper::new(iter);
        let results: Vec<_> = grouper.map(|r| r.unwrap()).collect();

        // Only transaction 1 should be recovered
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, 1);
    }

    #[test]
    fn test_grouper_with_index_changes() {
        let entries = vec![
            WALEntry::new(1, WALEntryType::Begin, vec![]),
            WALEntry::new(1, WALEntryType::Operation, b"insert".to_vec()),
            WALEntry::new(1, WALEntryType::IndexChange, b"idx1".to_vec()),
            WALEntry::new(1, WALEntryType::IndexChange, b"idx2".to_vec()),
            WALEntry::new(1, WALEntryType::Commit, vec![]),
        ];

        let iter = create_test_iterator(entries);
        let mut grouper = TransactionGrouper::new(iter);

        let tx = grouper.next().unwrap().unwrap();
        assert_eq!(tx.operation_count(), 1);
        assert_eq!(tx.index_change_count(), 2);
    }
}
