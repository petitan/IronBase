// storage/compaction.rs
// Storage compaction functionality

use super::{write_compaction_header, StorageEngine};
use crate::error::{IronBaseError, Result};
use serde_json::Value;
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Maximum memory (in bytes) allowed for document buffering during compaction
/// If exceeded, forces a flush to prevent OOM
const MAX_COMPACTION_MEMORY_BYTES: u64 = 256 * 1024 * 1024; // 256 MB

/// Compaction configuration
#[derive(Debug, Clone, Default)]
pub struct CompactionConfig {
    /// Number of documents to process in memory at once (default: 1000)
    pub chunk_size: usize,
    /// Optional cancellation flag - set to true to abort compaction
    pub cancel_flag: Option<Arc<AtomicBool>>,
    /// Force a full rebuild of every vector (HNSW) index even when there is no
    /// orphan pressure. The automatic bloat-triggered compact leaves this
    /// `false` (orphan-gated rebuild — cheap when nothing needs repair); an
    /// explicit operator-initiated `db_compact` sets it `true` to fully
    /// reconstruct possibly-degraded graphs. Default: `false`.
    pub force_vector_rebuild: bool,
}

impl CompactionConfig {
    /// Create a new compaction config with default values
    pub fn new() -> Self {
        Self {
            chunk_size: 1000,
            cancel_flag: None,
            force_vector_rebuild: false,
        }
    }

    /// Set the chunk size
    pub fn with_chunk_size(mut self, size: usize) -> Self {
        self.chunk_size = size;
        self
    }

    /// Set a cancellation flag for interruptible compaction
    pub fn with_cancel_flag(mut self, flag: Arc<AtomicBool>) -> Self {
        self.cancel_flag = Some(flag);
        self
    }

    /// Force an unconditional rebuild of all vector indexes (see field docs).
    pub fn with_force_vector_rebuild(mut self, force: bool) -> Self {
        self.force_vector_rebuild = force;
        self
    }

    /// Check if cancellation was requested
    pub(crate) fn is_cancelled(&self) -> bool {
        self.cancel_flag
            .as_ref()
            .map(|f| f.load(Ordering::Relaxed))
            .unwrap_or(false)
    }
}

/// Compaction statistics
#[derive(Debug, Clone, Default)]
pub struct CompactionStats {
    pub size_before: u64,
    pub size_after: u64,
    pub documents_scanned: u64,
    pub documents_kept: u64,
    pub tombstones_removed: u64,
    pub peak_memory_mb: u64,
    /// True if compaction was cancelled before completion
    pub cancelled: bool,
}

impl CompactionStats {
    pub fn space_saved(&self) -> u64 {
        self.size_before.saturating_sub(self.size_after)
    }

    pub fn compression_ratio(&self) -> f64 {
        if self.size_before == 0 {
            0.0
        } else {
            (self.size_after as f64 / self.size_before as f64) * 100.0
        }
    }
}

/// Checkpoint statistics
#[derive(Debug, Clone, Default)]
pub struct CheckpointStats {
    /// WAL file size before checkpoint (bytes)
    pub wal_size_before: u64,
    /// WAL file size after checkpoint (always 0 after clear)
    pub wal_size_after: u64,
    /// Number of operations that were in the WAL
    pub wal_ops_cleared: u64,
    /// Number of index files flushed to disk (.idx, .ftidx, .fzidx, .hnsw)
    pub indexes_flushed: usize,
}

/// Storage wastage statistics for auto-compaction decisions
///
/// Computed by iterating collections (O(C) cost, not O(N) documents).
/// Uses file_size vs estimated_live_bytes to determine bloat ratio.
#[derive(Debug, Clone)]
pub struct StorageWastage {
    /// Current .mlite file size on disk
    pub file_size_bytes: u64,
    /// Estimated live data size (last compact size_after, or 0 if never compacted)
    pub estimated_live_bytes: u64,
    /// file_size / estimated_live (1.0 = no bloat, f64::INFINITY if never compacted)
    pub bloat_ratio: f64,
    /// Total document writes (including tombstones and old versions)
    pub total_writes: u64,
    /// Total live documents
    pub total_live: u64,
    /// Dead writes (total_writes - total_live)
    pub dead_writes: u64,
}

// =========================================================================
// NON-BLOCKING COMPACTION: Lock-splitting types
// =========================================================================

