//! Database Maintenance Operations
//!
//! This module provides database lifecycle and maintenance operations:
//! flush, checkpoint, compact, close, and graceful shutdown.
//!
//! # Operations Overview
//!
//! | Operation | Effect | When to Use |
//! |-----------|--------|-------------|
//! | `flush()` | Persist all pending changes + metadata | After critical writes |
//! | `checkpoint()` | Clear WAL without full metadata sync | Long-running processes |
//! | `compact()` | Remove tombstones, reclaim space | Periodic maintenance |
//! | `close()` | Full flush + release file lock | Before reopening in another process |
//!
//! # Shutdown Sequence
//!
//! ```text
//! close() or Drop::drop()
//!     │
//!     ├── 1. Mark is_closed = true (prevent new ops)
//!     ├── 2. Flush indexes to .idx/.ftidx/.fzidx files
//!     ├── 3. Flush storage (metadata + fsync)
//!     ├── 4. Mark clean_shutdown flag in header
//!     └── 5. Release file lock
//! ```
//!
//! # Clean Shutdown Optimization
//!
//! When `clean_shutdown` flag is set in the file header:
//! - Next startup trusts persisted indexes (no rebuild needed)
//! - Warm-up time: <1s instead of ~100s for 70K documents
//!
//! # Drop Trait Behavior
//!
//! The `Drop` implementation performs the same shutdown sequence as `close()`,
//! but with error handling that logs warnings instead of returning errors.
//! For language bindings (Python, C#) where GC timing is unpredictable,
//! always call `close()` explicitly to ensure deterministic cleanup.
//!
//! # Batch Mode Warning
//!
//! In `DurabilityMode::Batch`, uncommitted operations are NOT automatically
//! persisted on `Drop`. Always call `close()` to flush pending batch operations.

use std::sync::atomic::Ordering;

use crate::durability::DurabilityMode;
use crate::error::Result;
use crate::storage::{RawStorage, Storage, StorageEngine};

use super::durability::BatchFlush;
use super::DatabaseCore;

// ============================================================================
// STORAGEENGINE-SPECIFIC MAINTENANCE OPERATIONS
// ============================================================================

impl DatabaseCore<StorageEngine> {
    /// Get database statistics as JSON (StorageEngine-specific)
    pub fn stats(&self) -> serde_json::Value {
        let storage = self.storage.read();
        storage.stats()
    }

    /// Storage compaction - removes tombstones and old document versions (StorageEngine-specific)
    ///
    /// Also flushes all indexes to disk (B+ tree and fulltext).
    pub fn compact(&self) -> Result<crate::storage::CompactionStats> {
        // Flush all indexes first
        self.flush_all_indexes()?;

        let mut storage = self.storage.write();
        storage.compact()
    }

    /// Close the database: flush all changes and release the file lock.
    ///
    /// This method is primarily useful for language bindings (Python, C#) where
    /// the garbage collector timing is unpredictable. After calling `close()`,
    /// another process can open the same database file immediately.
    ///
    /// Note: The database instance should not be used after calling `close()`.
    /// While the struct remains valid, the file lock is released and concurrent
    /// access from other processes is possible.
    ///
    /// # Example
    /// ```rust,no_run
    /// use ironbase_core::DatabaseCore;
    /// use ironbase_core::storage::StorageEngine;
    ///
    /// let db = DatabaseCore::<StorageEngine>::open("data.mlite")?;
    /// db.collection("users")?; // Create collection
    /// db.close()?; // Flush and release lock - now safe to reopen
    /// # Ok::<(), ironbase_core::IronBaseError>(())
    /// ```
    pub fn close(&self) -> Result<()> {
        // Mark as closed FIRST to prevent new operations
        self.is_closed.store(true, Ordering::SeqCst);

        // Flush all indexes to disk before closing
        // This enables fast restart (clean shutdown optimization)
        self.flush_all_indexes()?;

        // Flush all pending changes to disk
        self.flush()?;

        // Mark as clean shutdown BEFORE releasing lock
        // This enables fast startup next time (indexes can be trusted)
        {
            let mut storage = self.storage.write();
            storage.mark_clean_shutdown()?;
        }

        // Release the exclusive file lock so other processes can open the database
        let storage = self.storage.read();
        storage.release_lock()
    }

