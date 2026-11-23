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

/// Free space a compaction keeps on the volume, so that the database itself
/// can keep writing (and the WAL growing) while the compacted copy is built
/// next to it: a tenth of the data, at least 1 MB, at most 64 MB (a fixed
/// 64 MB would stop small databases on small volumes from ever compacting).
pub fn compaction_free_space_reserve(data_bytes: u64) -> u64 {
    const MB: u64 = 1024 * 1024;
    (data_bytes / 10).clamp(MB, 64 * MB)
}

/// The path of the compacted copy of the database at `db_path`
pub(crate) fn compaction_temp_path(db_path: &str) -> String {
    format!("{}.compact", db_path)
}

/// Free bytes on the volume of `path` (the database file or its temp copy)
fn available_space(path: &str) -> std::io::Result<u64> {
    #[cfg(test)]
    if let Some(bytes) = test_space::next() {
        return Ok(bytes);
    }
    let dir = std::path::Path::new(path)
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."));
    fs2::available_space(dir)
}

/// The free-space limit one compaction works under: every write of the
/// compacted copy (documents, catch-up, metadata) is checked first, so the
/// compaction stops before the disk fills instead of after.
#[derive(Debug, Clone)]
pub(crate) struct SpaceCheck {
    path: String,
    reserve: u64,
    /// False once the free space could not be read (an unsupported file
    /// system): said once, then not checked (it must not block compaction)
    enabled: bool,
}

impl SpaceCheck {
    /// The limit for the database at `db_path` holding `data_bytes` of data
    pub(crate) fn new(db_path: &str, data_bytes: u64) -> Self {
        Self {
            path: db_path.to_string(),
            reserve: compaction_free_space_reserve(data_bytes),
            enabled: true,
        }
    }

    /// `Ok` when `needed` bytes can be written and the reserve still stays
    /// free; else an `Io` error of kind `StorageFull` saying what is free and
    /// what is needed (`stage`: where the compaction stopped)
    pub(crate) fn ensure(&mut self, needed: u64, stage: &str) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }
        let available = match available_space(&self.path) {
            Ok(bytes) => bytes,
            Err(e) => {
                crate::log_warn!("Compaction: free disk space unknown ({}), not checked", e);
                self.enabled = false;
                return Ok(());
            }
        };
        if available >= needed.saturating_add(self.reserve) {
            return Ok(());
        }
        const MB: u64 = 1024 * 1024;
        Err(IronBaseError::Io(std::io::Error::new(
            std::io::ErrorKind::StorageFull,
            format!(
                "not enough disk space for compaction ({stage}): {} MB is free, {} MB is \
                 needed for the next write and {} MB must stay free for the database. \
                 The compaction stopped and its partial copy was removed; the database is \
                 unchanged. Free up space (the compacted copy needs about the size of the \
                 live data) or move the database to a larger volume.",
                available / MB,
                needed.div_ceil(MB),
                self.reserve.div_ceil(MB),
            ),
        )))
    }
}