/// Phase A result — snapshot of collections and temp file for standalone scan
pub struct CompactionSnapshot {
    /// Temporary file to write compacted data to
    pub temp_file: std::fs::File,
    /// Path to the temp file
    pub temp_path: String,
    /// Snapshot of collection metadata at the time of prepare
    /// Arc-wrapped to avoid expensive clone when sharing between Phase B scan
    /// and Phase C catch-up reconciliation
    pub snapshot_collections: Arc<HashMap<String, super::CollectionMeta>>,
    /// data_end_offset at snapshot time — data below this is IMMUTABLE
    pub snapshot_data_end_offset: u64,
    /// Document region boundary for offset validation
    pub file_len: u64,
    /// Path to the source database file (for pread)
    pub source_path: String,
    /// Header snapshot for metadata writing
    pub header: super::Header,
    /// File size before compaction (for stats)
    pub size_before: u64,
}

/// Phase B result — completed scan, ready for catch-up reconciliation
pub struct CompactionScanResult {
    /// Temp file with compacted documents written
    pub temp_file: std::fs::File,
    /// Path to the temp file
    pub temp_path: String,
    /// New collection metadata (document_catalog pointing into temp file)
    pub new_collections: HashMap<String, super::CollectionMeta>,
    /// Current write offset in the temp file
    pub write_offset: u64,
    /// Compaction statistics so far
    pub stats: CompactionStats,
    /// Header snapshot
    pub header: super::Header,
}

impl StorageEngine {
    /// Storage compaction - removes tombstones and old document versions
    /// Uses chunked processing to minimize memory usage
    pub fn compact(&mut self) -> Result<CompactionStats> {
        self.compact_with_config(&CompactionConfig::default())
    }

    /// Storage compaction with custom configuration
    ///
    /// Removes tombstones and old document versions using chunked processing.
    /// Internally uses the 3-phase lock-splitting approach:
    /// 1. compact_prepare() - Setup temp file and snapshot
    /// 2. compact_scan_standalone() - Iterate catalogs and write documents (pread, no lock needed)
    /// 3. compact_finalize_with_catchup() - Catch-up reconciliation + atomic swap
    ///
    /// When called on &mut self (holding storage.write()), all 3 phases run sequentially.
    /// For non-blocking compact, use compact_prepare/scan_standalone/finalize separately.
    pub fn compact_with_config(&mut self, config: &CompactionConfig) -> Result<CompactionStats> {
        let snapshot = self.compact_prepare()?;
        let snapshot_collections = Arc::clone(&snapshot.snapshot_collections);
        let scan_result = compact_scan_standalone(snapshot, config, &|_, _| {})?;
        self.compact_finalize_with_catchup(scan_result, &snapshot_collections)
    }

    // =========================================================================
    // PHASE A: Prepare (brief &mut self)
    // =========================================================================

    /// Phase A: Prepare compaction snapshot
    ///
    /// Flushes metadata, creates temp file, snapshots collections.
    /// This is a brief &mut self operation (~ms).
    ///
    /// Returns CompactionSnapshot for use by compact_scan_standalone().
    pub fn compact_prepare(&mut self) -> Result<CompactionSnapshot> {
        // CRITICAL: Flush metadata first to ensure header.metadata_offset is up-to-date!
        self.flush_metadata()?;

        let temp_path = format!("{}.compact", self.file_path);

        // For version 2+: only scan up to metadata_offset (don't read metadata as documents!)
        let file_len = if self.header.version >= 2 && self.header.metadata_offset > 0 {
            self.header.metadata_offset
        } else {
            self.file_len()?
        };

        // Create temporary new file
        let mut new_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&temp_path)?;

        // Write header only (no metadata yet - documents start at HEADER_SIZE)
        new_file.seek(SeekFrom::Start(0))?;
        let header_bytes = bincode::serialize(&self.header)
            .map_err(|e| IronBaseError::Serialization(e.to_string()))?;
        new_file.write_all(&header_bytes)?;

        // Position at start of document region
        new_file.seek(SeekFrom::Start(super::HEADER_SIZE))?;

        let size_before = self.file.metadata()?.len();
        let snapshot_data_end_offset = self.header.data_end_offset;

