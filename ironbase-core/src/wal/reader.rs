// wal/reader.rs
// Streaming WAL reader with iterator pattern

use std::io::{Read, Seek, SeekFrom};

use crate::error::{MongoLiteError, Result};

use super::entry::{WALEntry, WALEntryType, MAX_WAL_ENTRY_SIZE, WAL_HEADER_SIZE};

/// Streaming iterator for reading WAL entries
///
/// This iterator reads entries one at a time from the underlying reader,
/// avoiding buffering the entire WAL file in memory.
///
/// Memory usage: O(single entry) instead of O(entire WAL)
pub struct WALEntryIterator<R: Read + Seek> {
    reader: R,
    eof_reached: bool,
}

impl<R: Read + Seek> WALEntryIterator<R> {
    /// Create a new streaming WAL iterator
    pub fn new(mut reader: R) -> Result<Self> {
        // Seek to start
        reader.seek(SeekFrom::Start(0))?;
        Ok(Self {
            reader,
            eof_reached: false,
        })
    }

    /// Read the next entry from the WAL
    fn read_next(&mut self) -> Result<Option<WALEntry>> {
        // Read header: 8 (tx_id) + 1 (type) + 4 (len) = 13 bytes
        let mut header = [0u8; WAL_HEADER_SIZE];
        match self.reader.read_exact(&mut header) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                // End of file - no more entries
                return Ok(None);
            }
            Err(e) => return Err(MongoLiteError::Io(e)),
        }

        let tx_id = u64::from_le_bytes(header[0..8].try_into().unwrap());
        let entry_type = WALEntryType::from_u8(header[8])?;
        let data_len = u32::from_le_bytes(header[9..13].try_into().unwrap()) as usize;

        // SECURITY: Prevent OOM from malformed WAL with huge data_len
        if data_len > MAX_WAL_ENTRY_SIZE {
            return Err(MongoLiteError::WALCorruption);
        }

        // Read data
        let mut data = vec![0u8; data_len];
        self.reader.read_exact(&mut data)?;

        // Read checksum
        let mut checksum_bytes = [0u8; 4];
        self.reader.read_exact(&mut checksum_bytes)?;
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

        Ok(Some(entry))
    }
}

impl<R: Read + Seek> Iterator for WALEntryIterator<R> {
    type Item = Result<WALEntry>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.eof_reached {
            return None;
        }

        match self.read_next() {
            Ok(Some(entry)) => Some(Ok(entry)),
            Ok(None) => {
                self.eof_reached = true;
                None
            }
            Err(e) => Some(Err(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_iterator_reads_all_entries() {
        // Create some entries
        let entry1 = WALEntry::new(1, WALEntryType::Begin, vec![]);
        let entry2 = WALEntry::new(1, WALEntryType::Operation, b"data".to_vec());
        let entry3 = WALEntry::new(1, WALEntryType::Commit, vec![]);

        // Serialize them
        let mut data = Vec::new();
        data.extend_from_slice(&entry1.serialize());
        data.extend_from_slice(&entry2.serialize());
        data.extend_from_slice(&entry3.serialize());

        // Create iterator
        let cursor = Cursor::new(data);
        let iter = WALEntryIterator::new(cursor).unwrap();
        let entries: Vec<_> = iter.collect();

        assert_eq!(entries.len(), 3);
        assert!(entries[0].is_ok());
        assert!(entries[1].is_ok());
        assert!(entries[2].is_ok());

        let e1 = entries[0].as_ref().unwrap();
        let e2 = entries[1].as_ref().unwrap();
        let e3 = entries[2].as_ref().unwrap();

        assert_eq!(e1.entry_type, WALEntryType::Begin);
        assert_eq!(e2.entry_type, WALEntryType::Operation);
        assert_eq!(e2.data, b"data".to_vec());
        assert_eq!(e3.entry_type, WALEntryType::Commit);
    }

    #[test]
    fn test_iterator_handles_empty() {
        let cursor = Cursor::new(Vec::<u8>::new());
        let iter = WALEntryIterator::new(cursor).unwrap();
        let entries: Vec<_> = iter.collect();

        assert_eq!(entries.len(), 0);
    }

    #[test]
    fn test_iterator_detects_corruption() {
        let entry = WALEntry::new(1, WALEntryType::Begin, vec![]);
        let mut data = entry.serialize();

        // Corrupt checksum
        let len = data.len();
        data[len - 1] ^= 0xFF;

        let cursor = Cursor::new(data);
        let mut iter = WALEntryIterator::new(cursor).unwrap();
        let result = iter.next();

        assert!(matches!(result, Some(Err(MongoLiteError::WALCorruption))));
    }

    #[test]
    fn test_iterator_handles_interleaved_transactions() {
        // Create interleaved entries from two transactions
        let entries = vec![
            WALEntry::new(1, WALEntryType::Begin, vec![]),
            WALEntry::new(2, WALEntryType::Begin, vec![]),
            WALEntry::new(1, WALEntryType::Operation, b"tx1-op".to_vec()),
            WALEntry::new(2, WALEntryType::Operation, b"tx2-op".to_vec()),
            WALEntry::new(1, WALEntryType::Commit, vec![]),
            WALEntry::new(2, WALEntryType::Commit, vec![]),
        ];

        let mut data = Vec::new();
        for entry in &entries {
            data.extend_from_slice(&entry.serialize());
        }

        let cursor = Cursor::new(data);
        let iter = WALEntryIterator::new(cursor).unwrap();
        let read_entries: Vec<_> = iter.map(|r| r.unwrap()).collect();

        assert_eq!(read_entries.len(), 6);
        assert_eq!(read_entries[0].transaction_id, 1);
        assert_eq!(read_entries[1].transaction_id, 2);
        assert_eq!(read_entries[4].transaction_id, 1);
        assert_eq!(read_entries[5].transaction_id, 2);
    }
}
