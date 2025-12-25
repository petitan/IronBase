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
    /// # Ok::<(), ironbase_core::MongoLiteError>(())
    /// ```
    pub fn close(&self) -> Result<()> {
        // Mark as closed FIRST to prevent new operations
        self.is_closed.store(true, Ordering::SeqCst);

        // Flush all pending changes to disk
        self.flush()?;

        // Release the exclusive file lock so other processes can open the database
        let storage = self.storage.read();
        storage.release_lock()
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
    pub fn checkpoint(&self) -> Result<()> {
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