        // Snapshot collections — O(1) Arc::clone (copy-on-write). The catalog is
        // NOT deep-cloned here under the write lock (audit P0-1); the first
        // concurrent write during Phase B pays a single `Arc::make_mut` clone,
        // and an idle/read-only compaction pays nothing.
        let snapshot_collections = Arc::clone(&self.collections);

        Ok(CompactionSnapshot {
            temp_file: new_file,
            temp_path,
            snapshot_collections,
            snapshot_data_end_offset,
            file_len,
            source_path: self.file_path.clone(),
            header: self.header.clone(),
            size_before,
        })
    }

    // =========================================================================
    // PHASE C: Finalize with catch-up reconciliation (brief &mut self)
    // =========================================================================

    /// Phase C: Catch-up reconciliation + finalize
    ///
    /// Compares the snapshot collections with the current collections to find
    /// mutations (insert/update/delete) that happened during Phase B.
    /// Applies those mutations to the temp file, then performs atomic swap.
    ///
    /// This is a brief &mut self operation — the catch-up is O(mutations),
    /// not O(total_docs).
    pub fn compact_finalize_with_catchup(
        &mut self,
        mut scan_result: CompactionScanResult,
        snapshot_collections: &HashMap<String, super::CollectionMeta>,
    ) -> Result<CompactionStats> {
        // RAII guard: removes .compact temp file on error (any early ? return)
        // Disarmed on success before finalize_compaction() renames it.
        let mut temp_cleanup = TempFileCleanup::new(scan_result.temp_path.clone());

        let mut stats = scan_result.stats;

        // Phase C Step 1: Catch-up reconciliation
        // Diff snapshot vs current collections to find mutations during Phase B
        //
        // Collect mutations first, then apply them — avoids borrow conflict
        // between iterating &self.collections and calling self.read_data_at()

        // Collect all mutations into a Vec to avoid borrow conflicts
        enum CatchupOp {
            InsertOrUpdate {
                coll_name: String,
                doc_id: crate::document::DocumentId,
                offset: u64,
                is_update: bool,
            },
            Delete {
                coll_name: String,
                doc_id: crate::document::DocumentId,
            },
            NewCollection {
                coll_name: String,
                meta: Box<super::CollectionMeta>,
                docs: Vec<(crate::document::DocumentId, u64)>,
            },
            DroppedCollection {
                coll_name: String,
            },
        }

        let mut ops: Vec<CatchupOp> = Vec::new();

        for (coll_name, current_meta) in self.collections.iter() {
            let snap_meta = snapshot_collections.get(coll_name);

            match snap_meta {
                Some(snap) => {
                    // Find new inserts and updates
                    for (doc_id, &current_offset) in &current_meta.document_catalog {
                        match snap.document_catalog.get(doc_id) {
                            None => {
                                ops.push(CatchupOp::InsertOrUpdate {
                                    coll_name: coll_name.clone(),
                                    doc_id: doc_id.clone(),
                                    offset: current_offset,
                                    is_update: false,
                                });
                            }
                            Some(&snap_offset) if snap_offset != current_offset => {
                                ops.push(CatchupOp::InsertOrUpdate {
                                    coll_name: coll_name.clone(),
                                    doc_id: doc_id.clone(),
                                    offset: current_offset,
                                    is_update: true,
                                });
                            }
                            _ => {}
                        }
                    }

                    // Find deletes
                    for doc_id in snap.document_catalog.keys() {
                        if !current_meta.document_catalog.contains_key(doc_id) {
                            ops.push(CatchupOp::Delete {
                                coll_name: coll_name.clone(),
                                doc_id: doc_id.clone(),
                            });
                        }
                    }
                }
                None => {
                    // New collection during Phase B
                    let docs: Vec<_> = current_meta
                        .document_catalog
                        .iter()
                        .map(|(id, &off)| (id.clone(), off))
                        .collect();
                    ops.push(CatchupOp::NewCollection {
                        coll_name: coll_name.clone(),
                        meta: Box::new(current_meta.clone()),
                        docs,
                    });
                }
            }
        }

        // Dropped collections
        for name in snapshot_collections.keys() {
            if !self.collections.contains_key(name) {
                ops.push(CatchupOp::DroppedCollection {
                    coll_name: name.clone(),
                });
            }
        }

        // Apply all mutations (now we can borrow self mutably for read_data_at)
        let mut catchup_count: u64 = 0;

        for op in ops {
            match op {
                CatchupOp::InsertOrUpdate {
                    coll_name,
                    doc_id,
                    offset,
                    is_update,
                } => {
                    // read_data_at uses pread — &self, no seek
                    match self.read_data_at(offset) {
                        Ok(doc_bytes) => {
                            if is_update {
                                if let Some(coll_meta) =
                                    scan_result.new_collections.get_mut(&coll_name)
                                {
                                    coll_meta.document_catalog.remove(&doc_id);
                                }
                            }
                            scan_result.write_offset = write_doc_to_temp(
                                &mut scan_result.temp_file,
                                &mut scan_result.new_collections,
                                &coll_name,
                                &doc_id,
                                &doc_bytes,
                                scan_result.write_offset,
                            )?;
                            catchup_count += 1;
                        }
                        Err(e) => {
                            crate::log_warn!("Compact catch-up: skip doc {:?}: {}", doc_id, e);
                        }
                    }
                }
                CatchupOp::Delete { coll_name, doc_id } => {
                    if let Some(coll_meta) = scan_result.new_collections.get_mut(&coll_name) {
                        coll_meta.document_catalog.remove(&doc_id);
                        coll_meta.document_order.retain(|id| id != &doc_id);
                        if coll_meta.live_document_count > 0 {
                            coll_meta.live_document_count -= 1;
                        }
                        if coll_meta.document_count > 0 {
                            coll_meta.document_count -= 1;
                        }
                    }
                    catchup_count += 1;
                }
                CatchupOp::NewCollection {
                    coll_name,
                    meta,
                    docs,
                } => {
                    let mut new_coll_meta = *meta;
                    new_coll_meta.data_offset = super::HEADER_SIZE;
                    new_coll_meta.document_catalog.clear();
                    new_coll_meta.document_count = 0;
                    new_coll_meta.live_document_count = 0;
                    scan_result
                        .new_collections
                        .insert(coll_name.clone(), new_coll_meta);

                    for (doc_id, offset) in docs {
                        match self.read_data_at(offset) {
                            Ok(doc_bytes) => {
                                scan_result.write_offset = write_doc_to_temp(
                                    &mut scan_result.temp_file,
                                    &mut scan_result.new_collections,
                                    &coll_name,
                                    &doc_id,
                                    &doc_bytes,
                                    scan_result.write_offset,
                                )?;
                                catchup_count += 1;
                            }
                            Err(e) => {
                                crate::log_warn!(
                                    "Compact catch-up: skip doc {:?} in new collection: {}",
                                    doc_id,
                                    e
                                );
                            }
                        }
                    }
                }
                CatchupOp::DroppedCollection { coll_name } => {
                    scan_result.new_collections.remove(&coll_name);
                    catchup_count += 1;
                }
            }
        }

        // Phase C Step 2: Update document_order to match current state
        for (coll_name, coll_meta) in scan_result.new_collections.iter_mut() {
            if let Some(current_meta) = self.collections.get(coll_name) {
                coll_meta.document_order = current_meta
                    .document_order
                    .iter()
                    .filter(|id| coll_meta.document_catalog.contains_key(id))
                    .cloned()
                    .collect();
            }
        }

        if catchup_count > 0 {
            crate::log_info!("Compact catch-up: {} mutations reconciled", catchup_count);
        }

        // Phase C Step 3: Write metadata + finalize
        Self::write_compacted_metadata(
            &mut scan_result.temp_file,
            &scan_result.header,
            &scan_result.new_collections,
            scan_result.write_offset,
        )?;

        stats.size_after = scan_result.temp_file.metadata()?.len();

        // Disarm cleanup guard — finalize_compaction will rename (not delete) the temp file
        temp_cleanup.disarm();

        // Atomic file swap + reload
        self.finalize_compaction(&scan_result.temp_path, scan_result.temp_file)?;

        Ok(stats)
    }

    // =========================================================================
    // INTERNAL HELPERS (used by both blocking and non-blocking paths)
    // =========================================================================

    /// Write metadata at end of compacted file and update header
    fn write_compacted_metadata(
        new_file: &mut std::fs::File,
        header: &super::Header,
        new_collections: &HashMap<String, super::CollectionMeta>,
        metadata_offset: u64,
    ) -> Result<()> {
        // Serialize metadata body
        let mut metadata_buffer = std::io::Cursor::new(Vec::new());

        // Write collection count
        let count = (new_collections.len() as u32).to_le_bytes();
        metadata_buffer.write_all(&count)?;

        // Write each collection metadata
        for meta in new_collections.values() {
            let meta_bytes = serde_json::to_vec(meta)?;
            let len = (meta_bytes.len() as u32).to_le_bytes();
            metadata_buffer.write_all(&len)?;
            metadata_buffer.write_all(&meta_bytes)?;
        }

        let metadata_bytes = metadata_buffer.into_inner();
        let metadata_size = metadata_bytes.len() as u64;

        // Write metadata at end
        new_file.seek(SeekFrom::Start(metadata_offset))?;
        new_file.write_all(&metadata_bytes)?;

        // Use write_compaction_header() to safely update header with correct offsets
        write_compaction_header(new_file, header, metadata_offset, metadata_size)?;

        new_file.sync_all()?;

        Ok(())
    }

    /// Finalize compaction: close files, rename temp to original, reload metadata
    fn finalize_compaction(&mut self, temp_path: &str, new_file: std::fs::File) -> Result<()> {
        // Close new file before renaming
        drop(new_file);

        // Atomic rename + parent directory fsync for durability on POSIX
        crate::fs_utils::atomic_rename_and_sync(temp_path, &self.file_path)?;

        // Reopen the compacted file
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&self.file_path)?;

        // Reload metadata
        let (header, collections) = Self::load_metadata(&mut file)?;

        // Update self
        self.file = file;
        self.header = header;
        self.collections = Arc::new(collections);

        Ok(())
    }
}