    /// Flush all indexes (B+ tree, fulltext, and fuzzy) to disk
    fn flush_all_indexes(&self) -> Result<()> {
        let db_path = {
            let storage = self.storage.read();
            storage.get_file_path().to_string()
        };

        let index_managers = self.index_managers.read();
        for index_manager in index_managers.values() {
            let mut manager = index_manager.write();
            manager.flush_fulltext_indexes()?;
            manager.flush_fuzzy_indexes()?;
            manager.flush_btree_indexes(&db_path)?;
        }
        Ok(())
    }
}

impl<S: Storage + RawStorage> Drop for DatabaseCore<S> {
    fn drop(&mut self) {
        // Skip flush if already closed
        if self.is_closed.load(Ordering::SeqCst) {
            return;
        }

        // 1. Get db_path for index persistence
        let db_path = {
            if let Some(storage) = self.storage.try_read() {
                storage.get_file_path().to_string()
            } else {
                String::new()
            }
        };

        // 2. Flush all indexes to disk (B+ tree + fulltext + fuzzy)
        let index_managers = self.index_managers.read();
        for index_manager in index_managers.values() {
            let mut manager = index_manager.write();
            if let Err(e) = manager.flush_fulltext_indexes() {
                eprintln!("Warning: Failed to flush fulltext indexes on drop: {}", e);
            }
            if let Err(e) = manager.flush_fuzzy_indexes() {
                eprintln!("Warning: Failed to flush fuzzy indexes on drop: {}", e);
            }
            if !db_path.is_empty() {
                if let Err(e) = manager.flush_btree_indexes(&db_path) {
                    eprintln!("Warning: Failed to flush btree indexes on drop: {}", e);
                }
            }
        }
        drop(index_managers); // Release lock before storage flush

        // 3. Flush storage (metadata + sync)
        // Note: Batch mode pending operations are NOT flushed here because
        // flush_batch() requires StorageEngine-specific methods.
        // For full safety in Batch mode, always call close() explicitly.
        if let Some(mut storage) = self.storage.try_write() {
            if let Err(e) = storage.flush() {
                eprintln!("Warning: Failed to flush storage on drop: {}", e);
            }
            // 4. Mark as clean shutdown for fast restart
            if let Err(e) = storage.mark_clean_shutdown() {
                eprintln!("Warning: Failed to mark clean shutdown on drop: {}", e);
            }
        } else {
            eprintln!("Warning: Could not acquire storage lock on drop");
        }
    }
}

// ============================================================================
// GENERIC MAINTENANCE OPERATIONS (all storage backends)
// ============================================================================

impl<S: Storage + RawStorage> DatabaseCore<S> {
    /// Flush all changes to disk
    pub fn flush(&self) -> Result<()>
    where
        DatabaseCore<S>: BatchFlush,
    {
        // Ensure any pending batch operations are flushed before metadata sync
        self.flush_pending_batch()?;

        let mut storage = self.storage.write();
        storage.flush()
    }

    /// Checkpoint - Clear WAL without flushing metadata
    /// Use this in long-running processes to prevent WAL file growth
    ///
    /// Returns checkpoint statistics including WAL size before/after.
    pub fn checkpoint(&self) -> Result<crate::storage::CheckpointStats> {
        let mut storage = self.storage.write();
        storage.checkpoint()
    }

    /// Get database path
    pub fn path(&self) -> &str {
        &self.db_path
    }

    /// Get current durability mode
    pub fn durability_mode(&self) -> DurabilityMode {
        self.durability_mode
    }
}
