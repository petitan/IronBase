//! Writes that a collection's reads must see but that are not in storage yet.
//!
//! In `DurabilityMode::Batch` acknowledged inserts wait in a buffer until the
//! batch is flushed (WAL first, then storage). Every `CollectionCore` handle of
//! such a database holds the database's [`PendingWrites`], and its read entry
//! points flush the collection's pending inserts first, so a read sees every
//! write acknowledged before it ("read your writes", audit 2026-10-08).

use super::{CollectionCore, InsertOnePrepared};
use crate::error::Result;
use crate::storage::{RawStorage, Storage};

pub(crate) trait PendingWrites<S: Storage + RawStorage>: Send + Sync {
    /// Whether `collection` has acknowledged writes that are not in storage.
    fn has_pending(&self, collection: &str) -> bool;

    /// Make the pending writes of `collection.name` visible: commit them to
    /// the WAL and persist them through `collection`.
    fn flush_for_read(&self, collection: &CollectionCore<S>) -> Result<()>;

    /// Flush every collection's pending writes; `persist` writes one
    /// collection's documents to storage after the WAL commit. The caller
    /// holds the write lock (an auto-write guard or a transaction's lock).
    fn flush_all(
        &self,
        persist: &mut dyn FnMut(&str, Vec<InsertOnePrepared>) -> Result<()>,
    ) -> Result<()>;
}