// =========================================================================
// RAII temp file cleanup guard
// =========================================================================

/// RAII guard that removes a temp file on drop unless disarmed.
///
/// Used by compact_finalize_with_catchup() to ensure the .compact temp file
/// is cleaned up on any error path (early ? return). On success, the guard
/// is disarmed before finalize_compaction() renames the temp file.
struct TempFileCleanup {
    path: String,
    armed: bool,
}

impl TempFileCleanup {
    fn new(path: String) -> Self {
        Self { path, armed: true }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for TempFileCleanup {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_file(&self.path);
        }
    }
}

// =========================================================================
// PHASE B: Standalone scan (NO StorageEngine, NO lock)
// =========================================================================

/// Phase B: Scan and copy documents from source to temp file
///
/// This function does NOT require a StorageEngine reference — it opens
/// its own read-only file handle and uses pread() for thread-safe reads.
/// This allows it to run WITHOUT holding the storage write lock.
///
/// Progress is reported via the callback: (documents_processed, total_documents)
pub fn compact_scan_standalone(
    snapshot: CompactionSnapshot,
    config: &CompactionConfig,
    progress_callback: &dyn Fn(u64, u64),
) -> Result<CompactionScanResult> {
    let mut stats = CompactionStats {
        size_before: snapshot.size_before,
        ..Default::default()
    };

    let mut temp_file = snapshot.temp_file;
    let mut write_offset = super::HEADER_SIZE;

    // Prepare new collections metadata (deep clone the inner HashMap, reset catalogs)
    let mut new_collections: HashMap<String, super::CollectionMeta> =
        (*snapshot.snapshot_collections).clone();
    for coll_meta in new_collections.values_mut() {
        coll_meta.data_offset = super::HEADER_SIZE;
        coll_meta.document_catalog.clear();
        coll_meta.document_count = 0;
        coll_meta.live_document_count = 0;
    }

    // Open a separate read-only handle for pread()
    let source_file = std::fs::File::open(&snapshot.source_path)?;
    // PR-2 / Fix B: we read the source strictly in catalog order — hint sequential
    // so readahead stays effective even as we drop consumed pages below.
    super::io::advise_sequential(&source_file);

    // Count total documents for progress reporting
    let total_docs: u64 = snapshot
        .snapshot_collections
        .values()
        .map(|m| m.document_catalog.len() as u64)
        .sum();

    // Track documents per collection
    let mut collection_docs: HashMap<String, HashMap<crate::document::DocumentId, Value>> =
        HashMap::new();
    for coll_name in snapshot.snapshot_collections.keys() {
        collection_docs.insert(coll_name.clone(), HashMap::new());
    }

    let mut chunk_count = 0;
    let mut total_memory_bytes: u64 = 0;
    let mut docs_processed: u64 = 0;
    // PR-2 / Fix B: high-water mark of the source prefix already dropped from the
    // page cache. Advances monotonically so each region is dropped at most once
    // (O(file_size) total, not O(chunks·file_size)) and is robust to the
    // non-monotonic offset order across collections (snapshot is a HashMap).
    let mut src_dropped_upto: u64 = 0;

    for (coll_name, coll_meta) in snapshot.snapshot_collections.iter() {
        // Check for cancellation at collection boundary
        if config.is_cancelled() {
            let _ = fs::remove_file(&snapshot.temp_path);
            return Err(IronBaseError::Cancelled(
                "Compaction cancelled by user".to_string(),
            ));
        }

        // Iterate using document_order to preserve insertion order
        for doc_id in &coll_meta.document_order {
            let offset = match coll_meta.document_catalog.get(doc_id) {
                Some(&off) => off,
                None => continue,
            };

            // Check for cancellation periodically (every chunk)
            if chunk_count > 0 && chunk_count % config.chunk_size == 0 && config.is_cancelled() {
                let _ = fs::remove_file(&snapshot.temp_path);
                return Err(IronBaseError::Cancelled(
                    "Compaction cancelled by user".to_string(),
                ));
            }

            // Validate offset is before metadata
            if offset >= snapshot.file_len {
                crate::log_warn!(
                    "Skipping document {:?} at invalid offset {} (file_len: {})",
                    doc_id,
                    offset,
                    snapshot.file_len
                );
                stats.tombstones_removed += 1;
                docs_processed += 1;
                progress_callback(docs_processed, total_docs);
                continue;
            }

            // Read document using positioned I/O (no seek, thread-safe)
            match super::io::read_document_from_file(
                &source_file,
                offset,
                snapshot.snapshot_data_end_offset,
            ) {
                Ok(doc_bytes) => {
                    stats.documents_scanned += 1;

                    if let Ok(doc) = serde_json::from_slice::<Value>(&doc_bytes) {
                        if let Some(docs_by_id) = collection_docs.get_mut(coll_name.as_str()) {
                            // serde_json::Value heap size is ~2-4x the raw JSON bytes:
                            // String keys/values are heap-allocated, Map uses BTreeMap,
                            // Vec/Number/Bool have per-variant overhead
                            let doc_size_bytes = doc_bytes.len() as u64;
                            total_memory_bytes += doc_size_bytes * 3 + 128;

                            let current_memory_mb = total_memory_bytes / (1024 * 1024);
                            if current_memory_mb > stats.peak_memory_mb {
                                stats.peak_memory_mb = current_memory_mb;
                            }

                            docs_by_id.insert(doc_id.clone(), doc);
                            chunk_count += 1;

                            let should_flush = chunk_count >= config.chunk_size
                                || total_memory_bytes >= MAX_COMPACTION_MEMORY_BYTES;

                            if should_flush {
                                let flush_start = write_offset;
                                for (flush_coll_name, docs) in collection_docs.iter_mut() {
                                    if !docs.is_empty() {
                                        write_offset = flush_compaction_chunk_standalone(
                                            &mut temp_file,
                                            &mut new_collections,
                                            flush_coll_name,
                                            docs,
                                            write_offset,
                                            &mut stats,
                                        )?;
                                        docs.clear();
                                    }
                                }
                                // PR-2 / Fix B: keep the page-cache footprint ~chunk_size,
                                // not ~file_size. Write back + drop the just-written target
                                // range, then drop ONLY the newly-consumed source delta
                                // [src_dropped_upto, offset) — never re-dropping an already-
                                // evicted prefix, and never the in-progress doc's pages.
                                // Readahead ahead of `offset` is preserved. Advisory; data
                                // is unaffected.
                                super::io::flush_range_to_disk(
                                    &temp_file,
                                    flush_start,
                                    write_offset - flush_start,
                                );
                                super::io::advise_dontneed(
                                    &temp_file,
                                    flush_start,
                                    write_offset - flush_start,
                                );
                                if offset > src_dropped_upto {
                                    super::io::advise_dontneed(
                                        &source_file,
                                        src_dropped_upto,
                                        offset - src_dropped_upto,
                                    );
                                    src_dropped_upto = offset;
                                }
                                chunk_count = 0;
                                total_memory_bytes = 0;
                            }
                        }
                    }
                }
                Err(e) => {
                    crate::log_warn!(
                        "Skipping corrupt document {:?} at offset {}: {}",
                        doc_id,
                        offset,
                        e
                    );
                    stats.tombstones_removed += 1;
                }
            }

            docs_processed += 1;
            // Report progress every 1000 docs to avoid callback overhead
            if docs_processed % 1000 == 0 || docs_processed == total_docs {
                progress_callback(docs_processed, total_docs);
            }
        }
    }

    // Flush remaining documents
    let final_flush_start = write_offset;
    for (coll_name, docs) in collection_docs.iter_mut() {
        if !docs.is_empty() {
            write_offset = flush_compaction_chunk_standalone(
                &mut temp_file,
                &mut new_collections,
                coll_name,
                docs,
                write_offset,
                &mut stats,
            )?;
        }
    }
    // PR-2 / Fix B: write back + drop the final chunk's target pages too.
    super::io::flush_range_to_disk(
        &temp_file,
        final_flush_start,
        write_offset - final_flush_start,
    );
    super::io::advise_dontneed(
        &temp_file,
        final_flush_start,
        write_offset - final_flush_start,
    );

    temp_file.sync_all()?;

    // Final progress report
    progress_callback(docs_processed, total_docs);

    Ok(CompactionScanResult {
        temp_file,
        temp_path: snapshot.temp_path,
        new_collections,
        write_offset,
        stats,
        header: snapshot.header,
    })
}

