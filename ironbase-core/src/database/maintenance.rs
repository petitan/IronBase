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
use crate::log_warn;
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

    /// Compute storage wastage statistics for auto-compaction decisions.
    ///
    /// O(C) cost — iterates collections for document counts, reads file size via fs::metadata.
    /// Does NOT load documents or indexes.
    pub fn storage_wastage(&self) -> crate::storage::StorageWastage {
        let storage = self.storage.read();

        let mut total_writes: u64 = 0;
        let mut total_live: u64 = 0;
        for meta in storage.collections_ref().values() {
            total_writes += meta.document_count;
            total_live += meta.live_document_count;
        }
        let dead_writes = total_writes.saturating_sub(total_live);
        drop(storage); // Release read lock early

        let file_size_bytes = std::fs::metadata(&self.db_path)
            .map(|m| m.len())
            .unwrap_or(0);

        let estimated_live_bytes = self.last_compact_size.load(Ordering::Relaxed);

        let bloat_ratio = if estimated_live_bytes == 0 {
            f64::INFINITY
        } else {
            file_size_bytes as f64 / estimated_live_bytes as f64
        };

        crate::storage::StorageWastage {
            file_size_bytes,
            estimated_live_bytes,
            bloat_ratio,
            total_writes,
            total_live,
            dead_writes,
        }
    }

    /// Store the file size after the last successful compaction.
    ///
    /// Called by the MCP layer after compact finishes to calibrate bloat_ratio.
    pub fn set_last_compact_size(&self, size: u64) {
        self.last_compact_size.store(size, Ordering::Relaxed);
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
        let stats = storage.compact()?;
        // Calibrate last_compact_size for bloat_ratio calculations
        self.set_last_compact_size(stats.size_after);
        Ok(stats)
    }

    /// Non-blocking compact: 3-phase lock splitting
    ///
    /// Pre-Phase: HNSW rebuild + index flush (index_manager locks, NOT storage)
    /// Phase A: brief storage.write() — snapshot
    /// Phase B: NO LOCK — standalone pread scan
    /// Phase C: brief storage.write() — catch-up reconciliation + finalize
    ///
    /// The progress_callback receives (documents_processed, total_documents).
    /// Cancellation is controlled via CompactionConfig.cancel_flag.
    pub fn compact_nonblocking(
        &self,
        config: &crate::storage::CompactionConfig,
        progress_callback: &dyn Fn(u64, u64),
    ) -> Result<crate::storage::CompactionStats> {
        // Guard: prevent concurrent compaction at DatabaseCore level
        // (MCP adapter has its own guard; this protects direct Rust callers)
        if self
            .is_compacting
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::SeqCst,
                std::sync::atomic::Ordering::SeqCst,
            )
            .is_err()
        {
            return Err(crate::error::IronBaseError::OperationNotAllowed(
                "Compaction already in progress".to_string(),
            ));
        }

        // Ensure flag is cleared on all exit paths (success, error, panic)
        let result = self.compact_nonblocking_inner(config, progress_callback);
        self.is_compacting
            .store(false, std::sync::atomic::Ordering::SeqCst);
        // Calibrate last_compact_size on success
        if let Ok(ref stats) = result {
            self.set_last_compact_size(stats.size_after);
        }
        result
    }

    /// Inner implementation of compact_nonblocking (called within CAS guard)
    fn compact_nonblocking_inner(
        &self,
        config: &crate::storage::CompactionConfig,
        progress_callback: &dyn Fn(u64, u64),
    ) -> Result<crate::storage::CompactionStats> {
        // Pre-Phase: HNSW rebuild + index flush (does NOT hold storage lock)
        {
            let index_managers = self.index_managers.read();
            for (collection_name, index_manager) in index_managers.iter() {
                let mut mgr = index_manager.write();
                let rebuilt = mgr.rebuild_all_vector_indexes()?;
                if rebuilt > 0 {
                    tracing::info!(
                        collection = %collection_name, rebuilt,
                        "Compact (non-blocking): HNSW indexes rebuilt"
                    );
                }
            }
        }
        self.flush_all_indexes()?;

        // Phase A: brief storage.write() — snapshot
        let (snapshot, snapshot_collections) = {
            let mut storage = self.storage.write();
            let snapshot = storage.compact_prepare()?;
            // Arc::clone is O(1) — no deep copy of collection metadata
            let snap_colls = std::sync::Arc::clone(&snapshot.snapshot_collections);
            (snapshot, snap_colls)
        }; // storage.write() RELEASED

        // Phase B: NO LOCK — standalone pread scan
        let scan_result =
            crate::storage::compact_scan_standalone(snapshot, config, progress_callback)?;

        // Phase C: brief storage.write() — catch-up + finalize
        let mut storage = self.storage.write();
        storage.compact_finalize_with_catchup(scan_result, &snapshot_collections)
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
    /// # Parallelism (v1.0.351+)
    ///
    /// Two levels of parallelism (feature-gated: `parallel`):
    ///
    /// 1. **Cross-collection**: Multiple collections flush concurrently via `par_iter()`.
    ///    Each collection has its own `Arc<RwLock<IndexManager>>` — no contention.
    ///
    /// 2. **Within-collection fulltext batch**: All fulltext indexes in one collection
    ///    use batched three-phase flush: take all snapshots (single brief write lock),
    ///    serialize all in parallel (NO lock), commit all (single brief write lock).
    ///    N fulltext indexes at 13s each → ~13s total instead of N×13s.
    ///
    /// Btree/fuzzy/vector flushes remain sequential (< 300ms each, lock contention
    /// dominates over I/O — parallelizing would just serialize on the write lock).
    pub fn flush_all_indexes_counted(&self) -> Result<usize> {
        let db_path = {
            let storage = self.storage.read();
            storage.get_file_path().to_string()
        };

        let index_managers = self.index_managers.read();

        // Parallel across collections: each collection has its own
        // Arc<RwLock<IndexManager>>, so they can flush concurrently.
        #[cfg(feature = "parallel")]
        {
            let cpus = std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1);

            if cpus > 1 && index_managers.len() > 1 {
                use rayon::prelude::*;

                let results: Vec<Result<usize>> = index_managers
                    .par_iter()
                    .map(|(collection_name, index_manager)| {
                        Self::flush_collection_indexes(collection_name, index_manager, &db_path)
                    })
                    .collect();

                let mut total = 0;
                for r in results {
                    total += r?;
                }
                return Ok(total);
            }
        }

        // Sequential fallback: single collection, single core, or no parallel feature
        let mut total_flushed = 0;
        for (collection_name, index_manager) in index_managers.iter() {
            total_flushed +=
                Self::flush_collection_indexes(collection_name, index_manager, &db_path)?;
        }
        Ok(total_flushed)
    }

    /// Flush all dirty indexes for a single collection.
    ///
    /// Extracted from `flush_all_indexes_counted()` to enable cross-collection
    /// parallelism via rayon. Each collection has its own `Arc<RwLock<IndexManager>>`,
    /// so multiple collections can flush concurrently without lock contention.
    fn flush_collection_indexes(
        collection_name: &str,
        index_manager: &std::sync::Arc<parking_lot::RwLock<crate::index::IndexManager>>,
        db_path: &str,
    ) -> Result<usize> {
        let mut total_flushed = 0;

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

        // 2. Fulltext: batch three-phase flush with parallel serialize
        //    Phase 1: take ALL snapshots in single write lock (~µs each)
        //    Phase 2: serialize ALL in parallel (NO lock, 13s+ each → max(individual))
        //    Phase 3: commit ALL in single write lock (~ms each)
        if !dirty_ft.is_empty() {
            total_flushed += Self::flush_fulltext_batch(collection_name, index_manager, &dirty_ft)?;
        }

        // 3. Fuzzy, btree (sequential — each flush < 300ms, not worth parallelizing)
        for name in &dirty_fz {
            let t = std::time::Instant::now();
            let mut mgr = index_manager.write();
            let lock_wait_ms = t.elapsed().as_millis() as u64;
            let flush_start = std::time::Instant::now();
            if mgr.flush_one_fuzzy_index(name)? {
                let flush_ms = flush_start.elapsed().as_millis() as u64;
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
            let flush_start = std::time::Instant::now();
            if mgr.flush_one_btree_index(name, db_path)? {
                let flush_ms = flush_start.elapsed().as_millis() as u64;
                tracing::info!(
                    collection = %collection_name, index = %name,
                    kind = "btree", lock_wait_ms, flush_ms,
                    "Index flushed"
                );
                total_flushed += 1;
            }
        }

        // 4. HNSW orphan compaction + vector flush
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
            if mgr.flush_one_vector_index(name, db_path)? {
                let flush_ms = t.elapsed().as_millis() as u64 - lock_wait_ms;
                tracing::info!(
                    collection = %collection_name, index = %name,
                    kind = "vector", lock_wait_ms, flush_ms,
                    "Index flushed"
                );
                total_flushed += 1;
            }
        }

        Ok(total_flushed)
    }

    /// Batch three-phase fulltext flush with parallel serialization.
    ///
    /// Instead of sequential Phase1→Phase2→Phase3 per index, this batches:
    /// - Phase 1: Take ALL snapshots in a single write lock (~µs each)
    /// - Phase 2: Serialize ALL snapshots in parallel (NO lock, 13s+ each)
    /// - Phase 3: Commit/rollback ALL results in a single write lock (~ms each)
    ///
    /// With N fulltext indexes, total time drops from N×13s to ~13s (parallel I/O).
    fn flush_fulltext_batch(
        collection_name: &str,
        index_manager: &std::sync::Arc<parking_lot::RwLock<crate::index::IndexManager>>,
        dirty_ft: &[String],
    ) -> Result<usize> {
        let t_batch = std::time::Instant::now();

        // Phase 1: Take all snapshots in a single write lock (~µs each)
        let snapshots: Vec<(String, crate::fulltext::FulltextFlushSnapshot)> = {
            let mut mgr = index_manager.write();
            dirty_ft
                .iter()
                .filter_map(|name| {
                    mgr.take_fulltext_flush_snapshot(name)
                        .map(|s| (name.clone(), s))
                })
                .collect()
        };
        let snapshot_ms = t_batch.elapsed().as_millis() as u64;

        if snapshots.is_empty() {
            return Ok(0);
        }

        // Phase 2: Serialize all snapshots — parallel when >1 index and parallel feature
        let results: Vec<(String, Result<crate::fulltext::FulltextFlushResult>)>;

        #[cfg(feature = "parallel")]
        {
            let cpus = std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1);

            if cpus > 1 && snapshots.len() > 1 {
                use rayon::prelude::*;

                results = snapshots
                    .into_par_iter()
                    .map(|(name, snapshot)| {
                        let result = crate::fulltext::FulltextIndex::serialize_flush(&snapshot);
                        (name, result)
                    })
                    .collect();
            } else {
                results = snapshots
                    .into_iter()
                    .map(|(name, snapshot)| {
                        let result = crate::fulltext::FulltextIndex::serialize_flush(&snapshot);
                        (name, result)
                    })
                    .collect();
            }
        }

        #[cfg(not(feature = "parallel"))]
        {
            results = snapshots
                .into_iter()
                .map(|(name, snapshot)| {
                    let result = crate::fulltext::FulltextIndex::serialize_flush(&snapshot);
                    (name, result)
                })
                .collect();
        }

        let serialize_ms = t_batch.elapsed().as_millis() as u64 - snapshot_ms;

        // Phase 3: Commit successes, rollback failures in a single write lock
        let mut committed = 0usize;
        let mut last_error: Option<crate::error::IronBaseError> = None;
        {
            let mut mgr = index_manager.write();
            for (name, result) in results {
                match result {
                    Ok(r) => match mgr.commit_fulltext_flush(&name, r) {
                        Ok(()) => committed += 1,
                        Err(e) => {
                            tracing::error!(
                                collection = %collection_name, index = %name,
                                error = %e,
                                "Fulltext flush commit failed, rolling back"
                            );
                            mgr.rollback_fulltext_flush(&name);
                            last_error = Some(e);
                        }
                    },
                    Err(e) => {
                        tracing::error!(
                            collection = %collection_name, index = %name,
                            error = %e,
                            "Fulltext flush serialize failed, rolling back"
                        );
                        mgr.rollback_fulltext_flush(&name);
                        if last_error.is_none() {
                            last_error = Some(e);
                        }
                    }
                }
            }
        }
        let commit_ms = t_batch.elapsed().as_millis() as u64 - snapshot_ms - serialize_ms;

        tracing::info!(
            collection = %collection_name,
            count = committed, snapshot_ms, serialize_ms, commit_ms,
            total_ms = t_batch.elapsed().as_millis() as u64,
            "Fulltext indexes flushed (batch three-phase)"
        );

        if let Some(e) = last_error {
            return Err(e);
        }

        Ok(committed)
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
                log_warn!("Failed to flush fulltext indexes on drop: {}", e);
            }
            if let Err(e) = manager.flush_fuzzy_indexes() {
                log_warn!("Failed to flush fuzzy indexes on drop: {}", e);
            }
            if !db_path.is_empty() {
                if let Err(e) = manager.flush_btree_indexes(&db_path) {
                    log_warn!("Failed to flush btree indexes on drop: {}", e);
                }
                if let Err(e) = manager.flush_vector_indexes(&db_path) {
                    log_warn!("Failed to flush vector indexes on drop: {}", e);
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
                log_warn!("Failed to flush storage on drop: {}", e);
            }
            // 4. Mark as clean shutdown for fast restart
            if let Err(e) = storage.mark_clean_shutdown() {
                log_warn!("Failed to mark clean shutdown on drop: {}", e);
            }
        } else {
            log_warn!("Could not acquire storage lock on drop");
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