/// Before a compaction (under its guard, so no other compaction owns the
/// file): remove a copy left by an interrupted one (a crash, or a version
/// that left it on a full disk) — it holds space the new copy needs — and
/// check that the reserve is free. Done before the costly index work, so a
/// full disk is reported at once.
pub(crate) fn compaction_precheck(db_path: &str) -> Result<()> {
    let temp_path = compaction_temp_path(db_path);
    match fs::remove_file(&temp_path) {
        Ok(()) => crate::log_warn!("Compaction: removed a stale partial copy {}", temp_path),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    let data_bytes = fs::metadata(db_path).map(|m| m.len()).unwrap_or(0);
    SpaceCheck::new(db_path, data_bytes).ensure(0, "before starting")
}

/// Test hook: the free space `available_space` reports, one value per call
/// (the last one repeats), for this thread only
#[cfg(test)]
pub(crate) mod test_space {
    use std::cell::RefCell;

    thread_local! {
        static QUEUE: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
    }

    /// Report these values, in order, then the last one forever
    pub(crate) fn set(values: &[u64]) {
        QUEUE.with(|q| *q.borrow_mut() = values.iter().rev().copied().collect());
    }

    pub(crate) fn clear() {
        QUEUE.with(|q| q.borrow_mut().clear());
    }

    pub(super) fn next() -> Option<u64> {
        QUEUE.with(|q| {
            let mut q = q.borrow_mut();
            match q.len() {
                0 => None,
                1 => q.last().copied(),
                _ => q.pop(),
            }
        })
    }
}

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
        // Before anything is written (the metadata flush below writes too):
        // remove a stale copy, check that the reserve is free
        compaction_precheck(&self.file_path)?;

        // CRITICAL: Flush metadata first to ensure header.metadata_offset is up-to-date!
        self.flush_metadata()?;

        let temp_path = compaction_temp_path(&self.file_path);

        // For version 2+: only scan up to metadata_offset (don't read metadata as documents!)
        let file_len = if self.header.version >= 2 && self.header.metadata_offset > 0 {
            self.header.metadata_offset
        } else {
            self.file_len()?
        };

        // Create the temporary new file; removed again if the setup below
        // fails. The guard is declared first so that it is dropped last,
        // after the file is closed (Windows cannot remove an open file).
        let mut temp_cleanup = TempFileCleanup::new(temp_path.clone());
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

        // Phase B creates its own guard for the file
        temp_cleanup.disarm();
        drop(temp_cleanup);
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
        scan_result: CompactionScanResult,
        snapshot_collections: &HashMap<String, super::CollectionMeta>,
    ) -> Result<CompactionStats> {
        // Any early return below removes the temp file; disarmed only once
        // it has replaced the database. The guard is declared before the
        // local that owns the file, so the file is closed before removal
        // (Windows cannot remove an open file).
        let mut temp_cleanup = TempFileCleanup::new(scan_result.temp_path.clone());
        let mut scan_result = scan_result;
        let mut space = SpaceCheck::new(&self.file_path, scan_result.stats.size_before);

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
                                });
                            }
                            Some(&snap_offset) if snap_offset != current_offset => {
                                ops.push(CatchupOp::InsertOrUpdate {
                                    coll_name: coll_name.clone(),
                                    doc_id: doc_id.clone(),
                                    offset: current_offset,
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
                } => {
                    // read_data_at uses pread — &self, no seek
                    // An updated doc keeps its catalog entry: write_doc_to_temp
                    // just repoints it, so the counters are not bumped twice
                    // (audit 2026-10-06 #4).
                    match self.read_data_at_raw(offset) {
                        Ok(doc_bytes) => {
                            scan_result.write_offset = write_doc_to_temp(
                                &mut scan_result.temp_file,
                                &mut scan_result.new_collections,
                                &coll_name,
                                &doc_id,
                                &doc_bytes,
                                scan_result.write_offset,
                                &mut space,
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
                        match self.read_data_at_raw(offset) {
                            Ok(doc_bytes) => {
                                scan_result.write_offset = write_doc_to_temp(
                                    &mut scan_result.temp_file,
                                    &mut scan_result.new_collections,
                                    &coll_name,
                                    &doc_id,
                                    &doc_bytes,
                                    scan_result.write_offset,
                                    &mut space,
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

        // Phase C Step 2: Take everything except the document layout from the
        // current metadata. new_collections started as a clone of the Phase-A
        // snapshot, so last_id, index/schema/flag and auto-embed config changes
        // made during Phase B would otherwise revert on the swap (audit
        // 2026-10-06 #2: a reverted last_id makes later inserts reuse _ids and
        // overwrite documents). Only the catalog, document_order, counters and
        // data_offset describe the compacted file. The destructuring is
        // exhaustive so a new CollectionMeta field fails to compile here
        // instead of silently reverting.
        for (coll_name, coll_meta) in scan_result.new_collections.iter_mut() {
            if let Some(current_meta) = self.collections.get(coll_name) {
                let super::CollectionMeta {
                    name: _,
                    document_count: _,
                    live_document_count: _,
                    data_offset: _,
                    document_catalog: _,
                    index_offset,
                    last_id,
                    document_order,
                    indexes,
                    fuzzy_indexes,
                    fulltext_indexes,
                    vector_indexes,
                    schema,
                    flags,
                    auto_embedding_config,
                } = current_meta;
                coll_meta.document_order = document_order
                    .iter()
                    .filter(|id| coll_meta.document_catalog.contains_key(id))
                    .cloned()
                    .collect();
                coll_meta.index_offset = *index_offset;
                coll_meta.last_id = *last_id;
                coll_meta.indexes = indexes.clone();
                coll_meta.fuzzy_indexes = fuzzy_indexes.clone();
                coll_meta.fulltext_indexes = fulltext_indexes.clone();
                coll_meta.vector_indexes = vector_indexes.clone();
                coll_meta.schema = schema.clone();
                coll_meta.flags = *flags;
                coll_meta.auto_embedding_config = auto_embedding_config.clone();
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
            &mut space,
        )?;

        stats.size_after = scan_result.temp_file.metadata()?.len();

        // Atomic file swap + reload; the temp file is the database now
        // (on a failed rename the guard still removes it)
        self.finalize_compaction(
            &scan_result.temp_path,
            scan_result.temp_file,
            &mut temp_cleanup,
        )?;

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
        space: &mut SpaceCheck,
    ) -> Result<()> {
        // Serialize metadata body
        let mut metadata_buffer = std::io::Cursor::new(Vec::new());

        // Write collection count
        let count = (new_collections.len() as u32).to_le_bytes();
        metadata_buffer.write_all(&count)?;

        // Write each collection metadata
        for meta in new_collections.values() {
            let meta_bytes = serde_json::to_vec(meta)?;
            metadata_buffer.write_all(&Self::metadata_len_prefix(meta, &meta_bytes)?)?;
            metadata_buffer.write_all(&meta_bytes)?;
        }

        let metadata_bytes = metadata_buffer.into_inner();
        let metadata_size = metadata_bytes.len() as u64;

        // Write metadata at end (for millions of documents the catalogs are
        // hundreds of MB: checked like the documents)
        space.ensure(metadata_size, "writing the metadata")?;
        new_file.seek(SeekFrom::Start(metadata_offset))?;
        new_file.write_all(&metadata_bytes)?;

        // Use write_compaction_header() to safely update header with correct offsets
        write_compaction_header(new_file, header, metadata_offset, metadata_size)?;

        new_file.sync_all()?;

        Ok(())
    }

    /// Finalize compaction: close files, rename temp to original, reload metadata
    fn finalize_compaction(
        &mut self,
        temp_path: &str,
        new_file: std::fs::File,
        cleanup: &mut TempFileCleanup,
    ) -> Result<()> {
        // Close new file before renaming
        drop(new_file);

        // Atomic rename + parent directory fsync for durability on POSIX
        crate::fs_utils::atomic_rename_and_sync(temp_path, &self.file_path)?;
        // The temp path no longer exists: nothing to remove from here on
        cleanup.disarm();

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
        self.layout_generation += 1;

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
#[derive(Debug)]
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
            match fs::remove_file(&self.path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                // Said, so that the operator can remove the copy: it holds
                // about the live data's size on the database's volume
                Err(e) => crate::log_warn!(
                    "Compaction: could not remove the partial copy {}: {}",
                    self.path,
                    e
                ),
            }
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

    // Every early return below (a write error such as a full disk, the
    // free-space check, cancellation) drops this guard, which removes the
    // partial copy: left behind, it would keep the disk full and the database
    // unable to write. Declared before the file, so it is dropped after it.
    let mut temp_cleanup = TempFileCleanup::new(snapshot.temp_path.clone());
    let mut space = SpaceCheck::new(&snapshot.source_path, snapshot.size_before);
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
                                            &mut space,
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
                &mut space,
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
    // PR-2 / Fix B (K-1 follow-up): drop the SOURCE tail too. The in-loop drop
    // (above) deliberately stops at the in-progress doc's start (`offset`,
    // exclusive), so the last chunk's source pages are never reached by it.
    // Every document has now been read and flushed, so the whole remaining
    // source prefix up to the read boundary is safe to evict — otherwise it
    // lingers in the page cache until the handle is dropped, defeating the
    // ~chunk_size footprint goal for the final chunk. Advisory; data unaffected.
    if snapshot.snapshot_data_end_offset > src_dropped_upto {
        super::io::advise_dontneed(
            &source_file,
            src_dropped_upto,
            snapshot.snapshot_data_end_offset - src_dropped_upto,
        );
    }

    temp_file.sync_all()?;

    // Final progress report
    progress_callback(docs_processed, total_docs);

    // Phase C takes over the copy (and guards it itself)
    temp_cleanup.disarm();
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
    space: &mut SpaceCheck,
) -> Result<u64> {
    // Serialize the chunk first: the free-space check is for the bytes that
    // are written (re-serialized, with the collection mark), not the source's
    let mut records = Vec::with_capacity(docs_by_id.len());
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
        // Re-serialization sorts the keys: put the collection mark first again.
        let doc_bytes =
            super::io::mark_document_collection(coll_name, &serde_json::to_vec(doc)?)?.into_owned();
        records.push((doc_id, doc_bytes));
    }
    let chunk_bytes = records
        .iter()
        .map(|(_, bytes)| 4 + bytes.len() as u64)
        .sum::<u64>();
    space.ensure(chunk_bytes, "copying documents")?;

    for (doc_id, doc_bytes) in records {
        // Write document to new file
        let doc_offset = write_offset;
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
    space: &mut SpaceCheck,
) -> Result<u64> {
    space.ensure(4 + doc_bytes.len() as u64, "catching up")?;
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