// =========================================================================
// FREE FUNCTIONS (no &self needed)
// =========================================================================

/// Flush a chunk of documents to the compacted file (standalone, no &self)
fn flush_compaction_chunk_standalone(
    new_file: &mut std::fs::File,
    new_collections: &mut HashMap<String, super::CollectionMeta>,
    coll_name: &str,
    docs_by_id: &mut HashMap<crate::document::DocumentId, Value>,
    mut write_offset: u64,
    stats: &mut CompactionStats,
) -> Result<u64> {
    for (doc_id, doc) in docs_by_id.iter() {
        // Skip tombstones (deleted documents)
        if doc
            .get("_tombstone")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            stats.tombstones_removed += 1;
            continue;
        }

        // Write document to new file
        let doc_offset = write_offset;
        let doc_bytes = serde_json::to_vec(doc)?;
        let len = doc_bytes.len() as u32;

        new_file.write_all(&len.to_le_bytes())?;
        new_file.write_all(&doc_bytes)?;

        write_offset += 4 + doc_bytes.len() as u64;
        stats.documents_kept += 1;

        // Update document_catalog and document_count
        if let Some(coll_meta) = new_collections.get_mut(coll_name) {
            coll_meta
                .document_catalog
                .insert(doc_id.clone(), doc_offset);
            coll_meta.document_count += 1;
            coll_meta.live_document_count = coll_meta.live_document_count.saturating_add(1);
        }
    }

    Ok(write_offset)
}

/// Write a single document to the temp file and update its catalog entry
///
/// Used by catch-up reconciliation (Phase C) for new inserts and updates.
fn write_doc_to_temp(
    temp_file: &mut std::fs::File,
    new_collections: &mut HashMap<String, super::CollectionMeta>,
    coll_name: &str,
    doc_id: &crate::document::DocumentId,
    doc_bytes: &[u8],
    write_offset: u64,
) -> Result<u64> {
    // Seek to write position
    temp_file.seek(SeekFrom::Start(write_offset))?;

    let len = doc_bytes.len() as u32;
    temp_file.write_all(&len.to_le_bytes())?;
    temp_file.write_all(doc_bytes)?;

    let new_offset = write_offset + 4 + doc_bytes.len() as u64;

    if let Some(coll_meta) = new_collections.get_mut(coll_name) {
        let is_new = !coll_meta.document_catalog.contains_key(doc_id);
        coll_meta
            .document_catalog
            .insert(doc_id.clone(), write_offset);
        if is_new {
            coll_meta.document_count += 1;
            coll_meta.live_document_count = coll_meta.live_document_count.saturating_add(1);
            coll_meta.document_order.push(doc_id.clone());
        }
    }

    Ok(new_offset)
}
