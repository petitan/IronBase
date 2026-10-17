//! # WAL Reader - Streaming WAL Olvasó
//!
//! Iterator pattern a WAL bejegyzések memória-hatékony olvasásához.
//!
//! ## Memória Használat
//!
//! ```text
//! WALEntryIterator<R: Read + Seek>
//! ┌─────────────────────────────────────────────────────────┐
//! │  Memória: O(single entry) - NEM O(entire WAL)           │
//! │  Entry-nként olvas a fájlból                            │
//! │  EOF → None visszatérés (iterator vége)                 │
//! │  Hibás rekord, utána nincs ép → None (szakadt vég)      │
//! │  Hibás rekord, utána van ép → Err(WALCorruption)        │
//! └─────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Használat
//!
//! ```rust,ignore
//! let file = File::open("data.wal")?;
//! let iter = WALEntryIterator::new(BufReader::new(file))?;
//! for entry_result in iter {
//!     let entry = entry_result?;
//!     // Process entry...
//! }
//! ```

use std::io::{Read, Seek, SeekFrom};

use crate::error::{IronBaseError, Result};

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
        let start = self.reader.stream_position()?;

        // Read header: 8 (tx_id) + 1 (type) + 4 (len) = 13 bytes
        let mut header = [0u8; WAL_HEADER_SIZE];
        match self.reader.read_exact(&mut header) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                // End of file - no more entries
                return Ok(None);
            }
            Err(e) => return Err(IronBaseError::Io(e)),
        }

        let tx_id = u64::from_le_bytes(
            header[0..8]
                .try_into()
                .map_err(|_| IronBaseError::WALCorruption)?,
        );
        let data_len = u32::from_le_bytes(
            header[9..13]
                .try_into()
                .map_err(|_| IronBaseError::WALCorruption)?,
        ) as usize;

        // SECURITY: Prevent OOM from malformed WAL with huge data_len
        if data_len > MAX_WAL_ENTRY_SIZE {
            return self.invalid_record(start);
        }

        // Read data. A SHORT read here is a torn trailing entry: the process
        // crashed while appending this record, so its data/checksum never fully
        // reached disk (audit 2026-06-08 P1-2). Unless a valid record follows
        // (a damaged length inside the log), it ends the log.
        let mut data = vec![0u8; data_len];
        match self.reader.read_exact(&mut data) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                return self.invalid_record(start)
            }
            Err(e) => return Err(IronBaseError::Io(e)),
        }

        // Read checksum (same torn-tail handling — a crash can land here too).
        let mut checksum_bytes = [0u8; 4];
        match self.reader.read_exact(&mut checksum_bytes) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                return self.invalid_record(start)
            }
            Err(e) => return Err(IronBaseError::Io(e)),
        }
        let checksum = u32::from_le_bytes(checksum_bytes);

        // The checksum covers the raw header bytes and the data
        let mut hasher = crc32fast::Hasher::new();
        hasher.update(&header);
        hasher.update(&data);
        if hasher.finalize() != checksum {
            return self.invalid_record(start);
        }

        // A checksummed record of a type this version does not know was
        // written by a newer one: an error, never a torn tail to drop
        let entry_type = WALEntryType::from_u8(header[8])?;

        Ok(Some(WALEntry {
            transaction_id: tx_id,
            entry_type,
            data,
            checksum,
        }))
    }

    /// An invalid record at `start`: the end of the log if no valid record
    /// follows it, corruption otherwise.
    ///
    /// A crash can leave a tail the WAL never finished writing: cut short,
    /// zero-filled (the file grew but its blocks never reached the disk) or
    /// with a bad checksum (only part of the record reached it). Nothing
    /// after it was committed, so it ends the log; failing on it made the
    /// database impossible to open (WAL/recovery audit 2026-10-14 W6). A
    /// valid record after it means the damage is inside the log: stopping
    /// there would silently drop committed transactions, so it is an error,
    /// and the reader is left at that record for callers that skip errors.
    fn invalid_record(&mut self, start: u64) -> Result<Option<WALEntry>> {
        if let Some(next) = self.valid_entry_after(start)? {
            self.reader.seek(SeekFrom::Start(next))?;
            return Err(IronBaseError::WALCorruption);
        }
        let end = self.reader.seek(SeekFrom::End(0))?;
        tracing::warn!(
            offset = start,
            discarded_bytes = end - start,
            "WAL ends in an invalid record (torn write); treating it as the end of the log"
        );
        Ok(None)
    }

    /// Offset of the first checksummed record after `start`, if any.
    ///
    /// Scans the rest of the file in chunks (O(chunk) memory). Offsets whose
    /// length fits the file are checksummed in the chunk; a longer candidate
    /// is checksummed from the file in pieces. That second kind of work is
    /// bounded: past `SCAN_VERIFY_BUDGET` bytes the tail cannot be shown to
    /// be torn and is reported as corruption, never dropped.
    fn valid_entry_after(&mut self, start: u64) -> Result<Option<u64>> {
        const CHUNK: usize = 1024 * 1024;
        const SCAN_VERIFY_BUDGET: u64 = 256 * 1024 * 1024;
        let end = self.reader.seek(SeekFrom::End(0))?;
        let min_entry = WAL_HEADER_SIZE + 4;
        let mut verified: u64 = 0;
        let mut pos = start + 1;
        let mut buf = Vec::new();
        while pos + min_entry as u64 <= end {
            // The window also holds the header of its last candidate offset
            let window = (end - pos).min((CHUNK + WAL_HEADER_SIZE) as u64) as usize;
            buf.resize(window, 0);
            self.reader.seek(SeekFrom::Start(pos))?;
            self.reader.read_exact(&mut buf)?;
            let candidates = (window - WAL_HEADER_SIZE + 1).min(CHUNK);
            for i in 0..candidates {
                let header = &buf[i..i + WAL_HEADER_SIZE];
                let data_len =
                    u32::from_le_bytes([header[9], header[10], header[11], header[12]]) as usize;
                let offset = pos + i as u64;
                let entry_len = min_entry + data_len;
                if data_len > MAX_WAL_ENTRY_SIZE || offset + entry_len as u64 > end {
                    continue;
                }
                let valid = if i + entry_len <= window {
                    let body_end = i + entry_len - 4;
                    let stored = &buf[body_end..body_end + 4];
                    crc32fast::hash(&buf[i..body_end])
                        == u32::from_le_bytes([stored[0], stored[1], stored[2], stored[3]])
                } else {
                    verified += entry_len as u64;
                    if verified > SCAN_VERIFY_BUDGET {
                        tracing::warn!(
                            offset = start,
                            "WAL tail scan exceeded its budget; reporting the invalid record as corruption"
                        );
                        return Err(IronBaseError::WALCorruption);
                    }
                    self.entry_valid_at(offset, data_len)?
                };
                if valid {
                    return Ok(Some(offset));
                }
            }
            pos += candidates as u64;
        }
        Ok(None)
    }

    /// Whether the record at `offset` checksums, read in pieces from the file.
    fn entry_valid_at(&mut self, offset: u64, data_len: usize) -> Result<bool> {
        let mut piece = vec![0u8; 64 * 1024];
        let mut hasher = crc32fast::Hasher::new();
        self.reader.seek(SeekFrom::Start(offset))?;
        let mut left = WAL_HEADER_SIZE + data_len;
        while left > 0 {
            let n = left.min(piece.len());
            self.reader.read_exact(&mut piece[..n])?;
            hasher.update(&piece[..n]);
            left -= n;
        }
        let mut checksum = [0u8; 4];
        self.reader.read_exact(&mut checksum)?;
        Ok(hasher.finalize() == u32::from_le_bytes(checksum))
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

        // Corrupt checksum, then a valid record: damage inside the log
        let len = data.len();
        data[len - 1] ^= 0xFF;
        data.extend_from_slice(&WALEntry::new(2, WALEntryType::Begin, vec![]).serialize());

        let cursor = Cursor::new(data);
        let mut iter = WALEntryIterator::new(cursor).unwrap();
        let result = iter.next();

        assert!(matches!(result, Some(Err(IronBaseError::WALCorruption))));
    }

    #[test]
    fn test_iterator_ends_at_invalid_tail() {
        // Invalid records with nothing valid after them are a torn tail
        // (WAL/recovery audit 2026-10-14 W6): a bad checksum, zeros, an
        // unknown type or an oversized length all end the log cleanly
        let valid = WALEntry::new(1, WALEntryType::Begin, vec![]).serialize();
        let mut bad_checksum = WALEntry::new(1, WALEntryType::Commit, vec![]).serialize();
        let len = bad_checksum.len();
        bad_checksum[len - 1] ^= 0xFF;
        let mut huge = vec![0u8; WAL_HEADER_SIZE];
        huge[8] = WALEntryType::Operation as u8;
        huge[9..13].copy_from_slice(&u32::MAX.to_le_bytes());
        huge.extend_from_slice(&[0u8; 64]);

        for tail in [bad_checksum, vec![0u8; 4096], vec![0xFF; 100], huge] {
            let mut data = valid.clone();
            data.extend_from_slice(&tail);
            let entries: Vec<_> = WALEntryIterator::new(Cursor::new(data)).unwrap().collect();
            assert_eq!(entries.len(), 1);
            assert!(entries[0].is_ok());
        }
    }

    #[test]
    fn test_iterator_rejects_checksummed_unknown_type() {
        // A record of a type this version does not know, with a valid
        // checksum, was written by a newer version: never a torn tail to drop
        let mut data = WALEntry::new(1, WALEntryType::Begin, vec![]).serialize();
        let mut unknown = WALEntry::new(2, WALEntryType::Commit, b"new".to_vec()).serialize();
        unknown[8] = 0x7F;
        let body_end = unknown.len() - 4;
        let checksum = crc32fast::hash(&unknown[..body_end]);
        unknown[body_end..].copy_from_slice(&checksum.to_le_bytes());
        data.extend_from_slice(&unknown);

        let entries: Vec<_> = WALEntryIterator::new(Cursor::new(data)).unwrap().collect();
        assert!(entries[0].is_ok());
        assert!(matches!(entries[1], Err(IronBaseError::WALCorruption)));
    }

    #[test]
    fn test_iterator_short_read_inside_log_is_corruption() {
        // A damaged length that reaches past the end of the file, with valid
        // records after it: not a torn tail. The reader is left at the next
        // valid record, so a caller that skips the error still reads it.
        let mut damaged = WALEntry::new(1, WALEntryType::Operation, b"op".to_vec()).serialize();
        damaged[9..13].copy_from_slice(&1_000_000u32.to_le_bytes());
        let mut data = damaged;
        data.extend_from_slice(&WALEntry::new(2, WALEntryType::Begin, vec![]).serialize());
        data.extend_from_slice(&WALEntry::new(2, WALEntryType::Commit, vec![]).serialize());

        let mut iter = WALEntryIterator::new(Cursor::new(data)).unwrap();
        assert!(matches!(
            iter.next(),
            Some(Err(IronBaseError::WALCorruption))
        ));
        let next = iter.next().unwrap().unwrap();
        assert_eq!(next.transaction_id, 2);
        assert_eq!(next.entry_type, WALEntryType::Begin);
    }

    #[test]
    fn test_iterator_discards_torn_trailing_entry() {
        // A crash mid-WAL-append leaves a partial final record (audit P1-2). Two
        // truncation points are possible: mid-data and mid-checksum. In BOTH cases
        // recovery must replay the prior COMPLETE entries and stop cleanly — never
        // yield an Err (which previously aborted recovery → DB failed to reopen).
        let committed1 = WALEntry::new(1, WALEntryType::Begin, vec![]);
        let committed2 = WALEntry::new(1, WALEntryType::Operation, b"committed".to_vec());
        let torn = WALEntry::new(1, WALEntryType::Operation, b"never-fully-written".to_vec());
        let torn_bytes = torn.serialize();

        // Truncate at every point INSIDE the final record (just past the header,
        // mid-data, and mid-checksum) — each must be discarded without error.
        for cut in [
            WAL_HEADER_SIZE + 1,  // mid-data
            torn_bytes.len() - 6, // late-data
            torn_bytes.len() - 2, // mid-checksum (header + full data + 2/4 checksum bytes)
        ] {
            let mut data = Vec::new();
            data.extend_from_slice(&committed1.serialize());
            data.extend_from_slice(&committed2.serialize());
            data.extend_from_slice(&torn_bytes[..cut]);

            let iter = WALEntryIterator::new(Cursor::new(data)).unwrap();
            let entries: Vec<_> = iter.collect();

            assert_eq!(
                entries.len(),
                2,
                "torn tail (cut={cut}) must be discarded, not errored"
            );
            assert!(
                entries.iter().all(|e| e.is_ok()),
                "a torn tail (cut={cut}) must not surface an error"
            );
            assert_eq!(entries[1].as_ref().unwrap().data, b"committed".to_vec());
        }
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
