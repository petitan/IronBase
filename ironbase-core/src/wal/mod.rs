// wal/mod.rs
// Write-Ahead Log module
//
// This module provides:
// - `WALEntry` and `WALEntryType`: Entry types and serialization
// - `WALEntryIterator`: Streaming reader for WAL files
// - `WriteAheadLog`: WAL file manager (append, flush, clear)
// - `TransactionGrouper`: Streaming transaction aggregation
// - `CommittedTransaction`: Grouped transaction entries

mod entry;
mod reader;
mod recovery;
mod writer;

pub use entry::{WALEntry, WALEntryType, MAX_WAL_ENTRY_SIZE, WAL_HEADER_SIZE};
pub use reader::WALEntryIterator;
pub use recovery::{CommittedTransaction, TransactionGrouper};
pub use writer::WriteAheadLog;
