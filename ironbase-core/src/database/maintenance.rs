// src/database/maintenance.rs
// Database maintenance operations: flush, checkpoint, stats, compact

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
    pub fn compact(&self) -> Result<crate::storage::CompactionStats> {
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

        // Flush all fulltext indexes to disk before closing
        self.flush_fulltext_indexes()?;

        // Flush all pending changes to disk
        self.flush()?;

        // Release the exclusive file lock so other processes can open the database
        let storage = self.storage.read();
        storage.release_lock()
    }

    /// Flush all fulltext indexes to disk
    fn flush_fulltext_indexes(&self) -> Result<()> {
        let index_managers = self.index_managers.read();
        for index_manager in index_managers.values() {
            let mut manager = index_manager.write();
            manager.flush_fulltext_indexes()?;
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

        // 1. Flush fulltext indexes to disk
        let index_managers = self.index_managers.read();
        for index_manager in index_managers.values() {
            let mut manager = index_manager.write();
            if let Err(e) = manager.flush_fulltext_indexes() {
                eprintln!("Warning: Failed to flush fulltext indexes on drop: {}", e);
            }
        }
        drop(index_managers); // Release lock before storage flush

        // 2. Flush storage (metadata + sync)
        // Note: Batch mode pending operations are NOT flushed here because
        // flush_batch() requires StorageEngine-specific methods.
        // For full safety in Batch mode, always call close() explicitly.
        if let Some(mut storage) = self.storage.try_write() {
            if let Err(e) = storage.flush() {
                eprintln!("Warning: Failed to flush storage on drop: {}", e);
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
