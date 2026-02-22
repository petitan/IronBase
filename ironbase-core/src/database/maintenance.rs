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
//! | `checkpoint()` | **Flush indexes** + metadata + clear WAL | Long-running processes (MongoDB-style) |
//! | `compact()` | Remove tombstones, reclaim space | Periodic maintenance |
//! | `close()` | Full flush + release file lock | Before reopening in another process |
//!
//! # Checkpoint (MongoDB-style Index Persistence)
//!
//! The `checkpoint()` method implements MongoDB-style index persistence:
//! 1. Flush all indexes to .idx/.ftidx/.fzidx/.hnsw files
//! 2. Flush metadata (document_catalog)
//! 3. Clear WAL
//!
//! This ensures indexes survive crashes without requiring graceful shutdown.
//! Call periodically (e.g., every 60 seconds) in long-running servers.
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
        // Rebuild HNSW indexes to remove orphan nodes before flush
        {
            let index_managers = self.index_managers.read();
            for (collection_name, index_manager) in index_managers.iter() {
                let mut mgr = index_manager.write();
                let rebuilt = mgr.rebuild_all_vector_indexes()?;
                if rebuilt > 0 {
                    tracing::info!(
                        collection = %collection_name, rebuilt,
                        "Compact: HNSW indexes rebuilt"
                    );
                }
            }
        }

        // Flush all indexes
        self.flush_all_indexes()?;

        let mut storage = self.storage.write();
        storage.compact()
    }

    /// Checkpoint - flush indexes and clear WAL (MongoDB-style)
    ///
    /// This is the recommended way to ensure index durability in long-running processes.
    /// Unlike `close()`, this does NOT release the file lock, so the database remains usable.
    ///
    /// The checkpoint performs:
    /// 1. Flush all indexes to .idx/.ftidx/.fzidx/.hnsw files
    /// 2. Flush metadata to ensure document_catalog is persisted
    /// 3. Clear the WAL (all operations already in main file)
    ///
    /// # MongoDB Comparison
    ///
    /// This is similar to MongoDB's WiredTiger checkpoint mechanism:
    /// - MongoDB checkpoints every 60 seconds or 2GB of journal data
    /// - IronBase checkpoints on-demand via this method or MCP `checkpoint` tool
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use ironbase_core::DatabaseCore;
    /// use ironbase_core::storage::StorageEngine;
    ///
    /// let db = DatabaseCore::<StorageEngine>::open("data.mlite")?;
    /// // ... many operations ...
    /// let stats = db.checkpoint()?; // Flush indexes + clear WAL
    /// println!("Flushed {} indexes, cleared {} WAL ops", stats.indexes_flushed, stats.wal_ops_cleared);
    /// # Ok::<(), ironbase_core::IronBaseError>(())
    /// ```
    pub fn checkpoint(&self) -> Result<crate::storage::CheckpointStats> {
        // 1. Flush all indexes to disk first (like MongoDB's checkpoint)
        let indexes_flushed = self.flush_all_indexes_counted()?;

        // 2. Flush metadata and clear WAL
        let mut storage = self.storage.write();
        let mut stats = storage.checkpoint()?;

        // 3. Add index count to stats
        stats.indexes_flushed = indexes_flushed;

        Ok(stats)
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

        // Checkpoint: flush indexes + metadata + clear WAL
        // This ensures all data is persisted and WAL is empty
        self.checkpoint()?;

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

    /// Flush metadata and clear WAL only (without index flush).
    ///
    /// Uses a two-phase approach to minimize storage.write() lock hold time:
    ///
    /// **Phase A** (storage.read() — doesn't block inserts):
    /// Pre-serialize metadata buffer (~6-12MB for 130K docs).
    ///
    /// **Phase B** (storage.write() — brief):
    /// If no mutations happened since Phase A (guard check: data_end_offset + WAL size),
    /// write pre-serialized buffer to file + fsync + WAL clear.
    /// Otherwise, fall back to full checkpoint under lock.
    ///
    /// PERF FIX (v1.0.313): Previously held storage.write() for the entire
    /// serialize + write + fsync cycle. With 130K+ docs under Windows memory
    /// pressure, serialization alone took minutes due to page swapping,
    /// blocking all concurrent insert_one operations.
    ///
    /// Use this together with `flush_all_indexes()` for two-phase checkpoint:
    /// 1. `flush_all_indexes()` with db.read() (slow, but doesn't block inserts)
    /// 2. `checkpoint_wal_only()` — pre-serialize + brief lock
    pub fn checkpoint_wal_only(&self) -> Result<crate::storage::CheckpointStats> {
        // Phase A: Pre-serialize metadata under storage.read()
        // NOTE: storage.read() DOES block storage.write() callers (insert WAL commit)
        let pre_serialized = {
            let t = std::time::Instant::now();
            let storage = self.storage.read();
            let lock_wait_ms = t.elapsed().as_millis() as u64;
            if !storage.is_metadata_dirty() {
                tracing::info!(
                    lock_wait_ms,
                    "Checkpoint WAL Phase A: metadata clean, skipping"
                );
                None
            } else {
                tracing::info!(
                    lock_wait_ms,
                    "Checkpoint WAL Phase A: storage.read() acquired, serializing..."
                );
                let serialize_start = std::time::Instant::now();
                let metadata_bytes = StorageEngine::serialize_metadata(storage.collections_ref())?;
                let serialize_ms = serialize_start.elapsed().as_millis() as u64;
                let data_end_offset = storage.data_end_offset();
                let wal_size = storage.wal_file_size();
                tracing::info!(
                    serialize_ms,
                    bytes = metadata_bytes.len(),
                    "Checkpoint WAL Phase A: serialized, releasing storage.read()"
                );
                Some((metadata_bytes, data_end_offset, wal_size))
            }
        }; // storage.read() released — inserts can proceed

        // Phase B: Write + WAL clear under storage.write() (brief)
        let t = std::time::Instant::now();
        let mut storage = self.storage.write();
        let lock_wait_ms = t.elapsed().as_millis() as u64;
        tracing::info!(
            lock_wait_ms,
            "Checkpoint WAL Phase B: storage.write() acquired"
        );
        match pre_serialized {
            Some((metadata_bytes, snapshot_data_end, snapshot_wal_size))
                if snapshot_data_end >= crate::storage::HEADER_SIZE
                    && storage.data_end_offset() == snapshot_data_end
                    && storage.wal_file_size() == snapshot_wal_size =>
            {
                // Guard PASS: no mutations since Phase A — use pre-serialized buffer
                // Lock holds only: file write (~6MB) + header (256B) + fsync + WAL clear
                //
                // Guard conditions:
                // 1. data_end_offset >= HEADER_SIZE: v3+ database (v2 needs migration
                //    logic in flush_metadata that we don't replicate here)
                // 2. data_end_offset unchanged: no new documents written since Phase A
                // 3. WAL size unchanged: no new WAL commits since Phase A
                //
                // NOTE: Pre-serialized buffer uses collections' current data_offset/
                // index_offset values without normalization (flush_metadata sets these
                // to HEADER_SIZE). This is safe because: (a) v3+ databases always have
                // HEADER_SIZE after any previous flush, and (b) create_collection()
                // calls flush() which normalizes immediately.
                storage.checkpoint_with_preserialized(metadata_bytes)
            }
            _ => {
                // Guard FAIL: mutations happened between Phase A and B, v2 migration
                // needed, or no dirty metadata.
                // Fall back to full checkpoint under lock (serialize + write + fsync).
                // This is the same as the previous behavior — no worse than before.
                storage.checkpoint()
            }
        }
    }

    /// Flush all indexes (B+ tree, fulltext, fuzzy, and vector) to disk
    fn flush_all_indexes(&self) -> Result<()> {
        self.flush_all_indexes_counted()?;
        Ok(())
    }

    /// Flush all indexes and return count of flushed index files.
    ///
    /// This only writes index files (.idx, .ftidx, .fzidx, .hnsw) to disk.
    /// It does NOT touch the WAL or metadata. Safe to call with db.read().
    ///
    /// # Known issue: fulltext flush blocks insert_one (v1.0.314)
    ///
    /// Each index flush holds `index_manager.write()` for the ENTIRE serialize + file write.
    /// Under memory pressure (page swapping), fulltext flush for 130K docs takes 13s+ (normal)
    /// to 10+ MINUTES (swapping). During this time, `insert_one` blocks at
    /// `check_index_constraints()` which needs `indexes.read()` — blocked by the writer.
    ///
    /// **Measured (v1.0.314 tracing):**
    /// - `emails_body.plain_fts` fulltext: 13,203ms flush under `index_manager.write()`
    /// - `attachment_contents_markdown_fts`: 4,060ms flush
    /// - All btree indexes: < 300ms each
    /// - Metadata serialize: 57ms (8.4MB) — NOT the bottleneck
    ///
    /// # TODO: Arc COW snapshot (iparági sztenderd megoldás)
    ///
    /// Wrap index data in `Arc<T>`, use copy-on-write to eliminate lock contention:
    ///
    /// ```text
    /// FLUSH (checkpoint thread):
    ///   1. Brief write lock: snapshot = Arc::clone(&self.data)  // microseconds
    ///   2. Release lock
    ///   3. Serialize snapshot → file                             // minutes, NO LOCK
    ///   4. Brief write lock: dirty = false
    ///
    /// INSERT (concurrent):
    ///   1. Arc::make_mut(&mut self.data)  // COW: clone only if flush holds a ref
    ///   2. Modify in-place
    /// ```
    ///
    /// Affected files:
    /// - `ironbase-core/src/fulltext.rs` — FulltextIndex internals → Arc<FulltextData>
    /// - `ironbase-core/src/index/fuzzy.rs` — FuzzyIndex internals → Arc<FuzzyData>
    /// - `ironbase-core/src/index/manager.rs` — flush_one_* methods: Arc::clone + lockless write
    /// - This file — flush loop: brief lock for Arc::clone, no lock for serialize+write
    pub fn flush_all_indexes_counted(&self) -> Result<usize> {
        let db_path = {
            let storage = self.storage.read();
            storage.get_file_path().to_string()
        };

        let mut total_flushed = 0usize;
        let index_managers = self.index_managers.read();
        for (collection_name, index_manager) in index_managers.iter() {
            // 1. Collect dirty names under brief READ lock
            let (dirty_bt, dirty_ft, dirty_fz, dirty_vec) = {
                let mgr = index_manager.read();
                (
                    mgr.dirty_btree_index_names(),
                    mgr.dirty_fulltext_index_names(),
                    mgr.dirty_fuzzy_index_names(),
                    mgr.dirty_vector_index_names(),
                )
            };

            let dirty_total = dirty_bt.len() + dirty_ft.len() + dirty_fz.len() + dirty_vec.len();
            if dirty_total > 0 {
                tracing::info!(
                    collection = %collection_name,
                    btree = dirty_bt.len(),
                    fulltext = dirty_ft.len(),
                    fuzzy = dirty_fz.len(),
                    vector = dirty_vec.len(),
                    "Checkpoint: flushing dirty indexes"
                );
            }

            // 2. Flush one-by-one, lock/unlock between each index
            //    This reduces lock hold time from O(all_indexes) to O(1_index),
            //    allowing insert_one/add_to_indexes to proceed between flushes.
            for name in &dirty_ft {
                let t = std::time::Instant::now();
                let mut mgr = index_manager.write();
                let lock_wait_ms = t.elapsed().as_millis() as u64;
                if mgr.flush_one_fulltext_index(name)? {
                    let flush_ms = t.elapsed().as_millis() as u64 - lock_wait_ms;
                    tracing::info!(
                        collection = %collection_name, index = %name,
                        kind = "fulltext", lock_wait_ms, flush_ms,
                        "Index flushed"
                    );
                    total_flushed += 1;
                }
            }
            for name in &dirty_fz {
                let t = std::time::Instant::now();
                let mut mgr = index_manager.write();
                let lock_wait_ms = t.elapsed().as_millis() as u64;
                if mgr.flush_one_fuzzy_index(name)? {
                    let flush_ms = t.elapsed().as_millis() as u64 - lock_wait_ms;
                    tracing::info!(
                        collection = %collection_name, index = %name,
                        kind = "fuzzy", lock_wait_ms, flush_ms,
                        "Index flushed"
                    );
                    total_flushed += 1;
                }
            }
            for name in &dirty_bt {
                let t = std::time::Instant::now();
                let mut mgr = index_manager.write();
                let lock_wait_ms = t.elapsed().as_millis() as u64;
                if mgr.flush_one_btree_index(name, &db_path)? {
                    let flush_ms = t.elapsed().as_millis() as u64 - lock_wait_ms;
                    tracing::info!(
                        collection = %collection_name, index = %name,
                        kind = "btree", lock_wait_ms, flush_ms,
                        "Index flushed"
                    );
                    total_flushed += 1;
                }
            }
            // HNSW orphan compaction: rebuild indexes with >30% orphan ratio before flush
            {
                let t = std::time::Instant::now();
                let mut mgr = index_manager.write();
                let lock_wait_ms = t.elapsed().as_millis() as u64;
                let rebuilt = mgr.rebuild_vector_indexes_if_needed()?;
                if rebuilt > 0 {
                    let rebuild_ms = t.elapsed().as_millis() as u64 - lock_wait_ms;
                    tracing::info!(
                        collection = %collection_name,
                        rebuilt, lock_wait_ms, rebuild_ms,
                        "HNSW orphan compaction"
                    );
                }
            }
            for name in &dirty_vec {
                let t = std::time::Instant::now();
                let mut mgr = index_manager.write();
                let lock_wait_ms = t.elapsed().as_millis() as u64;
                if mgr.flush_one_vector_index(name, &db_path)? {
                    let flush_ms = t.elapsed().as_millis() as u64 - lock_wait_ms;
                    tracing::info!(
                        collection = %collection_name, index = %name,
                        kind = "vector", lock_wait_ms, flush_ms,
                        "Index flushed"
                    );
                    total_flushed += 1;
                }
            }
        }
        Ok(total_flushed)
    }

    /// Flush only B+ tree indexes to disk (skip fulltext, fuzzy, vector).
    ///
    /// Used by periodic checkpoint to avoid blocking `insert_one` for minutes.
    /// Fulltext flush holds `index_manager.write()` for 13+ seconds (130K docs),
    /// blocking `insert_one`'s `check_index_constraints()` which needs `indexes.read()`.
    /// B+ tree indexes flush in < 300ms each — acceptable lock hold time.
    ///
    /// Fulltext/fuzzy/vector indexes are flushed during `close()` and `compact()` only.
    /// On dirty shutdown, these indexes are rebuilt from documents (safe, just slower startup).
    pub fn flush_btree_indexes_counted(&self) -> Result<usize> {
        let db_path = {
            let storage = self.storage.read();
            storage.get_file_path().to_string()
        };

        let mut total_flushed = 0usize;
        let index_managers = self.index_managers.read();
        for (collection_name, index_manager) in index_managers.iter() {
            let dirty_bt = {
                let mgr = index_manager.read();
                mgr.dirty_btree_index_names()
            };

            if !dirty_bt.is_empty() {
                tracing::info!(
                    collection = %collection_name,
                    btree = dirty_bt.len(),
                    "Checkpoint: flushing dirty btree indexes (fulltext/fuzzy skipped)"
                );
            }

            for name in &dirty_bt {
                let t = std::time::Instant::now();
                let mut mgr = index_manager.write();
                let lock_wait_ms = t.elapsed().as_millis() as u64;
                if mgr.flush_one_btree_index(name, &db_path)? {
                    let flush_ms = t.elapsed().as_millis() as u64 - lock_wait_ms;
                    tracing::info!(
                        collection = %collection_name, index = %name,
                        kind = "btree", lock_wait_ms, flush_ms,
                        "Index flushed"
                    );
                    total_flushed += 1;
                }
            }
        }
        Ok(total_flushed)
    }
}

// ============================================================================
// MEMORYSTORAGE-SPECIFIC MAINTENANCE OPERATIONS
// ============================================================================

impl DatabaseCore<crate::storage::MemoryStorage> {
    /// Checkpoint for MemoryStorage (no-op since there's no disk)
    ///
    /// Returns default stats. MemoryStorage doesn't persist to disk,
    /// so checkpoint is a no-op. This method exists for API compatibility.
    pub fn checkpoint(&self) -> Result<crate::storage::CheckpointStats> {
        Ok(crate::storage::CheckpointStats::default())
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

        // 2. Flush all indexes to disk (B+ tree + fulltext + fuzzy + vector)
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
                if let Err(e) = manager.flush_vector_indexes(&db_path) {
                    eprintln!("Warning: Failed to flush vector indexes on drop: {}", e);
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

    /// Get database path
    pub fn path(&self) -> &str {
        &self.db_path
    }

    /// Get current durability mode
    pub fn durability_mode(&self) -> DurabilityMode {
        self.durability_mode
    }
}
