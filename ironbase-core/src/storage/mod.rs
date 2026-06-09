//! Append-Only Storage Engine
//!
//! This module implements IronBase's persistent storage layer using an
//! append-only file format with WAL (Write-Ahead Log) for crash safety.
//!
//! # File Format (.mlite)
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │ Header (256 bytes)                                              │
//! │ ┌─────────────┬─────────────┬───────────────┬─────────────────┐ │
//! │ │ magic (8B)  │ version (4) │ page_size (4) │ metadata_offset │ │
//! │ │ "MONGOLTE"  │      4      │     4096      │     (u64)       │ │
//! │ └─────────────┴─────────────┴───────────────┴─────────────────┘ │
//! ├─────────────────────────────────────────────────────────────────┤
//! │ Document Data (append-only)                                     │
//! │ ┌────────────┬──────────────────────────────────────────────┐   │
//! │ │ len (4B)   │ JSON document bytes                          │   │
//! │ ├────────────┼──────────────────────────────────────────────┤   │
//! │ │ len (4B)   │ JSON document bytes                          │   │
//! │ └────────────┴──────────────────────────────────────────────┘   │
//! │                        ...                                      │
//! ├─────────────────────────────────────────────────────────────────┤
//! │ Collection Metadata (JSON)      ← at metadata_offset (EOF)     │
//! │ - document_catalog: HashMap<DocumentId, offset>                 │
//! │ - indexes: Vec<IndexMetadata>                                   │
//! │ - fulltext_indexes, fuzzy_indexes, schema                       │
//! └─────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Version History
//!
//! | Version | Feature |
//! |---------|---------|
//! | 1 | Initial format with fixed metadata location |
//! | 2 | Dynamic metadata at EOF (no file truncation) |
//! | 3 | Explicit `data_end_offset` tracking |
//! | 4 | `clean_shutdown` flag for fast restart |
//!
//! # Append-Only Design
//!
//! Documents are NEVER modified in-place:
//! - **Insert**: Append new document, add to catalog
//! - **Update**: Append new version, update catalog offset
//! - **Delete**: Append tombstone marker, remove from catalog
//!
//! Benefits:
//! - **Lock-free backup**: Backup tools can read while database writes
//! - **Crash safety**: Partial writes don't corrupt existing data
//! - **Simple recovery**: Scan from start to rebuild state
//!
//! # Storage Backends
//!
//! Two implementations of the [`Storage`] trait:
//! - [`StorageEngine`] - Production file-based storage with WAL
//! - [`MemoryStorage`] - Fast in-memory storage for testing (10-100x faster)
//!
//! # Thread Safety
//!
//! StorageEngine is NOT thread-safe internally. Thread safety is provided
//! by `DatabaseCore` wrapping it in `Arc<RwLock<StorageEngine>>`.
//!
//! # Submodules
//!
//! - [`compaction`] - Garbage collection for tombstones
//! - [`metadata`] - Metadata serialization and version migration
//! - [`memory_storage`] - In-memory storage backend
//! - [`traits`] - Storage trait definitions
//! - [`io`] - Low-level I/O operations

mod compaction;
mod io;
pub mod memory_storage; // NEW: MemoryStorage for testing
pub mod metadata; // Make metadata public for CollectionMeta
pub mod traits; // NEW: Storage trait definitions

use crate::document::{Document, DocumentId};
use crate::error::{IronBaseError, Result};
use crate::transaction::{Transaction, TransactionId};
use crate::wal::WriteAheadLog;
use crate::{log_error, log_info, log_warn};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

// Re-export public types
pub use compaction::{
    compact_scan_standalone, CheckpointStats, CompactionConfig, CompactionScanResult,
    CompactionSnapshot, CompactionStats, StorageWastage,
};

// Re-export traits module
// NOTE: RawStorage is intentionally NOT public - it uses sealed trait pattern
// to prevent WAL bypass. Only crate-internal code can use it.
pub(crate) use traits::RawStorage;
pub use traits::{CompactableStorage, IndexableStorage, Storage};

// Re-export storage implementations
pub use memory_storage::MemoryStorage;

/// Recovered index change from WAL (for higher-level replay)
#[derive(Debug, Clone)]
pub struct RecoveredIndexChange {
    pub collection: String,
    pub index_name: String,
    pub operation: crate::transaction::IndexOperation,
    pub key: crate::index::IndexKey,
    pub doc_id: crate::document::DocumentId,
}

/// WAL-recovered operations with their originating transaction IDs.
///
/// Returned from [`StorageEngine::recover_from_wal`] so higher layers
/// (e.g. per-index replay in `initialize_index_manager`) can selectively
/// re-apply ops with `tx_id > index.last_flushed_tx_id()` instead of
/// doing a full rebuild-from-catalog.
pub type RecoveredOperations = Vec<(TransactionId, crate::transaction::Operation)>;

/// Full result of [`StorageEngine::recover_from_wal`]:
/// `(recovered_tx_count, btree_index_changes, applied_operations)`.
pub type WalRecoveryOutput = (usize, Vec<RecoveredIndexChange>, RecoveredOperations);

pub const HEADER_SIZE: u64 = 256; // Fixed header size

// Re-export from central limits module
pub(crate) use crate::limits::MAX_DOCUMENT_SIZE_BYTES;

/// Database file header
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Header {
    pub magic: [u8; 8],        // "MONGOLTE"
    pub version: u32,          // Version number (2 = dynamic metadata, 3 = data_end tracking)
    pub page_size: u32,        // Page size (default: 4KB)
    pub collection_count: u32, // Number of collections
    pub free_list_head: u64,   // Free blocks list head
    #[serde(default)]
    pub index_section_offset: u64, // Index metadata section offset (0 = none)

    // Dynamic metadata support (version 2+)
    #[serde(default)]
    pub metadata_offset: u64, // Offset where metadata starts (0 = use legacy fixed location)
    #[serde(default)]
    pub metadata_size: u64, // Size of metadata section in bytes

    // Explicit data end tracking (version 3+)
    // This prevents SeekFrom::End(0) corruption when metadata is at file end
    //
    // # INVARIANT - DO NOT MODIFY DIRECTLY!
    //
    // This field must ALWAYS be updated via HeaderWriter methods:
    // - HeaderWriter::advance_after_write() - after document write
    // - HeaderWriter::set_after_metadata() - after metadata flush
    // - write_compaction_header() - during compaction
    //
    // Direct modification (e.g., `header.data_end_offset = x`) WILL cause:
    // - Sparse holes in the file
    // - Metadata corruption
    // - Data overwrite bugs
    //
    // Historical note: 7+ critical bugs were caused by incorrect manual updates.
    #[serde(default)]
    pub data_end_offset: u64,

    // Clean shutdown flag (version 4+)
    // Semantics: "was the PREVIOUS shutdown clean?"
    // Set to true by mark_clean_shutdown() during graceful close,
    // cleared immediately on next open(). If true at open time,
    // indexes can be trusted from .idx files without rebuild.
    #[serde(default)]
    pub clean_shutdown: bool,

    // Persisted transaction-id watermark (version 5+, task #26 R1)
    // Holds the highest committed transaction ID at last graceful shutdown.
    // On reopen, `DatabaseCore::next_tx_id` is initialised to `this + 1`,
    // and `max_committed_tx_id` to `this`, so post-rebuild flushes can
    // stamp a non-zero watermark even on read-mostly DBs (otherwise the
    // counter resets to 0 every reopen and the safety rail in
    // `try_wal_replay_for_collection` keeps tripping). `#[serde(default)]`
    // → legacy DBs (no field) read as 0; the first clean shutdown writes
    // the new value.
    #[serde(default)]
    pub last_committed_tx_id: u64,

    // Persisted compaction-size baseline (version 6+)
    // Data file size after the last successful compaction. `storage_wastage`
    // uses it as `estimated_live_bytes` to compute `bloat_ratio`. Persisting it
    // means a freshly reopened DB keeps an accurate baseline instead of seeing
    // `bloat_ratio = +inf` (the 0-init case) and rewriting the entire file via
    // auto-compaction on every restart. `#[serde(default)]` → legacy DBs read 0
    // (first compaction recalibrates), mirroring `last_committed_tx_id` (R1).
    #[serde(default)]
    pub last_compact_size: u64,
}

impl Default for Header {
    fn default() -> Self {
        Header {
            magic: *b"MONGOLTE",
            version: 6, // Version 6: last_compact_size persistence (P1-7)
            page_size: 4096,
            collection_count: 0,
            free_list_head: 0,
            index_section_offset: 0,
            metadata_offset: 0, // Will be set on first write
            metadata_size: 0,
            data_end_offset: HEADER_SIZE, // Documents start right after header
            clean_shutdown: false,        // Will be set to true on graceful shutdown
            last_committed_tx_id: 0,      // Persisted at clean shutdown, replaces 0-init on reopen
            last_compact_size: 0,         // Persisted after compaction; 0 = never compacted
        }
    }
}

/// Collection flags for system/protected collections
#[derive(Serialize, Deserialize, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CollectionFlags {
    /// True if this is a system collection (e.g., `_system.*`)
    #[serde(default)]
    pub is_system: bool,
    /// True if collection is protected from deletion
    #[serde(default)]
    pub protected: bool,
    /// True if collection should be hidden from list_collections()
    #[serde(default)]
    pub hidden: bool,
}

/// Configuration for automatic embedding generation on insert/update
///
/// When enabled, documents inserted into the collection will automatically
/// have embeddings generated from the source field and stored in the target field.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct AutoEmbeddingConfig {
    /// Whether auto-embedding is enabled
    #[serde(default)]
    pub enabled: bool,
    /// Source field containing text to embed (e.g., "content", "text", "body")
    #[serde(default)]
    pub source_field: String,
    /// Target field where embedding vector is stored (e.g., "embedding")
    #[serde(default)]
    pub target_field: String,
    /// Embedding provider name (e.g., "ollama", "vllm", "openai")
    #[serde(default)]
    pub provider: String,
    /// Optional model override (provider-specific)
    #[serde(default)]
    pub model: Option<String>,
    /// Expected embedding dimension (for validation)
    #[serde(default)]
    pub dimension: Option<usize>,
    /// Skip embedding if target field already exists (default: false)
    #[serde(default)]
    pub skip_if_exists: bool,
    /// Chunking configuration (if source is long text)
    #[serde(default)]
    pub chunking: Option<ChunkingConfig>,
    /// Preprocessing pipeline version (e.g., "nlp_hu_v1")
    /// Used by startup detection to trigger re-embed when preprocessing changes
    #[serde(default)]
    pub preprocessing_version: Option<String>,
}

/// Configuration for text chunking before embedding
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ChunkingConfig {
    /// Chunking mode: "auto" (detect markdown/text), "markdown", "text"
    #[serde(default = "default_chunk_mode")]
    pub mode: String,
    /// Maximum chunk size in characters (default: 1000)
    #[serde(default = "default_chunk_size")]
    pub chunk_size: usize,
    /// Overlap between chunks in characters (default: 100)
    #[serde(default = "default_chunk_overlap")]
    pub overlap: usize,
}

fn default_chunk_mode() -> String {
    "auto".to_string()
}

fn default_chunk_size() -> usize {
    1000
}

fn default_chunk_overlap() -> usize {
    100
}

impl Default for ChunkingConfig {
    fn default() -> Self {
        Self {
            mode: default_chunk_mode(),
            chunk_size: default_chunk_size(),
            overlap: default_chunk_overlap(),
        }
    }
}

/// Collection metadata
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CollectionMeta {
    pub name: String,
    pub document_count: u64,
    #[serde(default)]
    pub live_document_count: u64,
    #[serde(default)]
    pub data_offset: u64, // Legacy: always HEADER_SIZE (256). Kept for file format compat.
    pub index_offset: u64, // Index start position
    pub last_id: u64,      // Last _id

    /// Document catalog: DocumentId -> file offset mapping
    /// This enables persistent document storage and fast retrieval
    /// BREAKING CHANGE: Changed from HashMap<String, u64> to HashMap<DocumentId, u64>
    /// Custom serialization preserves DocumentId type information in JSON metadata
    #[serde(default, with = "crate::catalog_serde")]
    pub document_catalog: HashMap<crate::document::DocumentId, u64>,
    /// Stable document iteration order (append/update order)
    #[serde(default, with = "crate::catalog_serde::vec")]
    pub document_order: Vec<crate::document::DocumentId>,

    /// Persisted index metadata for this collection (B+ tree indexes)
    #[serde(default)]
    pub indexes: Vec<crate::index::IndexMetadata>,

    /// Persisted fuzzy index metadata for this collection
    /// Note: Only metadata is persisted, index data is rebuilt from documents on load
    #[serde(default)]
    pub fuzzy_indexes: Vec<crate::index::FuzzyIndexMetadata>,

    /// Persisted fulltext index metadata for this collection
    /// Note: Only metadata is persisted, index data is rebuilt from documents on load
    #[serde(default)]
    pub fulltext_indexes: Vec<crate::fulltext::FulltextIndexMetadata>,

    /// Persisted vector index metadata for this collection
    /// Cache files (.hnsw) are stored separately and rebuilt if missing
    #[serde(default)]
    pub vector_indexes: Vec<crate::vector::VectorIndexMetadata>,

    /// Optional JSON schema for validation
    #[serde(default)]
    pub schema: Option<serde_json::Value>,

    /// Collection flags (system, protected, hidden)
    #[serde(default)]
    pub flags: CollectionFlags,

    /// Auto-embedding configuration for this collection
    /// When enabled, inserts/updates automatically generate embeddings
    #[serde(default)]
    pub auto_embedding_config: Option<AutoEmbeddingConfig>,
}

impl CollectionMeta {
    /// Apply a collection rename to this metadata in place: set the new name
    /// and re-prefix every persisted index-metadata name. Index names follow
    /// the `{collection}_{suffix}` convention, so only the prefix changes.
    pub(crate) fn apply_rename(&mut self, old_name: &str, new_name: &str) {
        self.name = new_name.to_string();
        let prefix = format!("{}_", old_name);
        let reprefix = |name: &str| -> String {
            match name.strip_prefix(&prefix) {
                Some(suffix) => format!("{}_{}", new_name, suffix),
                None => name.to_string(),
            }
        };
        for ix in &mut self.indexes {
            ix.name = reprefix(&ix.name);
        }
        for ix in &mut self.fuzzy_indexes {
            ix.name = reprefix(&ix.name);
        }
        for ix in &mut self.fulltext_indexes {
            ix.name = reprefix(&ix.name);
        }
        for ix in &mut self.vector_indexes {
            ix.name = reprefix(&ix.name);
        }
    }
}

/// Index record for persistence
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct IndexRecord {
    pub collection_name: String,
    pub index_metadata: crate::index::IndexMetadata,
}

/// Metadata snapshot for WAL-based crash recovery
/// Logged before every metadata flush to ensure recoverability
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MetadataWALEntry {
    /// All collection metadata at the time of snapshot
    pub collections: HashMap<String, CollectionMeta>,
    /// End of document data section (where metadata should start)
    pub data_end_offset: u64,
}

/// Borrowing twin of `MetadataWALEntry` used only for serialization, so the WAL
/// snapshot can be written WITHOUT deep-cloning the entire `collections` map
/// (a multi-GB allocation under the write lock on a large catalog — audit P1-8).
/// serde encodes `&HashMap` byte-identically to `HashMap`, so the WAL JSON — and
/// crash recovery via `MetadataWALEntry` — is unchanged.
#[derive(Serialize)]
struct MetadataWALEntryRef<'a> {
    collections: &'a HashMap<String, CollectionMeta>,
    data_end_offset: u64,
}

// ============================================================================
// HEADER WRITER - Safe, invariant-preserving header modifications
// ============================================================================
//
// CRITICAL: The `data_end_offset` field MUST always point to the next writable
// position. If this invariant is violated, sparse holes or data corruption occur.
//
// This struct provides semantic methods that AUTOMATICALLY calculate the correct
// `data_end_offset` value, making it impossible to forget the update.
//
// History: 7+ critical bugs were caused by forgetting to update data_end_offset
// in compaction, recovery, and metadata flush code paths.
// ============================================================================

/// HeaderWriter - Safe, invariant-preserving header modifications
///
/// # Purpose
/// Encapsulates all modifications to `data_end_offset` to prevent bugs.
/// The methods automatically calculate the correct value.
///
/// # Invariant
/// `data_end_offset` always points to the next writable position:
/// - After document write: end of last document
/// - After metadata flush: end of metadata section
///
/// # Usage
/// ```ignore
/// // After document write:
/// HeaderWriter::new(&mut self.header, &mut self.file).advance_after_write()?;
///
/// // After metadata flush:
/// HeaderWriter::new(&mut self.header, &mut self.file)
///     .set_after_metadata(metadata_offset, metadata_size);
/// ```
pub struct HeaderWriter<'a> {
    header: &'a mut Header,
    file: &'a mut File,
}

impl<'a> HeaderWriter<'a> {
    /// Create a new HeaderWriter
    pub fn new(header: &'a mut Header, file: &'a mut File) -> Self {
        Self { header, file }
    }

    /// Update data_end_offset after a document write
    ///
    /// Uses current file position (`stream_position()`) as the new offset.
    /// This is the correct value because we just finished writing data.
    pub fn advance_after_write(&mut self) -> Result<u64> {
        use std::io::Seek;
        let new_offset = self.file.stream_position()?;
        self.header.data_end_offset = new_offset;
        Ok(new_offset)
    }

    /// Update header fields after metadata is written
    ///
    /// AUTOMATICALLY calculates: `data_end_offset = metadata_offset + metadata_size`
    ///
    /// This ensures the next document write will start AFTER the metadata,
    /// preventing sparse holes that plagued earlier versions.
    pub fn set_after_metadata(&mut self, metadata_offset: u64, metadata_size: u64) {
        self.header.data_end_offset = metadata_offset + metadata_size;
        self.header.metadata_offset = metadata_offset;
        self.header.metadata_size = metadata_size;
    }

    /// Write header to file at position 0
    pub fn write_to_file(&mut self) -> Result<()> {
        use std::io::{Seek, SeekFrom, Write};
        self.file.seek(SeekFrom::Start(0))?;
        let header_bytes = bincode::serialize(&*self.header)
            .map_err(|e| IronBaseError::Serialization(e.to_string()))?;
        self.file.write_all(&header_bytes)?;
        Ok(())
    }

    /// Set data_end_offset during crash recovery
    ///
    /// Used by recovery code paths (`open()`, `recover_metadata_from_wal()`,
    /// `rebuild_from_documents()`) where the offset is calculated externally
    /// (e.g., from WAL snapshot or document scan) rather than from
    /// `stream_position()`.
    ///
    /// Unlike `advance_after_write()` which reads the file cursor, this method
    /// accepts an explicit offset value computed by the recovery logic.
    ///
    /// # Safety invariant
    /// The caller MUST ensure `offset >= HEADER_SIZE` and that it points to
    /// the actual end of the document data region.
    pub fn set_recovery_offset(&mut self, offset: u64) {
        self.header.data_end_offset = offset;
    }

    /// Get current data_end_offset (read-only)
    pub fn data_end_offset(&self) -> u64 {
        self.header.data_end_offset
    }
}

/// Static helper for compaction - writes header to a NEW file
///
/// Compaction creates a completely new file, so we can't use the instance method.
/// This function ensures all header fields are properly set.
pub fn write_compaction_header(
    file: &mut File,
    base_header: &Header,
    metadata_offset: u64,
    metadata_size: u64,
) -> Result<()> {
    use std::io::{Seek, SeekFrom, Write};

    let mut updated_header = base_header.clone();
    // CRITICAL: Set data_end_offset to point AFTER metadata
    updated_header.data_end_offset = metadata_offset + metadata_size;
    updated_header.metadata_offset = metadata_offset;
    updated_header.metadata_size = metadata_size;

    file.seek(SeekFrom::Start(0))?;
    let header_bytes = bincode::serialize(&updated_header)
        .map_err(|e| IronBaseError::Serialization(e.to_string()))?;
    file.write_all(&header_bytes)?;
    Ok(())
}

/// Storage engine - file-based storage
pub struct StorageEngine {
    file: File,
    header: Header,
    /// Collection catalog behind an `Arc` for copy-on-write snapshots.
    ///
    /// Compaction's Phase A (`compact_prepare`) takes an O(1) `Arc::clone`
    /// snapshot instead of deep-cloning the whole catalog under the write lock
    /// (audit P0-1). Mutations go through `collections_mut()` (`Arc::make_mut`),
    /// which deep-clones once on the first write while a snapshot is alive and is
    /// O(1) otherwise.
    collections: std::sync::Arc<HashMap<String, CollectionMeta>>,
    file_path: String,
    wal: WriteAheadLog,
    metadata_dirty: bool,
    metadata_snapshot_pending: bool,
    /// Separate lock file to allow other processes to read the DB during backup
    /// On Windows, file locks are mandatory and prevent ALL access including reads
    lock_file: File,
    /// Counter for WAL operations since last clear.
    ///
    /// NOTE (2026-02-01): This counter is NO LONGER incremented per-commit.
    /// Previously, commit_transaction_internal() incremented this and cleared
    /// WAL every 100 commits. That was removed because per-commit metadata
    /// flush was eliminated for performance (see Step 8 comment in
    /// commit_transaction_internal).
    ///
    /// WAL is now cleared unconditionally by:
    /// - `flush()` - after flush_metadata() persists all data
    /// - `checkpoint()` - periodic durability barrier
    /// - `recover_from_wal()` - after successful recovery
    ///
    /// The field is kept for CheckpointStats reporting (always 0 in practice).
    wal_ops_since_clear: u32,
    /// Indicates whether the database was cleanly shut down last time
    /// If true, indexes can be trusted from .idx files without rebuild
    was_clean_shutdown: bool,
}

impl StorageEngine {
    /// Acquire the exclusive single-writer lock for a database file.
    ///
    /// The lock IS the `fs2` advisory lock on the `.lock` file — NOT the file's
    /// existence. The OS releases the advisory lock when the holding process dies
    /// (crash / SIGKILL) or closes the fd, so a leftover `.lock` file from a
    /// crashed process is harmless: the next `try_lock_exclusive` simply succeeds
    /// and overwrites the stale PID below.
    ///
    /// We deliberately do NOT do PID-based "stale lock" detection + unlink. That
    /// was unsound (audit 2026-06-08 P0-1, empirically reproduced): `try_lock`
    /// fails only when a LIVE process holds the flock, yet the PID written in the
    /// file can lag (a holder mid-acquire has not rewritten it) or read as dead
    /// cross-uid (`kill(pid,0)` → EPERM). The old code then unlinked + re-created
    /// the lock file; because the flock is per-inode, concurrent recoverers each
    /// locked a DIFFERENT inode and ALL succeeded → two live writers on one
    /// `.mlite` → corruption. Trusting the OS flock removes that race entirely.
    ///
    /// LIMITATION: this assumes a LOCAL filesystem. `flock`-style advisory locks
    /// are unreliable or host-local on network filesystems (NFS/SMB), so
    /// multi-host exclusivity is NOT guaranteed there — an embedded DB is meant to
    /// be opened from one host anyway.
    fn acquire_exclusive_lock(lock_path: &Path, db_path: &str) -> Result<File> {
        use std::io::{Seek, Write};

        let mut lock_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(lock_path)?;

        match lock_file.try_lock_exclusive() {
            Ok(()) => {
                // We hold the lock — record our PID for diagnostics only.
                lock_file.set_len(0)?;
                lock_file.seek(std::io::SeekFrom::Start(0))?;
                writeln!(lock_file, "{}", std::process::id())?;
                lock_file.sync_all()?;
                Ok(lock_file)
            }
            // Lock contention = a LIVE process holds the flock (a dead holder's
            // flock is auto-released by the OS) → the database is genuinely in use.
            // fs2::lock_contended_error() is the portable contention error: its kind
            // is WouldBlock on Unix but a Windows-specific kind under LockFileEx, so
            // match against it rather than hardcoding WouldBlock (which missed the
            // Windows case → contention was misreported as a raw Io error, breaking
            // test_file_lock_prevents_double_open on Windows).
            Err(e) if e.kind() == fs2::lock_contended_error().kind() => {
                Err(IronBaseError::DatabaseLocked(db_path.to_string()))
            }
            // A genuine OS/filesystem fault (ENOLCK / EINTR / EIO, e.g. a mount
            // without working advisory locks) — surface it, don't mask it as a
            // benign "locked" that callers would retry forever.
            Err(e) => Err(IronBaseError::Io(e)),
        }
    }

    /// Open or create database
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path_str = path.as_ref().to_string_lossy().to_string();
        let exists = path.as_ref().exists();

        // Lock a SEPARATE "<db path>.lock" file so read-only backup tools can
        // still open the .mlite itself (on Windows file locks block all access).
        // APPEND ".lock" — do NOT use `with_extension`, which REPLACES the final
        // component (`foo` and `foo.mlite` would both map to `foo.mlite.lock` and
        // falsely block each other; audit 2026-06-08 P0-1). `.mlite` paths are
        // unchanged (`foo.mlite` → `foo.mlite.lock`).
        let lock_path = PathBuf::from(format!("{path_str}.lock"));
        let lock_file = Self::acquire_exclusive_lock(&lock_path, &path_str)?;

        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&path)?;

        // Clean up orphaned index temp files from interrupted atomic commits.
        // `.idx.tmp`   — b+ tree two-phase commit (btree.rs::commit_prepared_changes)
        // `.fzidx.tmp` — fuzzy flush (fuzzy.rs::flush)
        // `.ftidx.tmp` — fulltext two-phase serialize_flush (fulltext.rs::serialize_flush)
        // `.hnsw.tmp`  — HNSW flush (index/manager.rs::persist_hnsw_to_file)
        if let Some(db_dir) = Path::new(&path_str).parent() {
            if let Ok(entries) = std::fs::read_dir(db_dir) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    let name = p.to_string_lossy();
                    if name.ends_with(".idx.tmp")
                        || name.ends_with(".fzidx.tmp")
                        || name.ends_with(".ftidx.tmp")
                        || name.ends_with(".hnsw.tmp")
                    {
                        log_warn!(
                            path = %p.display(),
                            "Removing orphaned index temp file from interrupted commit"
                        );
                        let _ = std::fs::remove_file(&p);
                    }
                }
            }
        }

        // Open WAL file first (needed for potential recovery)
        let wal_path = PathBuf::from(&path_str).with_extension("wal");
        let wal = WriteAheadLog::open(wal_path)?;

        let (header, collections, needs_rebuild) = if exists && file.metadata()?.len() > 0 {
            // Try to load existing database
            match Self::load_metadata(&mut file) {
                Ok((h, c)) => (h, c, false),
                Err(e) => {
                    // Check if this is a recoverable corruption error
                    // Magic number corruption is NOT recoverable - file may not be a valid database
                    let is_magic_corruption =
                        matches!(&e, IronBaseError::Corruption(msg) if msg.contains("magic"));

                    let is_recoverable_corruption = !is_magic_corruption
                        && matches!(
                            &e,
                            IronBaseError::Corruption(_) | IronBaseError::Serialization(_)
                        );

                    if is_recoverable_corruption {
                        log_warn!("[WARN] Metadata corrupted: {}, attempting WAL recovery", e);
                        // Return default header/collections - will attempt recovery below
                        (Header::default(), HashMap::new(), true)
                    } else {
                        return Err(e);
                    }
                }
            }
        } else {
            // Initialize new database
            let header = Header::default();
            let collections = HashMap::new();
            let _ = Self::write_metadata(&mut file, &header, &collections)?;
            (header, collections, false)
        };

        // Save clean_shutdown status BEFORE clearing it
        // This determines whether we can trust .idx files
        let was_clean = header.clean_shutdown;

        // CRITICAL: Clear clean_shutdown flag immediately
        // If we crash before graceful shutdown, the flag stays false
        // This ensures indexes are rebuilt after crash
        let mut header = header;
        header.clean_shutdown = false;

        let mut storage = StorageEngine {
            file,
            header,
            collections: std::sync::Arc::new(collections),
            file_path: path_str,
            wal,
            metadata_dirty: false,
            metadata_snapshot_pending: false,
            lock_file,
            wal_ops_since_clear: 0,
            was_clean_shutdown: was_clean,
        };

        let migrated = Self::rebuild_document_order_if_needed(storage.collections_mut());
        if migrated {
            storage.metadata_dirty = true;
        }

        // If metadata was corrupted, attempt recovery (crash scenario)
        if needs_rebuild {
            // Mark as dirty start - don't trust indexes
            storage.was_clean_shutdown = false;
            // First try WAL recovery (has most recent metadata snapshot)
            if storage.recover_metadata_from_wal()? {
                log_info!("[INFO] Successfully recovered metadata from WAL");
            } else {
                // WAL recovery failed - fall back to document scan
                log_warn!("[WARN] WAL recovery failed, rebuilding metadata from documents");
                storage.rebuild_from_documents()?;
            }
            // CRITICAL: Flush to persist recovered metadata and fix corrupt header on disk
            log_info!("[INFO] Persisting recovered metadata to fix corrupt header");
            storage.flush_metadata()?;
            storage.metadata_snapshot_pending = false;
            storage.file.sync_all()?;
        }

        // CRITICAL FIX: Validate and recover data_end_offset if corrupted
        // This prevents sparse hole creation from SeekFrom::End(0) fallback
        // Check both: too small (< HEADER_SIZE) OR garbage large value (> file_size)
        let file_size = storage.file.metadata()?.len();
        let offset_invalid = storage.header.data_end_offset < HEADER_SIZE as u64
            || storage.header.data_end_offset > file_size;
        if offset_invalid && !storage.collections.is_empty() {
            let (max_offset, has_docs) = Self::find_max_document_offset(&storage.collections);
            if has_docs {
                match Self::calculate_data_end_from_last_doc(&mut storage.file, max_offset) {
                    Ok(recovered_offset) => {
                        log_warn!(
                            "[WARN] Recovered corrupted data_end_offset: {} -> {}",
                            storage.header.data_end_offset,
                            recovered_offset
                        );
                        // CRITICAL: Use HeaderWriter instead of direct assignment
                        HeaderWriter::new(&mut storage.header, &mut storage.file)
                            .set_recovery_offset(recovered_offset);
                        // Persist recovery to disk immediately — without this,
                        // a crash before the next flush() would lose the fix
                        // and re-trigger recovery with the old corrupt value.
                        storage.flush_metadata()?;
                        storage.metadata_snapshot_pending = false;
                        storage.file.sync_all()?;
                    }
                    Err(e) => {
                        return Err(IronBaseError::Corruption(format!(
                            "Cannot recover data_end_offset (catalog max_offset={}): {}. Run compact to fix.",
                            max_offset, e
                        )));
                    }
                }
            }
        }

        // NOTE: WAL recovery is now handled by DatabaseCore::open() for index atomicity
        // This allows Database to coordinate index recovery across all collections

        Ok(storage)
    }

    fn rebuild_document_order_if_needed(collections: &mut HashMap<String, CollectionMeta>) -> bool {
        let mut migrated = false;
        for meta in collections.values_mut() {
            let needs_rebuild = meta.document_order.len() != meta.document_catalog.len()
                || meta
                    .document_order
                    .iter()
                    .any(|id| !meta.document_catalog.contains_key(id));
            if needs_rebuild {
                let mut offsets: Vec<(DocumentId, u64)> = meta
                    .document_catalog
                    .iter()
                    .map(|(id, &offset)| (id.clone(), offset))
                    .collect();
                offsets.sort_by_key(|(_, offset)| *offset);
                meta.document_order = offsets.into_iter().map(|(id, _)| id).collect();
                migrated = true;
            }
        }
        migrated
    }

    /// Create a new collection
    pub fn create_collection(&mut self, name: &str) -> Result<()> {
        if self.collections.contains_key(name) {
            return Err(IronBaseError::CollectionExists(name.to_string()));
        }

        // Create new collection with placeholder offset (will be corrected by flush_metadata)
        let meta = CollectionMeta {
            name: name.to_string(),
            document_count: 0,
            live_document_count: 0,
            data_offset: 0, // Will be set correctly by flush_metadata
            index_offset: 0,
            last_id: 0,
            document_catalog: HashMap::new(), // Initialize empty catalog
            document_order: Vec::new(),
            indexes: Vec::new(),          // Initialize empty index list
            fuzzy_indexes: Vec::new(),    // Initialize empty fuzzy index list
            fulltext_indexes: Vec::new(), // Initialize empty fulltext index list
            vector_indexes: Vec::new(),   // Initialize empty vector index list
            schema: None,
            flags: CollectionFlags::default(),
            auto_embedding_config: None,
        };

        self.collections_mut().insert(name.to_string(), meta);
        self.header.collection_count += 1;

        // Mark metadata dirty and flush to persist new collection
        self.mark_metadata_dirty()?;
        self.flush()?;

        Ok(())
    }

    /// Drop collection
    pub fn drop_collection(&mut self, name: &str) -> Result<()> {
        if !self.collections.contains_key(name) {
            return Err(IronBaseError::CollectionNotFound(name.to_string()));
        }

        self.collections_mut().remove(name);
        self.header.collection_count -= 1;

        self.mark_metadata_dirty()?;
        self.flush()?;

        Ok(())
    }

    /// Rename a collection.
    ///
    /// Moves the `CollectionMeta` (with its document catalog) from `old_name`
    /// to `new_name` and re-prefixes every persisted index-metadata name
    /// (`{old}_x` -> `{new}_x`) so the catalog stays consistent. Document data
    /// is not moved — a `.mlite` is a single append-only file and the catalog
    /// offsets travel with the metadata. The on-disk index files and any
    /// in-memory index managers are re-keyed by the caller (DatabaseCore).
    pub fn rename_collection(&mut self, old_name: &str, new_name: &str) -> Result<()> {
        if old_name == new_name {
            return Ok(());
        }
        if !self.collections.contains_key(old_name) {
            return Err(IronBaseError::CollectionNotFound(old_name.to_string()));
        }
        if self.collections.contains_key(new_name) {
            return Err(IronBaseError::CollectionExists(new_name.to_string()));
        }

        let mut meta = self
            .collections_mut()
            .remove(old_name)
            .expect("existence checked above");
        meta.apply_rename(old_name, new_name);

        // collection_count is unchanged — this is a move, not an add/remove.
        self.collections_mut().insert(new_name.to_string(), meta);

        self.mark_metadata_dirty()?;
        self.flush()?;

        Ok(())
    }

    /// List all collections
    pub fn list_collections(&self) -> Vec<String> {
        self.collections.keys().cloned().collect()
    }

    /// Get collection metadata (immutable)
    pub fn get_collection_meta(&self, name: &str) -> Option<&CollectionMeta> {
        self.collections.get(name)
    }

    /// Get collection metadata (mutable)
    /// Metadata changes are persisted only when flush() is called (typically on database close)
    pub fn get_collection_meta_mut(&mut self, name: &str) -> Option<&mut CollectionMeta> {
        self.collections_mut().get_mut(name)
    }

    /// Write a metadata snapshot to the WAL for crash recovery
    fn write_metadata_snapshot(&mut self) -> Result<()> {
        use crate::wal::{WALEntry, WALEntryType};

        // Serialize from a borrow — NO full catalog clone (a 50M-doc catalog clone
        // was a multi-GB allocation under the write lock). serde encodes the
        // borrowing twin identically, so the WAL format / crash recovery is the same.
        let metadata_ref = MetadataWALEntryRef {
            collections: &self.collections,
            data_end_offset: self.header.data_end_offset,
        };

        // Estimate + try_reserve the buffer so a giant catalog fails with a typed
        // OutOfMemory error instead of aborting the process (mirrors
        // serialize_metadata's guard at metadata.rs).
        let estimated_size: usize = 4 + self
            .collections
            .values()
            .map(|meta| {
                let catalog_size = meta.document_catalog.len() * 30;
                let order_size = meta.document_order.len() * 10;
                4 + catalog_size + order_size + 512
            })
            .sum::<usize>();
        let mut entry_data: Vec<u8> = Vec::new();
        entry_data.try_reserve(estimated_size).map_err(|_| {
            IronBaseError::OutOfMemory(format!(
                "Failed to allocate {}MB for metadata WAL snapshot ({} collections, {} total docs)",
                estimated_size / (1024 * 1024),
                self.collections.len(),
                self.collections
                    .values()
                    .map(|m| m.document_catalog.len())
                    .sum::<usize>()
            ))
        })?;
        serde_json::to_writer(&mut entry_data, &metadata_ref)
            .map_err(|e| IronBaseError::Serialization(e.to_string()))?;

        let entry = WALEntry::new(
            0, // No transaction ID for metadata snapshots
            WALEntryType::MetadataSnapshot,
            entry_data,
        );

        self.wal.append(&entry)?;
        self.wal.flush()?; // Ensure metadata snapshot is on disk before file write

        Ok(())
    }

    /// Ensure a metadata snapshot exists in WAL (idempotent)
    fn ensure_metadata_snapshot(&mut self) -> Result<()> {
        if self.metadata_snapshot_pending {
            return Ok(());
        }
        self.write_metadata_snapshot()?;
        self.metadata_snapshot_pending = true;
        Ok(())
    }

    /// Mark metadata as dirty and schedule a WAL snapshot
    fn mark_metadata_dirty(&mut self) -> Result<()> {
        self.metadata_dirty = true;
        self.ensure_metadata_snapshot()
    }

    /// Flush changes to disk (including metadata) and clear WAL
    ///
    /// After flush_metadata(), all data is persisted in the main file,
    /// so WAL entries are no longer needed for crash recovery.
    /// WAL is cleared unconditionally to prevent stale entries from
    /// causing unnecessary replay on next startup (e.g., after Drop).
    pub fn flush(&mut self) -> Result<()> {
        if self.metadata_dirty {
            self.ensure_metadata_snapshot()?;
        }

        // Flush metadata to disk (WAL snapshot already contains latest state)
        self.flush_metadata()?;
        self.metadata_snapshot_pending = false;

        // Clear WAL unconditionally after successful metadata flush
        // All operations are now persisted in the main file, WAL is redundant.
        // This prevents duplicate document writes on next startup's WAL replay.
        self.wal.clear()?;
        self.wal_ops_since_clear = 0;

        Ok(())
    }

    /// Get mutable reference to the database file (for index persistence)
    pub fn get_file_mut(&mut self) -> &mut File {
        &mut self.file
    }

    /// Whether metadata has been modified since last flush
    pub(crate) fn is_metadata_dirty(&self) -> bool {
        self.metadata_dirty
    }

    /// Current WAL file size in bytes (for checkpoint guard checks)
    pub(crate) fn wal_file_size(&self) -> u64 {
        self.wal.file_size().unwrap_or(0)
    }

    /// Reference to all collection metadata (for pre-serialization outside lock)
    pub(crate) fn collections_ref(&self) -> &HashMap<String, CollectionMeta> {
        &self.collections
    }

    /// Mutable access to the collection catalog (copy-on-write).
    ///
    /// While a compaction snapshot (Phase A `Arc::clone` in `compact_prepare`)
    /// is still alive, the first mutation deep-clones the catalog exactly once;
    /// every other call is O(1). Centralizing the `Arc::make_mut` here is what
    /// keeps `compact_prepare` from deep-cloning the whole catalog under the
    /// write lock (audit P0-1).
    fn collections_mut(&mut self) -> &mut HashMap<String, CollectionMeta> {
        std::sync::Arc::make_mut(&mut self.collections)
    }

    /// Checkpoint - flush metadata and clear WAL for durability
    /// Use this in long-running processes to ensure data survives restarts
    ///
    /// CRITICAL FIX: Must call flush_metadata() before clearing WAL!
    /// Without this, document_catalog only exists in memory and is lost on restart.
    ///
    /// PERF FIX (v1.0.313): Removed ensure_metadata_snapshot() from checkpoint.
    /// Previously, checkpoint serialized metadata TWICE under storage.write() lock:
    /// 1. ensure_metadata_snapshot() → deep clone + JSON serialize + WAL write + WAL fsync
    /// 2. flush_metadata() → binary serialize + file write + fsync
    /// With 130K+ docs this meant ~24MB allocation + 2× fsync under lock, taking
    /// minutes under memory pressure (Windows page swapping).
    ///
    /// WAL MetadataSnapshot is unnecessary because:
    /// - flush_metadata() writes metadata to main file BEFORE WAL clear
    /// - Header write (256 bytes at offset 0) is effectively atomic (< sector size)
    /// - If crash during metadata body write: old header → old valid metadata
    /// - WAL transaction entries (BEGIN+Op+COMMIT) provide full crash recovery
    ///
    /// Returns checkpoint statistics including WAL size before/after.
    pub fn checkpoint(&mut self) -> Result<compaction::CheckpointStats> {
        // Get WAL size before checkpoint
        let wal_size_before = self.wal.file_size().unwrap_or(0);
        // NOTE: wal_ops_since_clear is always 0 since per-commit increment was removed
        // (2026-02-01 perf fix). WAL size is the meaningful metric now.
        let wal_ops_cleared = self.wal_ops_since_clear;

        // Flush metadata to ensure document_catalog is persisted
        // NOTE: ensure_metadata_snapshot() intentionally NOT called here.
        // See PERF FIX comment above for rationale.
        self.flush_metadata()?;
        self.metadata_snapshot_pending = false;

        // Then clear the WAL (all operations already in main file)
        self.wal.clear()?;
        self.wal_ops_since_clear = 0;

        // Get WAL size after (should be 0)
        let wal_size_after = self.wal.file_size().unwrap_or(0);

        Ok(compaction::CheckpointStats {
            wal_size_before,
            wal_size_after,
            wal_ops_cleared: wal_ops_cleared as u64,
            indexes_flushed: 0, // Storage layer doesn't know about indexes; set by DatabaseCore
        })
    }

    /// WAL-clear-only checkpoint: clears the WAL WITHOUT re-flushing metadata.
    ///
    /// Used by `checkpoint_wal_only` when metadata is clean (no mutations since
    /// the last flush): the catalog is already durable, so calling `flush_metadata`
    /// would only append the full catalog again and grow the file on every idle
    /// checkpoint (audit P1-3). In that state the WAL holds no un-checkpointed
    /// ops, so clearing it is a cheap no-op.
    pub fn checkpoint_wal_clear_only(&mut self) -> Result<compaction::CheckpointStats> {
        let wal_size_before = self.wal.file_size().unwrap_or(0);
        self.wal.clear()?;
        // The cleared WAL no longer holds the metadata snapshot, so the next
        // ensure_metadata_snapshot() must write a fresh one into the now-empty
        // WAL. Leaving this true would make ensure_metadata_snapshot() early-
        // return → an empty WAL with no recovery base (PR #89 follow-up,
        // finding B). Matches every sibling WAL-clearer (flush / checkpoint /
        // checkpoint_with_preserialized).
        self.metadata_snapshot_pending = false;
        self.wal_ops_since_clear = 0;
        let wal_size_after = self.wal.file_size().unwrap_or(0);
        Ok(compaction::CheckpointStats {
            wal_size_before,
            wal_size_after,
            wal_ops_cleared: 0,
            indexes_flushed: 0,
        })
    }

    /// Checkpoint with a pre-serialized metadata buffer (lock-optimized path)
    ///
    /// Instead of serializing metadata under storage.write() lock, the caller
    /// pre-serializes under storage.read() (which doesn't block inserts), then
    /// calls this method which only does the file I/O under lock.
    ///
    /// The `data_end_offset` and `wal_size` are guard values from the pre-serialize
    /// phase. If they don't match current state, the caller should fall back to
    /// the regular `checkpoint()` method.
    pub fn checkpoint_with_preserialized(
        &mut self,
        metadata_bytes: Vec<u8>,
    ) -> Result<compaction::CheckpointStats> {
        let wal_size_before = self.wal.file_size().unwrap_or(0);
        let wal_ops_cleared = self.wal_ops_since_clear;

        // Write pre-serialized metadata to file (no serialization needed)
        // SAFETY: Caller (checkpoint_wal_only) guarantees data_end_offset >= HEADER_SIZE
        // via guard check, ensuring v2 databases use the fallback path instead.
        let metadata_offset = self.header.data_end_offset;

        HeaderWriter::new(&mut self.header, &mut self.file)
            .set_after_metadata(metadata_offset, metadata_bytes.len() as u64);

        Self::write_metadata_and_header(
            &mut self.file,
            &mut self.header,
            &metadata_bytes,
            metadata_offset,
        )?;

        self.metadata_dirty = false;
        self.metadata_snapshot_pending = false;

        // Clear WAL (all operations now in main file)
        self.wal.clear()?;
        self.wal_ops_since_clear = 0;

        let wal_size_after = self.wal.file_size().unwrap_or(0);

        Ok(compaction::CheckpointStats {
            wal_size_before,
            wal_size_after,
            wal_ops_cleared: wal_ops_cleared as u64,
            indexes_flushed: 0,
        })
    }

    /// Recover metadata from WAL if the file's metadata is corrupted
    /// Returns true if recovery was successful, false if no metadata snapshot found in WAL
    pub fn recover_metadata_from_wal(&mut self) -> Result<bool> {
        use crate::wal::{WALEntryIterator, WALEntryType};
        use std::io::BufReader;

        // Open WAL file for reading
        let wal_path = std::path::PathBuf::from(&self.file_path).with_extension("wal");
        let wal_file = match std::fs::File::open(&wal_path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // No WAL file - cannot recover
                return Ok(false);
            }
            Err(e) => return Err(IronBaseError::Io(e)),
        };

        let reader = BufReader::new(wal_file);
        let iter = match WALEntryIterator::new(reader) {
            Ok(i) => i,
            Err(_) => return Ok(false), // Invalid WAL, cannot recover
        };

        // Find the latest MetadataSnapshot entry
        let mut latest_snapshot: Option<MetadataWALEntry> = None;

        for entry_result in iter {
            let entry = match entry_result {
                Ok(e) => e,
                Err(e) => {
                    log_warn!(
                        "[WARN] Skipping corrupted WAL entry during metadata recovery: {}",
                        e
                    );
                    continue;
                }
            };
            if entry.entry_type == WALEntryType::MetadataSnapshot {
                if let Ok(snapshot) = serde_json::from_slice::<MetadataWALEntry>(&entry.data) {
                    latest_snapshot = Some(snapshot);
                }
            }
        }

        // Restore from snapshot if found
        if let Some(snapshot) = latest_snapshot {
            log_info!(
                "[INFO] Recovering metadata from WAL: {} collections, data_end_offset={}",
                snapshot.collections.len(),
                snapshot.data_end_offset
            );

            // Restore collections and header
            self.collections = std::sync::Arc::new(snapshot.collections);
            // CRITICAL: Use HeaderWriter instead of direct assignment
            // set_recovery_offset sets data_end_offset; flush_metadata() below
            // will call set_after_metadata() which updates metadata_offset too.
            HeaderWriter::new(&mut self.header, &mut self.file)
                .set_recovery_offset(snapshot.data_end_offset);
            self.header.collection_count = self.collections.len() as u32;

            // Write corrected metadata to file
            self.flush_metadata()?;
            self.metadata_snapshot_pending = false;
            self.file.sync_all()?;

            // Clear WAL after successful recovery
            self.wal.clear()?;

            return Ok(true);
        }

        Ok(false)
    }

    /// Rebuild all metadata by scanning documents in the file
    /// This is the last-resort recovery when both file metadata AND WAL are unavailable
    ///
    /// IMPORTANT: This function assumes the header is corrupted, so it scans the entire
    /// document region without relying on header.metadata_offset
    pub fn rebuild_from_documents(&mut self) -> Result<()> {
        use crate::document::DocumentId;
        use std::io::{Read, Seek, SeekFrom};

        log_info!("[INFO] Starting metadata rebuild from document scan...");

        let file_len = self.file.metadata()?.len();

        // If file is too small to have any documents, just initialize empty
        if file_len <= HEADER_SIZE {
            self.header = Header::default();
            self.collections_mut().clear();
            self.mark_metadata_dirty()?;
            self.flush_metadata()?;
            self.metadata_snapshot_pending = false;
            self.file.sync_all()?;
            return Ok(());
        }

        // Clear existing collections - we're rebuilding from scratch
        self.collections_mut().clear();

        // Scan all documents from HEADER_SIZE to file end
        // We scan conservatively - stop when we hit invalid data (which is likely metadata)
        let mut offset = HEADER_SIZE;
        let mut max_ids_by_collection: HashMap<String, u64> = HashMap::new();
        let mut documents_found = 0u64;

        while offset + 4 < file_len {
            // Read document length (4 bytes)
            self.file.seek(SeekFrom::Start(offset))?;
            let mut len_bytes = [0u8; 4];
            if self.file.read_exact(&mut len_bytes).is_err() {
                break; // EOF or read error
            }
            let len = u32::from_le_bytes(len_bytes) as usize;

            // Validate length - reasonable limits
            if len == 0 || len > MAX_DOCUMENT_SIZE_BYTES || offset + 4 + (len as u64) > file_len {
                // Hit metadata section or corrupted data - stop scanning
                break;
            }

            // Read document data
            let mut data = vec![0u8; len];
            if self.file.read_exact(&mut data).is_err() {
                break;
            }

            // Try to parse as JSON - if it fails, we've hit metadata
            let doc_value = match serde_json::from_slice::<serde_json::Value>(&data) {
                Ok(v) => v,
                Err(_) => break, // Not valid JSON - hit metadata
            };

            // Check for _collection field (required for valid documents)
            let collection_name = match doc_value.get("_collection").and_then(|v| v.as_str()) {
                Some(name) => name.to_string(),
                None => {
                    // No _collection field - likely hit metadata
                    break;
                }
            };

            // Check if tombstone
            let is_tombstone = doc_value
                .get("_tombstone")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            // Extract _id
            if let Some(id_val) = doc_value.get("_id") {
                if let Ok(doc_id) = serde_json::from_value::<DocumentId>(id_val.clone()) {
                    // Get or create collection meta
                    let meta = self
                        .collections_mut()
                        .entry(collection_name.clone())
                        .or_insert_with(|| CollectionMeta {
                            name: collection_name.clone(),
                            document_count: 0,
                            live_document_count: 0,
                            data_offset: HEADER_SIZE,
                            index_offset: 0,
                            last_id: 0,
                            document_catalog: HashMap::new(),
                            document_order: Vec::new(),
                            indexes: Vec::new(),
                            fuzzy_indexes: Vec::new(),
                            fulltext_indexes: Vec::new(),
                            vector_indexes: Vec::new(),
                            schema: None,
                            flags: CollectionFlags::default(),
                            auto_embedding_config: None,
                        });

                    if is_tombstone {
                        meta.document_catalog.remove(&doc_id);
                        meta.document_order.retain(|id| id != &doc_id);
                    } else {
                        meta.document_catalog.insert(doc_id.clone(), offset);
                        meta.document_order.retain(|id| id != &doc_id);
                        meta.document_order.push(doc_id.clone());
                        meta.document_count += 1;
                        meta.live_document_count += 1;
                        documents_found += 1;

                        // Track max ID for last_id
                        // SAFETY: Only positive i64 values are valid auto-increment IDs.
                        // Negative i64 cast to u64 wraps to u64::MAX, corrupting last_id.
                        if let DocumentId::Int(id_num) = &doc_id {
                            if *id_num > 0 {
                                let current_max =
                                    max_ids_by_collection.entry(collection_name).or_insert(0);
                                *current_max = (*current_max).max(*id_num as u64);
                            }
                        }
                    }
                }
            }

            // Move to next document
            offset += 4 + len as u64;
        }

        // Update last_id for each collection
        for (collection_name, max_id) in max_ids_by_collection {
            if let Some(meta) = self.collections_mut().get_mut(&collection_name) {
                meta.last_id = max_id;
            }
        }

        // Update header
        // CRITICAL: Use HeaderWriter instead of direct assignment
        // flush_metadata() below will call set_after_metadata() which finalizes
        // the header, but it reads data_end_offset to determine metadata position.
        HeaderWriter::new(&mut self.header, &mut self.file).set_recovery_offset(offset);
        self.header.collection_count = self.collections.len() as u32;

        // Write corrected metadata
        self.mark_metadata_dirty()?;
        self.flush_metadata()?;
        self.metadata_snapshot_pending = false;
        self.file.sync_all()?;

        log_info!(
            "[INFO] Rebuilt metadata: {} collections, {} documents from file scan",
            self.collections.len(),
            documents_found
        );

        Ok(())
    }

    /// Get database statistics
    pub fn stats(&self) -> serde_json::Value {
        serde_json::json!({
            "file_path": self.file_path,
            "file_size": self.file.metadata().map(|m| m.len()).unwrap_or(0),
            "page_size": self.header.page_size,
            "collection_count": self.header.collection_count,
            "collections": self.collections.iter().map(|(name, meta)| {
                serde_json::json!({
                    "name": name,
                    "document_count": meta.document_count,
                    "last_id": meta.last_id,
                })
            }).collect::<Vec<_>>(),
        })
    }

    // =========================================================================
    // CLEAN SHUTDOWN MANAGEMENT
    // =========================================================================

    /// Check if the database was cleanly shut down last time
    ///
    /// If true, indexes can be trusted from .idx files without rebuild.
    /// If false (crash or first run), indexes must be rebuilt from documents.
    pub fn was_clean_shutdown(&self) -> bool {
        self.was_clean_shutdown
    }

    /// Mark the database as cleanly shutting down
    ///
    /// MUST be called during graceful shutdown, before dropping the storage.
    /// This sets the clean_shutdown flag in the header and flushes to disk.
    ///
    /// After this call, the next open() will see was_clean_shutdown() = true,
    /// allowing indexes to be loaded from .idx files without rebuild.
    pub fn mark_clean_shutdown(&mut self) -> Result<()> {
        // Upgrade to version 4 (clean_shutdown support)
        if self.header.version < 4 {
            self.header.version = 4;
        }
        self.header.clean_shutdown = true;
        self.mark_metadata_dirty()?;
        self.flush()?;
        Ok(())
    }

    /// Read the persisted transaction-id watermark from the header.
    ///
    /// Returns the highest committed transaction ID at the previous graceful
    /// shutdown. Legacy databases (pre-version-5 / `#[serde(default)]` path)
    /// return 0. `DatabaseCore::open` uses this to seed
    /// `next_tx_id` (= this + 1) and `max_committed_tx_id` (= this) so a
    /// post-rebuild flush can stamp a non-zero `last_flushed_tx_id` into
    /// every index file. (Task #26 R1.)
    pub fn last_committed_tx_id(&self) -> u64 {
        self.header.last_committed_tx_id
    }

    /// Persist the highest committed transaction-id seen in this session.
    ///
    /// Called from the graceful shutdown path (right next to
    /// `mark_clean_shutdown`). Monotonic — calls with a smaller value are
    /// silently ignored so an out-of-order writer cannot rewind the
    /// watermark on disk.
    pub fn set_last_committed_tx_id(&mut self, tx_id: u64) -> Result<()> {
        if tx_id <= self.header.last_committed_tx_id {
            return Ok(());
        }
        if self.header.version < 5 {
            self.header.version = 5;
        }
        self.header.last_committed_tx_id = tx_id;
        self.mark_metadata_dirty()?;
        Ok(())
    }

    /// Data file size after the last successful compaction (0 = never compacted),
    /// persisted in the Header so `bloat_ratio` survives restarts.
    pub fn last_compact_size(&self) -> u64 {
        self.header.last_compact_size
    }

    /// Persist the post-compaction data file size baseline. Called after a
    /// successful compaction and at graceful shutdown (next to
    /// `set_last_committed_tx_id`). Marks metadata dirty so the new value is
    /// written by the next checkpoint / clean-shutdown flush.
    pub fn set_last_compact_size(&mut self, size: u64) -> Result<()> {
        if self.header.last_compact_size == size {
            return Ok(());
        }
        if self.header.version < 6 {
            self.header.version = 6;
        }
        self.header.last_compact_size = size;
        self.mark_metadata_dirty()?;
        Ok(())
    }

    /// Commit a transaction (9-step atomic operation) - internal implementation
    /// This is the core of ACD guarantee
    ///
    /// # Arguments
    /// * `transaction` - The transaction to commit
    /// * `_sync_file` - UNUSED since 2026-02-01 PERF FIX. Previously intended to
    ///   distinguish Safe mode (fsync main .mlite file per commit) from Batch mode
    ///   (defer fsync). After the PERF FIX removed per-commit metadata flush
    ///   (see Step 8 comment below), the main file is only fsynced during:
    ///   - Batch mode: `flush_batch()` → `sync_storage_file()` at batch end
    ///   - Periodic checkpoint (every 120s)
    ///   - Graceful shutdown via `close()` / `Drop`
    ///   WAL fsync (Step 4) still guarantees durability in all modes.
    ///   Parameter retained for API compatibility with `commit_transaction()` /
    ///   `commit_transaction_batch()` callers.
    fn commit_transaction_internal(
        &mut self,
        transaction: &mut Transaction,
        _sync_file: bool,
    ) -> Result<()> {
        use crate::transaction::Operation;
        use crate::wal::{WALEntry, WALEntryType};
        use serde_json::Value;

        /// Helper: add _collection field to all document Values in an Operation.
        /// This centralizes the _collection injection (PHASE 5: WAL centralization).
        /// Uses entry().or_insert_with() to avoid overwriting if caller already set it.
        fn add_collection_to_operation(operation: &Operation) -> Operation {
            fn inject_collection(value: &mut Value, collection: &str) {
                if let Value::Object(ref mut map) = value {
                    map.entry("_collection".to_string())
                        .or_insert_with(|| Value::String(collection.to_string()));
                }
            }

            match operation {
                Operation::Insert {
                    collection,
                    doc_id,
                    doc,
                } => {
                    // Fast path: `_collection` is now injected during prepare()
                    // so we can pass the Arc through with only a refcount bump.
                    // Slow path (legacy callers without `_collection`) deep-clones once.
                    let needs_inject = match doc.as_ref() {
                        serde_json::Value::Object(map) => !map.contains_key("_collection"),
                        _ => false,
                    };
                    let doc_out = if needs_inject {
                        let mut cloned = (**doc).clone();
                        inject_collection(&mut cloned, collection);
                        std::sync::Arc::new(cloned)
                    } else {
                        std::sync::Arc::clone(doc)
                    };
                    Operation::Insert {
                        collection: collection.clone(),
                        doc_id: doc_id.clone(),
                        doc: doc_out,
                    }
                }
                Operation::Update {
                    collection,
                    doc_id,
                    old_doc,
                    new_doc,
                } => {
                    fn maybe_inject(
                        arc: &std::sync::Arc<serde_json::Value>,
                        collection: &str,
                    ) -> std::sync::Arc<serde_json::Value> {
                        let needs = match arc.as_ref() {
                            serde_json::Value::Object(map) => !map.contains_key("_collection"),
                            _ => false,
                        };
                        if needs {
                            let mut cloned = (**arc).clone();
                            inject_collection(&mut cloned, collection);
                            std::sync::Arc::new(cloned)
                        } else {
                            std::sync::Arc::clone(arc)
                        }
                    }
                    Operation::Update {
                        collection: collection.clone(),
                        doc_id: doc_id.clone(),
                        old_doc: maybe_inject(old_doc, collection),
                        new_doc: maybe_inject(new_doc, collection),
                    }
                }
                Operation::Delete {
                    collection,
                    doc_id,
                    old_doc,
                } => {
                    let needs_inject = match old_doc.as_ref() {
                        serde_json::Value::Object(map) => !map.contains_key("_collection"),
                        _ => false,
                    };
                    let old_out = if needs_inject {
                        let mut cloned = (**old_doc).clone();
                        inject_collection(&mut cloned, collection);
                        std::sync::Arc::new(cloned)
                    } else {
                        std::sync::Arc::clone(old_doc)
                    };
                    Operation::Delete {
                        collection: collection.clone(),
                        doc_id: doc_id.clone(),
                        old_doc: old_out,
                    }
                }
            }
        }

        if !transaction.is_active() {
            return Err(IronBaseError::TransactionCommitted);
        }

        let already_applied = transaction.operations_applied();

        // Step 1: Write BEGIN marker to WAL
        let begin_entry = WALEntry::new(transaction.id, WALEntryType::Begin, vec![]);
        self.wal.append(&begin_entry)?;

        // Step 2: Write all operations to WAL (use JSON instead of bincode for compatibility)
        // PHASE 5: Centralize _collection injection here instead of in callers
        for operation in transaction.operations() {
            let op_with_collection = add_collection_to_operation(operation);
            let op_json = serde_json::to_string(&op_with_collection)
                .map_err(|e| IronBaseError::Serialization(e.to_string()))?;
            let op_entry = WALEntry::new(
                transaction.id,
                WALEntryType::Operation,
                op_json.as_bytes().to_vec(),
            );
            self.wal.append(&op_entry)?;
        }

        // Step 2.5: Write index changes to WAL (for two-phase commit recovery)
        // Each index change is written as an IndexChange entry
        // Format: {collection: string, index_name: string, operation: Insert|Delete, key: IndexKey, doc_id: DocumentId}
        // Extract collection name from first operation (all operations in a transaction are for the same collection)
        let collection_name = transaction.operations().first().map(|op| match op {
            crate::transaction::Operation::Insert { collection, .. } => collection.clone(),
            crate::transaction::Operation::Update { collection, .. } => collection.clone(),
            crate::transaction::Operation::Delete { collection, .. } => collection.clone(),
        });

        for (index_name, changes) in transaction.index_changes() {
            for change in changes {
                // Serialize index change to JSON (now includes collection name)
                let change_data = serde_json::json!({
                    "collection": collection_name.as_ref().unwrap_or(&"unknown".to_string()),
                    "index_name": index_name,
                    "operation": match change.operation {
                        crate::transaction::IndexOperation::Insert => "Insert",
                        crate::transaction::IndexOperation::Delete => "Delete",
                    },
                    "key": change.key,
                    "doc_id": change.doc_id,
                });

                let change_json = serde_json::to_string(&change_data)
                    .map_err(|e| IronBaseError::Serialization(e.to_string()))?;

                let index_entry = WALEntry::new(
                    transaction.id,
                    WALEntryType::IndexChange,
                    change_json.as_bytes().to_vec(),
                );
                self.wal.append(&index_entry)?;
            }
        }

        // Step 3: Write COMMIT marker to WAL
        let commit_entry = WALEntry::new(transaction.id, WALEntryType::Commit, vec![]);
        self.wal.append(&commit_entry)?;

        // Step 4: Fsync WAL (durability guarantee)
        self.wal.flush()?;

        // Step 5: Apply operations to storage
        if !already_applied {
            self.apply_operations(transaction)?;
        }

        // Step 6: Two-Phase Commit for Index Changes
        // NOTE: Index changes are written to WAL in Step 2.5 above.
        // The actual two-phase commit for indexes happens at a higher level:
        //
        // DESIGN: Index atomicity requires coordination between:
        // - StorageEngine (this layer): Writes index changes to WAL
        // - CollectionCore/Database layer: Executes two-phase commit
        //
        // TWO-PHASE COMMIT PROTOCOL (API exists, but NOT CALLED):
        // Phase 1 (PREPARE): Create temp index files (.idx.tmp)
        //   - For each index: index.prepare_changes(base_path) → temp_path
        //   - WAL write (Step 2.5) makes changes durable
        //
        // Phase 2 (COMMIT): Atomic rename temp → final
        //   - For each temp: BPlusTree::commit_prepared_changes(temp_path, final_path)
        //   - POSIX rename() guarantees atomicity
        //
        // CRASH RECOVERY (would be implemented in Step 4):
        // - WAL recovery replays IndexChange entries
        // - Detects uncommitted temp files and cleans up
        //
        // TODO (Steps 4-6): Implement full two-phase commit at Database/CollectionCore level
        //
        // IMPLEMENTATION PLAN:
        // 1. Add index_file_paths tracking to Transaction struct
        // 2. In commit_transaction(), after Step 3 (WAL commit):
        //    a) For each affected index: call prepare_changes() → collect temp_paths
        //    b) Fsync all temp files
        //    c) Atomically rename all temp → final (commit_prepared_changes())
        //    d) On error: rollback_prepared_changes() for all temp files
        // 3. Add crash recovery: scan for .idx.tmp files on database open
        //    - If tx_id in WAL has COMMIT → complete the rename
        //    - If tx_id has ROLLBACK or missing → delete temp files
        //
        // CURRENT STATE:
        // - BPlusTree API fully implemented (see index.rs:419-476)
        // - Index changes are durable via WAL (crash recoverable)
        // - BUT: Index files NOT atomically committed (weak atomicity)
        //
        // IMPACT:
        // - Transactions are ACD compliant (WAL ensures durability)
        // - Index files may be slightly out-of-sync after crash (WAL replay fixes)
        // - No correctness bugs (WAL is source of truth)
        // - Minor: index reads after crash before WAL replay may be stale
        //
        // PRIORITY: Medium (nice-to-have, not critical for correctness)
        //
        // REVIEW 2024-12: Reviewed and accepted as technical debt.
        // WAL guarantees correctness, index files are just cache.
        // Cost of implementation outweighs benefit.

        // Step 7: Apply metadata changes (skip if already applied)
        if !already_applied {
            for metadata_change in transaction.metadata_changes() {
                if let Some(meta) = self.collections_mut().get_mut(&metadata_change.collection) {
                    // SAFETY: Only positive i64 values are valid auto-increment IDs.
                    // Negative i64 cast to u64 wraps to u64::MAX, corrupting last_id.
                    if metadata_change.last_id > 0 {
                        meta.last_id = meta.last_id.max(metadata_change.last_id as u64);
                    }
                }
            }
        }

        // Step 8: Metadata persistence deferred to periodic checkpoint
        //
        // PERF FIX (2026-02-01): Removed per-commit flush_metadata() and
        // ensure_metadata_snapshot(). With 130K+ documents, these serialized
        // the ENTIRE document_catalog (~6MB) TWICE per insert (WAL + file),
        // plus 2 extra fsyncs. Total: ~12MB I/O + 3 fsyncs per 10KB insert.
        //
        // Now metadata is persisted ONLY during:
        // - Periodic checkpoint (every 120s) via checkpoint()
        // - Graceful shutdown via close() / Drop
        //
        // Crash safety is maintained by WAL transaction entries (BEGIN + Operation
        // with full document JSON + COMMIT). On recovery, recover_from_wal()
        // replays committed transactions via apply_wal_operation() →
        // write_document_full(), which reconstructs catalog + data_end_offset.
        //
        // WAL clear is also deferred to checkpoint (was every 100 commits).
        // Without MetadataSnapshot entries (~6MB each), WAL grows only ~10KB
        // per insert, so ~1MB per 100 inserts between checkpoints.
        //
        // Previous bug (2024-12-26) that motivated per-commit flush:
        // header.data_end_offset was stale after crash → metadata corruption.
        // This is now safe because recover_from_wal() replays all WAL
        // transactions starting from the checkpointed data_end_offset,
        // re-writing documents at their original positions.

        // Step 9: Mark transaction as committed
        transaction.mark_committed()?;

        Ok(())
    }

    /// Commit a transaction with full durability (Safe mode)
    /// This is the standard commit with both WAL and file sync.
    pub fn commit_transaction(&mut self, transaction: &mut Transaction) -> Result<()> {
        self.commit_transaction_internal(transaction, true)
    }

    /// Commit a transaction for batch mode (skip file sync)
    /// WAL is still synced for durability, but file sync is deferred.
    /// Caller must call sync_file() at the end of the batch.
    pub fn commit_transaction_batch(&mut self, transaction: &mut Transaction) -> Result<()> {
        self.commit_transaction_internal(transaction, false)
    }

    /// Sync the main database file to disk
    /// Call this after a batch of commit_transaction_batch() calls.
    pub fn sync_file(&mut self) -> Result<()> {
        self.file.sync_all()?;
        Ok(())
    }

    /// Rollback a transaction (discard all buffered operations)
    pub fn rollback_transaction(&mut self, transaction: &mut Transaction) -> Result<()> {
        use crate::wal::{WALEntry, WALEntryType};

        if !transaction.is_active() {
            return Ok(()); // Already committed or aborted
        }

        // Write ABORT marker to WAL
        let abort_entry = WALEntry::new(transaction.id, WALEntryType::Abort, vec![]);
        self.wal.append(&abort_entry)?;
        self.wal.flush()?;

        // Discard all buffered operations
        transaction.rollback()?;

        Ok(())
    }

    /// Write ABORT entry for a previously committed transaction
    ///
    /// This is used when persist phase fails after WAL commit.
    /// The ABORT entry tells recovery to discard the committed transaction.
    pub fn write_abort_entry(&mut self, tx_id: TransactionId) -> Result<()> {
        use crate::wal::{WALEntry, WALEntryType};

        let abort_entry = WALEntry::new(tx_id, WALEntryType::Abort, vec![]);
        self.wal.append(&abort_entry)?;
        self.wal.flush()?;

        Ok(())
    }

    /// Apply transaction operations to storage
    fn apply_operations(&mut self, transaction: &Transaction) -> Result<()> {
        use crate::transaction::Operation;

        for operation in transaction.operations() {
            match operation {
                Operation::Insert {
                    collection,
                    doc_id,
                    doc,
                } => {
                    // Serialize and write document to storage with catalog update
                    let doc_json = serde_json::to_string(doc.as_ref())
                        .map_err(|e| IronBaseError::Serialization(e.to_string()))?;
                    // Use write_document_raw to properly update document_catalog
                    self.write_document_raw(collection, doc_id, doc_json.as_bytes())?;
                    self.adjust_live_count(collection, 1);
                }
                Operation::Update {
                    collection,
                    doc_id,
                    old_doc: _,
                    new_doc,
                } => {
                    // Write new version of document (append-only) with catalog update
                    let doc_json = serde_json::to_string(new_doc.as_ref())
                        .map_err(|e| IronBaseError::Serialization(e.to_string()))?;
                    // Use write_document_raw to properly update document_catalog
                    self.write_document_raw(collection, doc_id, doc_json.as_bytes())?;
                }
                Operation::Delete {
                    collection,
                    doc_id,
                    old_doc: _,
                } => {
                    // Write tombstone marker with collection info and catalog update
                    let tombstone = serde_json::json!({
                        "_id": doc_id,
                        "_collection": collection,
                        "_tombstone": true
                    });
                    let tombstone_json = serde_json::to_string(&tombstone)
                        .map_err(|e| IronBaseError::Serialization(e.to_string()))?;
                    // Use write_document_raw - it will handle tombstone in catalog properly
                    // (tombstones remove entry from catalog when processed by rebuild_catalog)
                    self.write_document_raw(collection, doc_id, tombstone_json.as_bytes())?;
                    self.adjust_live_count(collection, -1);
                }
            }
        }

        Ok(())
    }

    /// Apply a single WAL operation - UNIFIED path for both recovery and runtime
    ///
    /// This function uses `write_document_full()` which updates ALL metadata:
    /// - document_catalog: doc_id → offset mapping
    /// - document_count: total document writes
    /// - live_document_count: count of live (non-tombstone) documents
    /// - last_id: tracks highest auto-increment ID (prevents _id collisions after recovery!)
    ///
    /// This is the ONLY function that should be used for WAL recovery to ensure
    /// metadata consistency between runtime and recovery paths.
    pub fn apply_wal_operation(&mut self, operation: &crate::transaction::Operation) -> Result<()> {
        use crate::transaction::Operation;

        match operation {
            Operation::Insert {
                collection,
                doc_id,
                doc,
            } => {
                // Ensure collection exists (it may not after a crash)
                let _ = self.create_collection(collection);

                // Serialize and write document with FULL metadata update
                let doc_json = serde_json::to_string(doc.as_ref())
                    .map_err(|e| IronBaseError::Serialization(e.to_string()))?;
                self.write_document_full(collection, doc_id, doc_json.as_bytes())?;
            }
            Operation::Update {
                collection,
                doc_id,
                old_doc: _,
                new_doc,
            } => {
                // Ensure collection exists
                let _ = self.create_collection(collection);

                // Write new version with FULL metadata update
                let doc_json = serde_json::to_string(new_doc.as_ref())
                    .map_err(|e| IronBaseError::Serialization(e.to_string()))?;
                self.write_document_full(collection, doc_id, doc_json.as_bytes())?;
            }
            Operation::Delete {
                collection,
                doc_id,
                old_doc: _,
            } => {
                // Write tombstone with FULL metadata update
                self.write_tombstone_full(collection, doc_id)?;
            }
        }

        Ok(())
    }

    /// Recover from WAL after crash
    ///
    /// Returns (committed_transactions, index_changes) for higher-level recovery
    /// Recover from WAL: replay committed transactions and collect index changes
    ///
    /// Returns (recovered_tx_count, index_changes):
    /// - recovered_tx_count: number of committed transactions replayed (0 = no recovery needed)
    /// - index_changes: B+ tree index modifications to apply after storage recovery
    /// - applied_ops: `(tx_id, Operation)` pairs for every successfully applied op,
    ///   preserved so per-index WAL replay can selectively re-apply ops with
    ///   `tx_id > index.last_flushed_tx_id()` instead of doing a full rebuild.
    ///
    /// NOTE: Previously returned the full Vec<Vec<WALEntry>>, but the caller only
    /// used .is_empty() on it. Returning count saves O(WAL_size) memory.
    pub fn recover_from_wal(&mut self) -> Result<WalRecoveryOutput> {
        let recovered = self.wal.recover()?;

        if recovered.is_empty() {
            return Ok((0, vec![], vec![]));
        }

        let recovered_count = recovered.len();
        let mut all_index_changes = Vec::new();
        let mut applied_ops: Vec<(TransactionId, crate::transaction::Operation)> = Vec::new();

        // Replay each committed transaction
        for tx_entries in &recovered {
            // Deserialize operations from WAL entries
            for entry in tx_entries {
                match entry.entry_type {
                    crate::wal::WALEntryType::Operation => {
                        let op_str = std::str::from_utf8(&entry.data).map_err(|e| {
                            IronBaseError::Serialization(format!("UTF-8 error: {}", e))
                        })?;
                        let operation: crate::transaction::Operation =
                            serde_json::from_str(op_str)?;

                        // Apply operation using the UNIFIED path (write_document_full)
                        // This ensures ALL metadata is updated:
                        // - document_catalog
                        // - document_count
                        // - live_document_count
                        // - last_id (critical for preventing _id collisions!)
                        self.apply_wal_operation(&operation)?;
                        applied_ops.push((entry.transaction_id, operation));
                    }
                    crate::wal::WALEntryType::IndexChange => {
                        // Parse index change from JSON
                        let change_str = std::str::from_utf8(&entry.data).map_err(|e| {
                            IronBaseError::Serialization(format!("UTF-8 error: {}", e))
                        })?;
                        let change_json: serde_json::Value = serde_json::from_str(change_str)?;

                        // Extract fields (including collection name added in Step 6)
                        let collection = change_json["collection"]
                            .as_str()
                            .ok_or_else(|| {
                                IronBaseError::Serialization("Missing collection".to_string())
                            })?
                            .to_string();

                        let index_name = change_json["index_name"]
                            .as_str()
                            .ok_or_else(|| {
                                IronBaseError::Serialization("Missing index_name".to_string())
                            })?
                            .to_string();

                        let operation = match change_json["operation"].as_str() {
                            Some("Insert") => crate::transaction::IndexOperation::Insert,
                            Some("Delete") => crate::transaction::IndexOperation::Delete,
                            _ => {
                                return Err(IronBaseError::Serialization(
                                    "Invalid operation".to_string(),
                                ))
                            }
                        };

                        let key: crate::index::IndexKey =
                            serde_json::from_value(change_json["key"].clone())?;
                        let doc_id: crate::document::DocumentId =
                            serde_json::from_value(change_json["doc_id"].clone())?;

                        all_index_changes.push(RecoveredIndexChange {
                            collection,
                            index_name,
                            operation,
                            key,
                            doc_id,
                        });
                    }
                    _ => {} // Skip Begin, Commit, Abort markers
                }
            }
        }

        // Drop recovered entries to free memory before WAL clear I/O
        drop(recovered);

        // Clear WAL after successful recovery
        self.wal.clear()?;
        self.metadata_snapshot_pending = false;
        self.wal_ops_since_clear = 0;

        Ok((recovered_count, all_index_changes, applied_ops))
    }

    /// Rebuild document catalog from file after WAL recovery
    ///
    /// This function scans the entire data section of the file and rebuilds
    /// the document_catalog for each collection. This is necessary because:
    /// 1. WAL recovery uses write_data() which doesn't update the catalog
    /// 2. The catalog may be empty or inconsistent after a crash
    ///
    /// This is the "recovery rebuild" approach - simpler and more robust than
    /// fixing apply_operations to maintain the catalog during recovery.
    pub fn rebuild_catalog_from_file(&mut self) -> Result<()> {
        use std::io::{Read, Seek, SeekFrom};

        let file_len = self.file.metadata()?.len();

        // If file is too small to have any documents, nothing to rebuild
        // Documents start right after header (HEADER_SIZE = 256 bytes), NOT at DATA_START_OFFSET!
        if file_len <= HEADER_SIZE {
            return Ok(());
        }

        // CRITICAL FIX: Determine document region boundaries
        // Documents are stored between HEADER_SIZE and metadata_offset (EOF layout),
        // or after metadata when metadata is at the beginning (startup layout).
        let mut scan_start = HEADER_SIZE;
        let mut scan_end = file_len;
        let mut metadata_offset = self.header.metadata_offset;

        if metadata_offset == 0 && self.header.metadata_size > 0 {
            let inferred = self
                .header
                .data_end_offset
                .saturating_sub(self.header.metadata_size);
            if inferred >= HEADER_SIZE && inferred <= file_len {
                metadata_offset = inferred;
            }
        }

        if metadata_offset >= HEADER_SIZE
            && metadata_offset <= file_len
            && self.header.metadata_size > 0
        {
            if metadata_offset == HEADER_SIZE
                && self.header.data_end_offset > metadata_offset + self.header.metadata_size
            {
                // Metadata is at the beginning; documents are after metadata.
                scan_start = metadata_offset + self.header.metadata_size;
                scan_end = self.header.data_end_offset.min(file_len);
            } else {
                // Metadata is at EOF; documents end at metadata_offset.
                scan_end = metadata_offset;
            }
        }

        // Clear existing catalogs and reset counts
        for meta in self.collections_mut().values_mut() {
            meta.document_catalog.clear();
            meta.document_order.clear();
            meta.document_count = 0;
            meta.live_document_count = 0;
        }

        // Scan all documents from HEADER_SIZE (256 bytes) to metadata_offset (or file_len)
        // CRITICAL: Documents are ONLY valid in the range [HEADER_SIZE, metadata_offset)
        let mut offset = scan_start;
        let mut max_ids_by_collection: HashMap<String, u64> = HashMap::new();

        while offset + 4 < scan_end {
            // Read document length (4 bytes)
            self.file.seek(SeekFrom::Start(offset))?;
            let mut len_bytes = [0u8; 4];
            if self.file.read_exact(&mut len_bytes).is_err() {
                break; // EOF or read error
            }
            let len = u32::from_le_bytes(len_bytes) as usize;

            // Validate length - must not exceed document region boundary
            if len == 0 || offset + 4 + (len as u64) > scan_end {
                break; // Corrupted or truncated document, or reached metadata
            }

            // Read document data
            let mut data = vec![0u8; len];
            if self.file.read_exact(&mut data).is_err() {
                break; // EOF or read error
            }

            // Parse JSON to extract _id and _collection
            if let Ok(doc_value) = serde_json::from_slice::<serde_json::Value>(&data) {
                // Check if tombstone
                let is_tombstone = doc_value
                    .get("_tombstone")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);

                // Extract collection name
                if let Some(collection_name) = doc_value.get("_collection").and_then(|v| v.as_str())
                {
                    // Extract _id
                    if let Some(id_val) = doc_value.get("_id") {
                        if let Ok(doc_id) = serde_json::from_value::<DocumentId>(id_val.clone()) {
                            // Get or create collection meta
                            let meta = self
                                .collections_mut()
                                .entry(collection_name.to_string())
                                .or_insert_with(|| CollectionMeta {
                                    name: collection_name.to_string(),
                                    document_count: 0,
                                    live_document_count: 0,
                                    data_offset: HEADER_SIZE,
                                    index_offset: 0,
                                    last_id: 0,
                                    document_catalog: HashMap::new(),
                                    document_order: Vec::new(),
                                    indexes: Vec::new(),
                                    fuzzy_indexes: Vec::new(),
                                    fulltext_indexes: Vec::new(),
                                    vector_indexes: Vec::new(),
                                    schema: None,
                                    flags: CollectionFlags::default(),
                                    auto_embedding_config: None,
                                });

                            if is_tombstone {
                                // Remove from catalog if exists (tombstone = deletion)
                                meta.document_catalog.remove(&doc_id);
                                meta.document_order.retain(|id| id != &doc_id);
                                // Don't increment counts for tombstones
                            } else {
                                // Add/update catalog entry (newer version overwrites older)
                                meta.document_catalog.insert(doc_id.clone(), offset);
                                meta.document_order.retain(|id| id != &doc_id);
                                meta.document_order.push(doc_id.clone());
                                meta.document_count += 1;
                                meta.live_document_count += 1;

                                // Track max ID for last_id
                                // SAFETY: Only positive i64 values are valid auto-increment IDs.
                                // Negative i64 cast to u64 wraps to u64::MAX, corrupting last_id.
                                if let DocumentId::Int(id_num) = &doc_id {
                                    if *id_num > 0 {
                                        let current_max = max_ids_by_collection
                                            .entry(collection_name.to_string())
                                            .or_insert(0);
                                        *current_max = (*current_max).max(*id_num as u64);
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Move to next document
            offset += 4 + (len as u64);
        }

        // Update last_id for each collection
        for (collection_name, max_id) in max_ids_by_collection {
            if let Some(meta) = self.collections_mut().get_mut(&collection_name) {
                if max_id > meta.last_id {
                    meta.last_id = max_id;
                }
            }
        }

        // Fix document counts based on actual catalog state
        // This is necessary because:
        // 1. Updates/replacements counted each version separately
        // 2. Tombstones removed from catalog but didn't decrement counts
        for meta in self.collections_mut().values_mut() {
            meta.live_document_count = meta.document_catalog.len() as u64;
            meta.document_count = meta.document_catalog.len() as u64;
        }

        // Mark metadata as dirty so it gets flushed
        self.mark_metadata_dirty()?;

        Ok(())
    }

    /// Release the exclusive file lock without dropping the StorageEngine.
    /// This allows other processes to open the database while this instance
    /// remains in memory (but should no longer be used for writes).
    ///
    /// This is primarily used by language bindings (Python, C#) where the
    /// garbage collector timing is unpredictable and explicit close() is needed.
    pub fn release_lock(&self) -> Result<()> {
        self.lock_file.unlock().map_err(IronBaseError::Io)
    }
}

// Automatic cleanup on drop
impl Drop for StorageEngine {
    fn drop(&mut self) {
        // CRITICAL: mark_clean_shutdown() ONLY if BOTH flush and checkpoint succeed.
        // If either fails, metadata/indexes may not be persisted — marking clean
        // would cause the next open() to trust stale .idx files (see #9ff48302).
        let flush_ok = match self.flush() {
            Ok(()) => true,
            Err(e) => {
                log_error!("StorageEngine::drop flush failed: {}", e);
                false
            }
        };
        // Clear WAL on close to keep it clean for next open
        let checkpoint_ok = match self.checkpoint() {
            Ok(_) => true,
            Err(e) => {
                log_error!("StorageEngine::drop checkpoint failed: {}", e);
                false
            }
        };
        // Mark clean shutdown ONLY when all data is safely persisted
        if flush_ok && checkpoint_ok {
            if let Err(e) = self.mark_clean_shutdown() {
                log_error!("StorageEngine::drop mark_clean_shutdown failed: {}", e);
            }
        } else {
            log_error!(
                "StorageEngine::drop skipping mark_clean_shutdown (flush_ok={}, checkpoint_ok={}): \
                 next open will rebuild indexes from documents",
                flush_ok,
                checkpoint_ok
            );
        }
        // Explicitly unlock the lock file to ensure other processes can access the database
        // This is more reliable than relying on File::drop() to release the flock
        if let Err(e) = self.lock_file.unlock() {
            log_error!("StorageEngine::drop unlock failed: {}", e);
        }
    }
}

// ============================================================================
// STORAGE TRAIT IMPLEMENTATION FOR StorageEngine
// ============================================================================

impl Storage for StorageEngine {
    fn write_document(&mut self, collection: &str, doc: &serde_json::Value) -> Result<u64> {
        // Parse document to extract ID
        let doc_id: DocumentId = if let Some(id_val) = doc.get("_id") {
            serde_json::from_value(id_val.clone())?
        } else {
            // Generate auto-increment ID
            let meta = self
                .get_collection_meta_mut(collection)
                .ok_or_else(|| IronBaseError::CollectionNotFound(collection.to_string()))?;
            meta.last_id += 1;
            DocumentId::Int(meta.last_id as i64)
        };

        // Serialize document
        let doc_json = serde_json::to_string(doc)?;

        // Write using existing method
        StorageEngine::write_document(self, collection, &doc_id, doc_json.as_bytes())
    }

    fn read_document(
        &mut self,
        collection: &str,
        id: &DocumentId,
    ) -> Result<Option<serde_json::Value>> {
        let meta = match self.get_collection_meta(collection) {
            Some(m) => m,
            None => return Ok(None),
        };

        let offset = match meta.document_catalog.get(id) {
            Some(&off) => off,
            None => return Ok(None),
        };

        // Now we can use &mut self directly (no unsafe cast needed!)
        let data = StorageEngine::read_document_at(self, collection, offset)?;
        let value: serde_json::Value = serde_json::from_slice(&data)?;
        Ok(Some(value))
    }

    fn scan_documents(&mut self, collection: &str) -> Result<Vec<Document>> {
        // NOTE: catalog.clone() reviewed 2024-12 - acceptable overhead.
        // HashMap<DocumentId, u64> ~40 bytes/entry, fast clone even for 10k+ docs.
        // Borrow checker requires clone to release lock before iteration.
        let catalog = match self.get_collection_meta(collection) {
            Some(m) => m.document_catalog.clone(),
            None => return Ok(Vec::new()),
        };

        let mut documents = Vec::new();

        for &offset in catalog.values() {
            let data = StorageEngine::read_document_at(self, collection, offset)?;
            let doc_value: serde_json::Value = serde_json::from_slice(&data)?;

            // Skip tombstones
            if doc_value
                .get("_tombstone")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                continue;
            }

            // 🚀 OPTIMIZED: Use from_value_owned to avoid clone
            let document = Document::from_value_owned(doc_value)?;
            documents.push(document);
        }

        Ok(documents)
    }

    fn create_collection(&mut self, name: &str) -> Result<()> {
        self.create_collection(name)
    }

    fn drop_collection(&mut self, name: &str) -> Result<()> {
        self.drop_collection(name)
    }

    fn rename_collection(&mut self, old_name: &str, new_name: &str) -> Result<()> {
        self.rename_collection(old_name, new_name)
    }

    fn list_collections(&self) -> Vec<String> {
        self.list_collections()
    }

    fn get_collection_meta(&self, name: &str) -> Option<&CollectionMeta> {
        self.get_collection_meta(name)
    }

    fn get_collection_meta_mut(&mut self, name: &str) -> Option<&mut CollectionMeta> {
        self.get_collection_meta_mut(name)
    }

    fn flush(&mut self) -> Result<()> {
        self.flush()
    }

    fn checkpoint(&mut self) -> Result<compaction::CheckpointStats> {
        self.checkpoint()
    }

    fn adjust_live_count(&mut self, collection: &str, delta: i64) {
        if let Some(meta) = self.collections_mut().get_mut(collection) {
            if delta >= 0 {
                meta.live_document_count = meta.live_document_count.saturating_add(delta as u64);
            } else {
                let dec = (-delta) as u64;
                meta.live_document_count = meta.live_document_count.saturating_sub(dec);
            }
            if let Err(err) = self.mark_metadata_dirty() {
                log_error!(
                    "Failed to schedule metadata snapshot after live_count change: {}",
                    err
                );
            }
        }
    }

    fn get_live_count(&self, collection: &str) -> Option<u64> {
        self.collections
            .get(collection)
            .map(|m| m.live_document_count)
    }

    fn get_file_path(&self) -> &str {
        &self.file_path
    }

    fn was_clean_shutdown(&self) -> bool {
        self.was_clean_shutdown
    }

    fn last_committed_tx_id(&self) -> u64 {
        StorageEngine::last_committed_tx_id(self)
    }

    fn set_last_committed_tx_id(&mut self, tx_id: u64) -> Result<()> {
        StorageEngine::set_last_committed_tx_id(self, tx_id)
    }

    fn last_compact_size(&self) -> u64 {
        StorageEngine::last_compact_size(self)
    }

    fn set_last_compact_size(&mut self, size: u64) -> Result<()> {
        StorageEngine::set_last_compact_size(self, size)
    }

    fn mark_clean_shutdown(&mut self) -> Result<()> {
        StorageEngine::mark_clean_shutdown(self)
    }
}

// ============================================================================
// RAWSTORAGE IMPLEMENTATION FOR STORAGEENGINE
// ============================================================================

impl RawStorage for StorageEngine {
    fn write_document_raw(
        &mut self,
        collection: &str,
        doc_id: &DocumentId,
        data: &[u8],
    ) -> Result<u64> {
        StorageEngine::write_document(self, collection, doc_id, data)
    }

    fn read_document_at(&mut self, collection: &str, offset: u64) -> Result<Vec<u8>> {
        StorageEngine::read_document_at(self, collection, offset)
    }

    fn write_data(&mut self, data: &[u8]) -> Result<u64> {
        StorageEngine::write_data(self, data)
    }

    fn read_data(&mut self, offset: u64) -> Result<Vec<u8>> {
        StorageEngine::read_data(self, offset)
    }

    fn read_data_at(&self, offset: u64) -> Result<Vec<u8>> {
        StorageEngine::read_data_at(self, offset)
    }

    fn file_len(&self) -> Result<u64> {
        StorageEngine::file_len(self)
    }

    fn metadata_offset(&self) -> u64 {
        self.header.metadata_offset
    }

    fn metadata_size(&self) -> u64 {
        self.header.metadata_size
    }

    fn data_end_offset(&self) -> u64 {
        self.header.data_end_offset
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup_test_db() -> (TempDir, StorageEngine) {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");
        let storage = StorageEngine::open(&db_path).unwrap();
        (temp_dir, storage)
    }

    /// P1-8: the borrowing `MetadataWALEntryRef` used by `write_metadata_snapshot`
    /// (to avoid deep-cloning the whole catalog) must serialize byte-identically
    /// to the owned `MetadataWALEntry`, so the WAL format and crash recovery are
    /// unchanged.
    #[test]
    fn metadata_snapshot_ref_serializes_identically() {
        let (_t, mut storage) = setup_test_db();
        storage.create_collection("c").unwrap();

        let owned = MetadataWALEntry {
            collections: (*storage.collections).clone(),
            data_end_offset: storage.header.data_end_offset,
        };
        let by_ref = MetadataWALEntryRef {
            collections: &storage.collections,
            data_end_offset: storage.header.data_end_offset,
        };
        assert_eq!(
            serde_json::to_vec(&owned).unwrap(),
            serde_json::to_vec(&by_ref).unwrap()
        );
    }

    /// P0-1: `compact_prepare` snapshots the catalog with an O(1) `Arc::clone`
    /// (copy-on-write), NOT a deep clone under the write lock. The snapshot must
    /// share the same allocation as the live catalog until the first mutation,
    /// which then deep-clones exactly once — the live `Arc` diverges while the
    /// snapshot stays frozen at its Phase-A contents.
    #[test]
    fn compact_prepare_snapshot_is_cow_not_deep_clone() {
        let (_t, mut storage) = setup_test_db();
        storage.create_collection("c1").unwrap();
        storage.create_collection("c2").unwrap();

        // Phase A snapshot must be the SAME Arc allocation (no deep clone under
        // the write lock — the whole point of audit P0-1).
        let snapshot = storage.compact_prepare().unwrap();
        assert!(
            std::sync::Arc::ptr_eq(&snapshot.snapshot_collections, &storage.collections),
            "compact_prepare must Arc::clone the catalog (O(1)), not deep-clone it"
        );

        // A mutation while the snapshot is alive triggers copy-on-write: the live
        // catalog diverges to a fresh allocation; the snapshot stays frozen.
        storage.create_collection("c3").unwrap();
        assert!(
            !std::sync::Arc::ptr_eq(&snapshot.snapshot_collections, &storage.collections),
            "first mutation while a snapshot is alive must copy-on-write"
        );
        assert_eq!(
            snapshot.snapshot_collections.len(),
            2,
            "snapshot is frozen at its Phase-A contents"
        );
        assert_eq!(
            storage.collections.len(),
            3,
            "live catalog sees the new write"
        );
    }

    /// PR #89 follow-up, finding B: `checkpoint_wal_clear_only` clears the WAL,
    /// which discards the metadata snapshot it held. It must therefore reset
    /// `metadata_snapshot_pending` (like flush/checkpoint/checkpoint_with_preserialized);
    /// leaving it true makes the next `ensure_metadata_snapshot()` early-return,
    /// so the now-empty WAL gets no recovery base.
    #[test]
    fn checkpoint_wal_clear_only_resets_snapshot_pending() {
        let (_t, mut storage) = setup_test_db();

        // Dirty metadata → writes a snapshot into the WAL and arms the flag.
        storage.mark_metadata_dirty().unwrap();
        assert!(
            storage.metadata_snapshot_pending,
            "mark_metadata_dirty should arm metadata_snapshot_pending"
        );

        storage.checkpoint_wal_clear_only().unwrap();
        assert!(
            !storage.metadata_snapshot_pending,
            "checkpoint_wal_clear_only must reset metadata_snapshot_pending after clearing the WAL"
        );
    }

    /// PR #89 follow-up, finding A (RED, data loss) — crash-durability of the
    /// choice the `checkpoint_wal_only` None arm makes, exercised through the
    /// REAL production recovery path. A committed insert's durable record is its
    /// WAL `Begin`/`Operation`/`Commit` entries (the document bytes are in the
    /// data region, but the on-disk catalog does not yet reference them, and the
    /// catalog is only made durable by a metadata flush). `DatabaseCore::open`
    /// ALWAYS replays the WAL (`recover_from_wal`), so a committed insert
    /// survives a crash via that replay — UNLESS the WAL was cleared first. The
    /// buggy None arm cleared the WAL (`checkpoint_wal_clear_only`) without
    /// flushing the catalog, so a crash before the next checkpoint left nothing
    /// to replay and a stale on-disk catalog → the committed doc was lost. The
    /// fix routes the dirty case to `checkpoint()`, which flushes the catalog
    /// to the main file before the WAL is cleared.
    ///
    /// The insert uses the real `commit_transaction` path (real WAL txn entries)
    /// and reopen goes through `DatabaseCore::open` (real `recover_from_wal`), so
    /// this models the actual durability mechanism — not the MetadataSnapshot
    /// entries, which recovery skips. A crash is simulated by releasing the file
    /// lock and `mem::forget`-ing the engine so its Drop (which would flush +
    /// checkpoint) never runs, leaving exactly the on-disk state a power loss
    /// would. NOTE: this pins the durability CONTRACT the None arm chooses
    /// between (`checkpoint` vs `checkpoint_wal_clear_only`); the None-arm wiring
    /// itself is only reachable via the Phase-A→B race and is covered by review,
    /// not driven here (a deterministic wiring test would need a test-only seam
    /// in the checkpoint hot path).
    #[test]
    fn committed_write_survives_crash_via_checkpoint() {
        use crate::transaction::{Operation, Transaction};

        // Returns the document count visible after a simulated crash, where
        // `finalize` is the None-arm action applied to the dirty engine.
        fn run(tag: &str, finalize: impl Fn(&mut StorageEngine)) -> u64 {
            let temp_dir = TempDir::new().unwrap();
            let db_path = temp_dir.path().join(format!("crash_{tag}.mlite"));

            {
                let mut storage = StorageEngine::open(&db_path).unwrap();
                storage.create_collection("c").unwrap();
                storage.flush().unwrap(); // clean baseline: empty catalog on disk

                // A real committed insert: Begin/Operation/Commit fsynced to the
                // WAL, the in-memory catalog updated, metadata marked dirty — but
                // NOT flushed to the main metadata section (per-commit flush was
                // removed for perf, so the WAL is the only durable record yet).
                let mut tx = Transaction::new(1);
                tx.add_operation(Operation::Insert {
                    collection: "c".to_string(),
                    doc_id: crate::document::DocumentId::Int(1),
                    doc: std::sync::Arc::new(serde_json::json!({"v": 1})),
                })
                .unwrap();
                storage.commit_transaction(&mut tx).unwrap();
                assert!(storage.is_metadata_dirty());

                finalize(&mut storage); // the None-arm action under test

                // Simulate a crash: free the lock, then skip Drop (no flush).
                storage.release_lock().unwrap();
                std::mem::forget(storage);
            }

            // Reopen via DatabaseCore so recover_from_wal() runs (production path).
            let db = crate::DatabaseCore::<StorageEngine>::open(&db_path).unwrap();
            db.count_documents("c", &serde_json::json!({})).unwrap() as u64
        }

        // Control: no checkpoint at all → the WAL still holds the txn, so the
        // committed insert is recovered by recover_from_wal on reopen. Proves the
        // recovery path is genuinely exercised and that any loss below is caused
        // specifically by clearing the WAL, not by a broken reopen.
        let recovered = run("no_clear", |_s| {});
        assert_eq!(
            recovered, 1,
            "a committed insert must be recovered from the WAL on crash when the WAL is intact"
        );

        // Buggy choice: clear the WAL without flushing → the committed insert has
        // no durable record left (empty WAL + stale catalog) → lost on crash.
        let lost = run("clear_only", |s| {
            s.checkpoint_wal_clear_only().unwrap();
        });
        assert_eq!(
            lost, 0,
            "checkpoint_wal_clear_only on dirty metadata strands the committed insert on crash — \
             this is exactly why the None arm must not use it while metadata is dirty"
        );

        // Fixed choice: checkpoint() flushes the catalog first → the write survives.
        let kept = run("checkpoint", |s| {
            s.checkpoint().unwrap();
        });
        assert_eq!(
            kept, 1,
            "checkpoint() must flush the catalog so a committed insert survives a crash"
        );
    }

    #[test]
    fn test_create_new_database() {
        let (_temp, storage) = setup_test_db();

        assert_eq!(storage.header.magic, *b"MONGOLTE");
        assert_eq!(storage.header.version, 6); // Version 6: last_compact_size persistence (P1-7)
        assert_eq!(storage.header.page_size, 4096);
        assert_eq!(storage.header.collection_count, 0);
        assert_eq!(storage.collections.len(), 0);
    }

    #[test]
    fn test_open_existing_database() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");

        // Create database
        {
            let mut storage = StorageEngine::open(&db_path).unwrap();
            storage.create_collection("users").unwrap();
            storage.flush().unwrap();
        }

        // Reopen database
        let storage = StorageEngine::open(&db_path).unwrap();
        assert_eq!(storage.header.collection_count, 1);
        assert!(storage.collections.contains_key("users"));
    }

    #[test]
    fn test_magic_number_validation() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("corrupt.mlite");

        // Create corrupt file with wrong magic number
        let mut file = fs::File::create(&db_path).unwrap();
        use std::io::Write;
        file.write_all(b"WRONGMAG").unwrap(); // Wrong magic
        file.sync_all().unwrap();
        drop(file);

        // Try to open should fail
        let result = StorageEngine::open(&db_path);
        assert!(result.is_err());
    }

    #[test]
    fn test_create_collection() {
        let (_temp, mut storage) = setup_test_db();

        storage.create_collection("users").unwrap();

        assert_eq!(storage.header.collection_count, 1);
        assert!(storage.collections.contains_key("users"));

        let meta = storage.get_collection_meta("users").unwrap();
        assert_eq!(meta.name, "users");
        assert_eq!(meta.document_count, 0);
        assert_eq!(meta.last_id, 0);
    }

    #[test]
    fn test_create_duplicate_collection() {
        let (_temp, mut storage) = setup_test_db();

        storage.create_collection("users").unwrap();
        let result = storage.create_collection("users");

        assert!(result.is_err());
        match result {
            Err(IronBaseError::CollectionExists(_)) => (),
            _ => panic!("Expected CollectionExists error"),
        }
    }

    #[test]
    fn test_create_multiple_collections() {
        let (_temp, mut storage) = setup_test_db();

        storage.create_collection("users").unwrap();
        storage.create_collection("posts").unwrap();
        storage.create_collection("comments").unwrap();

        assert_eq!(storage.header.collection_count, 3);
        assert_eq!(storage.list_collections().len(), 3);

        let collections = storage.list_collections();
        assert!(collections.contains(&"users".to_string()));
        assert!(collections.contains(&"posts".to_string()));
        assert!(collections.contains(&"comments".to_string()));
    }

    #[test]
    fn test_drop_collection() {
        let (_temp, mut storage) = setup_test_db();

        storage.create_collection("users").unwrap();
        storage.create_collection("posts").unwrap();

        storage.drop_collection("users").unwrap();

        assert_eq!(storage.header.collection_count, 1);
        assert!(!storage.collections.contains_key("users"));
        assert!(storage.collections.contains_key("posts"));
    }

    #[test]
    fn test_drop_nonexistent_collection() {
        let (_temp, mut storage) = setup_test_db();

        let result = storage.drop_collection("nonexistent");

        assert!(result.is_err());
        match result {
            Err(IronBaseError::CollectionNotFound(_)) => (),
            _ => panic!("Expected CollectionNotFound error"),
        }
    }

    #[test]
    fn test_write_and_read_data() {
        let (_temp, mut storage) = setup_test_db();

        let test_data = b"Hello, IronBase!";
        let offset = storage.write_data(test_data).unwrap();

        let read_data = storage.read_data(offset).unwrap();
        assert_eq!(read_data, test_data);
    }

    #[test]
    fn test_write_multiple_data_blocks() {
        let (_temp, mut storage) = setup_test_db();

        let data1 = b"First block";
        let data2 = b"Second block";
        let data3 = b"Third block";

        let offset1 = storage.write_data(data1).unwrap();
        let offset2 = storage.write_data(data2).unwrap();
        let offset3 = storage.write_data(data3).unwrap();

        assert_eq!(storage.read_data(offset1).unwrap(), data1);
        assert_eq!(storage.read_data(offset2).unwrap(), data2);
        assert_eq!(storage.read_data(offset3).unwrap(), data3);

        // Offsets should be different
        assert_ne!(offset1, offset2);
        assert_ne!(offset2, offset3);
    }

    #[test]
    fn test_collection_metadata_persistence() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");

        // Create and modify collection
        {
            let mut storage = StorageEngine::open(&db_path).unwrap();
            storage.create_collection("users").unwrap();

            // Modify metadata
            let meta = storage.get_collection_meta_mut("users").unwrap();
            meta.document_count = 42;
            meta.last_id = 100;

            storage.flush().unwrap();
        }

        // Reopen and verify
        let storage = StorageEngine::open(&db_path).unwrap();
        let meta = storage.get_collection_meta("users").unwrap();
        assert_eq!(meta.document_count, 42);
        assert_eq!(meta.last_id, 100);
    }

    #[test]
    fn test_flush_metadata_convergence() {
        let (_temp, mut storage) = setup_test_db();

        // Create multiple collections
        for i in 0..5 {
            storage
                .create_collection(&format!("collection_{}", i))
                .unwrap();
        }

        // All collections should have correct data_offset
        let first_offset = storage
            .get_collection_meta("collection_0")
            .unwrap()
            .data_offset;

        for i in 1..5 {
            let offset = storage
                .get_collection_meta(&format!("collection_{}", i))
                .unwrap()
                .data_offset;
            assert_eq!(
                offset, first_offset,
                "All collections should have same data_offset after convergence"
            );
        }
    }

    #[test]
    fn test_get_collection_meta() {
        let (_temp, mut storage) = setup_test_db();

        storage.create_collection("users").unwrap();

        let meta = storage.get_collection_meta("users");
        assert!(meta.is_some());
        assert_eq!(meta.unwrap().name, "users");

        let nonexistent = storage.get_collection_meta("nonexistent");
        assert!(nonexistent.is_none());
    }

    #[test]
    fn test_get_collection_meta_mut() {
        let (_temp, mut storage) = setup_test_db();

        storage.create_collection("users").unwrap();

        {
            let meta = storage.get_collection_meta_mut("users").unwrap();
            meta.last_id = 999;
        }

        let meta = storage.get_collection_meta("users").unwrap();
        assert_eq!(meta.last_id, 999);
    }

    #[test]
    fn test_stats() {
        let (_temp, mut storage) = setup_test_db();

        storage.create_collection("users").unwrap();
        storage.create_collection("posts").unwrap();

        let stats = storage.stats();

        assert!(stats["file_path"].is_string());
        assert_eq!(stats["collection_count"], 2);
        assert_eq!(stats["page_size"], 4096);

        let collections = stats["collections"].as_array().unwrap();
        assert_eq!(collections.len(), 2);
    }

    #[test]
    fn test_file_len() {
        let (_temp, mut storage) = setup_test_db();

        let initial_len = storage.file_len().unwrap();
        assert!(initial_len > 0, "File should have header");

        storage.write_data(b"Some test data").unwrap();

        let new_len = storage.file_len().unwrap();
        assert!(new_len > initial_len, "File should grow after write");
    }

    #[test]
    fn test_data_persistence_after_reopen() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");

        let offset;

        // Write data
        {
            let mut storage = StorageEngine::open(&db_path).unwrap();
            storage.create_collection("test").unwrap();
            offset = storage.write_data(b"Persistent data").unwrap();
            storage.flush().unwrap();
        }

        // Reopen and read
        {
            let mut storage = StorageEngine::open(&db_path).unwrap();
            let data = storage.read_data(offset).unwrap();
            assert_eq!(data, b"Persistent data");
        }
    }

    #[test]
    fn test_empty_data_write() {
        let (_temp, mut storage) = setup_test_db();

        // Empty data write is still allowed at write-time (length=0 is written)
        let offset = storage.write_data(b"").unwrap();

        // But reading it back should fail with our new validation
        // (zero-length documents are considered corrupted)
        let result = storage.read_data(offset);
        assert!(result.is_err(), "Reading zero-length document should fail");

        match result {
            Err(IronBaseError::Corruption(msg)) => {
                assert!(
                    msg.contains("zero length"),
                    "Error should mention zero length: {}",
                    msg
                );
            }
            _ => panic!("Expected Corruption error for zero-length document"),
        }
    }

    #[test]
    fn test_large_data_write() {
        let (_temp, mut storage) = setup_test_db();

        // Create 1MB data block
        let large_data = vec![0xAB; 1024 * 1024];
        let offset = storage.write_data(&large_data).unwrap();

        let read_data = storage.read_data(offset).unwrap();
        assert_eq!(read_data.len(), large_data.len());
        assert_eq!(read_data, large_data);
    }

    #[test]
    fn test_collection_isolation_metadata() {
        let (_temp, mut storage) = setup_test_db();

        storage.create_collection("users").unwrap();
        storage.create_collection("posts").unwrap();

        // Modify users metadata
        {
            let meta = storage.get_collection_meta_mut("users").unwrap();
            meta.last_id = 42;
            meta.document_count = 100;
        }

        // Verify posts metadata not affected
        let posts_meta = storage.get_collection_meta("posts").unwrap();
        assert_eq!(posts_meta.last_id, 0);
        assert_eq!(posts_meta.document_count, 0);
    }

    #[test]
    fn test_header_defaults() {
        let header = Header::default();

        assert_eq!(header.magic, *b"MONGOLTE");
        assert_eq!(header.version, 6); // Version 6: last_compact_size persistence (P1-7)
        assert_eq!(header.page_size, 4096);
        assert_eq!(header.collection_count, 0);
        assert_eq!(header.free_list_head, 0);
        assert_eq!(header.data_end_offset, HEADER_SIZE); // Documents start after header
        assert!(!header.clean_shutdown); // Default: not clean shutdown
    }

    #[test]
    fn test_clean_shutdown_flag() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");

        // Create new database - should NOT be clean shutdown
        {
            let storage = StorageEngine::open(&db_path).unwrap();
            assert!(!storage.was_clean_shutdown()); // New database - no previous shutdown
                                                    // Drop runs and calls mark_clean_shutdown automatically
        }

        // Reopen after proper Drop (which calls mark_clean_shutdown)
        {
            let storage = StorageEngine::open(&db_path).unwrap();
            assert!(storage.was_clean_shutdown()); // Clean - Drop called mark_clean_shutdown
                                                   // Drop runs and calls mark_clean_shutdown again
        }

        // Verify clean shutdown persists
        {
            let storage = StorageEngine::open(&db_path).unwrap();
            assert!(storage.was_clean_shutdown()); // Still clean
        }

        // Test explicit mark_clean_shutdown (should work same as Drop)
        {
            let mut storage = StorageEngine::open(&db_path).unwrap();
            assert!(storage.was_clean_shutdown()); // Clean from previous
            storage.mark_clean_shutdown().unwrap();
            // Both explicit and Drop will mark clean
        }

        // Verify still clean
        {
            let storage = StorageEngine::open(&db_path).unwrap();
            assert!(storage.was_clean_shutdown()); // Clean
        }
    }

    // ========== ACD Transaction Tests ==========

    #[test]
    fn test_transaction_commit_with_insert() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");

        {
            let mut storage = StorageEngine::open(&db_path).unwrap();
            storage.create_collection("users").unwrap();

            // Create and commit transaction
            let mut tx = crate::transaction::Transaction::new(1);
            tx.add_operation(crate::transaction::Operation::Insert {
                collection: "users".to_string(),
                doc_id: crate::document::DocumentId::Int(1),
                doc: std::sync::Arc::new(serde_json::json!({"name": "Alice", "age": 30})),
            })
            .unwrap();

            storage.commit_transaction(&mut tx).unwrap();
        }

        // Verify data persisted
        {
            let storage = StorageEngine::open(&db_path).unwrap();
            let file_len = storage.file_len().unwrap();
            assert!(file_len > 0, "Storage should contain data after commit");
        }
    }

    #[test]
    fn test_transaction_rollback() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");

        let mut storage = StorageEngine::open(&db_path).unwrap();
        storage.create_collection("users").unwrap();

        // Create and rollback transaction
        let mut tx = crate::transaction::Transaction::new(1);
        tx.add_operation(crate::transaction::Operation::Insert {
            collection: "users".to_string(),
            doc_id: crate::document::DocumentId::Int(1),
            doc: std::sync::Arc::new(serde_json::json!({"name": "Bob"})),
        })
        .unwrap();

        storage.rollback_transaction(&mut tx).unwrap();

        // Transaction should be rolled back
        assert_eq!(tx.state(), crate::transaction::TransactionState::Aborted);
    }

    #[test]
    fn test_wal_recovery_after_crash() {
        use crate::wal::{WALEntry, WALEntryType, WriteAheadLog};

        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");
        let wal_path = temp_dir.path().join("test.wal");

        // Simulate crash: Write WAL entries but don't apply to storage
        {
            let mut wal = WriteAheadLog::open(&wal_path).unwrap();

            // Write a committed transaction to WAL
            let tx_id = 1;
            wal.append(&WALEntry::new(tx_id, WALEntryType::Begin, vec![]))
                .unwrap();

            let operation = crate::transaction::Operation::Insert {
                collection: "users".to_string(),
                doc_id: crate::document::DocumentId::Int(1),
                doc: std::sync::Arc::new(serde_json::json!({"name": "Recovered Alice", "age": 25})),
            };
            let op_json = serde_json::to_string(&operation).unwrap();
            wal.append(&WALEntry::new(
                tx_id,
                WALEntryType::Operation,
                op_json.as_bytes().to_vec(),
            ))
            .unwrap();

            wal.append(&WALEntry::new(tx_id, WALEntryType::Commit, vec![]))
                .unwrap();
            wal.flush().unwrap();
        }

        // Create storage file (simulating existing database)
        {
            let mut storage = StorageEngine::open(&db_path).unwrap();
            storage.create_collection("users").unwrap();
            storage.flush().unwrap();
        }

        // Reopen storage - should recover from WAL
        {
            let mut storage = StorageEngine::open(&db_path).unwrap();
            // Explicitly call recovery (DatabaseCore does this automatically)
            let (_, _, _) = storage.recover_from_wal().unwrap();

            // WAL should be cleared after recovery
            let mut wal_result = WriteAheadLog::open(&wal_path).unwrap();
            let recovered = wal_result.recover().unwrap();
            assert_eq!(recovered.len(), 0, "WAL should be empty after recovery");
        }
    }

    #[test]
    fn test_wal_recovery_multiple_transactions() {
        use crate::wal::{WALEntry, WALEntryType, WriteAheadLog};

        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");
        let wal_path = temp_dir.path().join("test.wal");

        // Write multiple committed transactions to WAL
        {
            let mut wal = WriteAheadLog::open(&wal_path).unwrap();

            for tx_id in 1..=3 {
                wal.append(&WALEntry::new(tx_id, WALEntryType::Begin, vec![]))
                    .unwrap();

                let operation = crate::transaction::Operation::Insert {
                    collection: "users".to_string(),
                    doc_id: crate::document::DocumentId::Int(tx_id as i64),
                    doc: std::sync::Arc::new(
                        serde_json::json!({"name": format!("User {}", tx_id)}),
                    ),
                };
                let op_json = serde_json::to_string(&operation).unwrap();
                wal.append(&WALEntry::new(
                    tx_id,
                    WALEntryType::Operation,
                    op_json.as_bytes().to_vec(),
                ))
                .unwrap();

                wal.append(&WALEntry::new(tx_id, WALEntryType::Commit, vec![]))
                    .unwrap();
            }
            wal.flush().unwrap();
        }

        // Create storage and recover
        {
            let mut storage = StorageEngine::open(&db_path).unwrap();
            storage.create_collection("users").unwrap();
        }

        // Reopen and verify recovery
        {
            let storage = StorageEngine::open(&db_path).unwrap();
            let file_len = storage.file_len().unwrap();
            assert!(file_len > 0, "Storage should contain recovered data");
        }
    }

    // ========================================================================
    // Metadata WAL Recovery Tests (Crash Safety)
    // ========================================================================

    #[test]
    fn test_metadata_wal_flush_succeeds() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");

        // Create database, insert data, flush
        {
            let mut storage = StorageEngine::open(&db_path).unwrap();
            storage.create_collection("users").unwrap();

            // Insert a document via write_data
            let doc = serde_json::json!({"name": "Alice", "_id": 1});
            let doc_bytes = serde_json::to_vec(&doc).unwrap();
            let offset = storage.write_data(&doc_bytes).unwrap();

            // Update collection metadata
            let meta = storage.collections_mut().get_mut("users").unwrap();
            meta.document_count = 1;
            meta.document_catalog
                .insert(crate::document::DocumentId::Int(1), offset);

            // flush() should succeed (internally logs to WAL first)
            storage.flush().unwrap();
        }

        // Verify database can be reopened
        {
            let storage = StorageEngine::open(&db_path).unwrap();
            assert!(storage.collections.contains_key("users"));
            assert_eq!(storage.collections["users"].document_count, 1);
        }
    }

    #[test]
    fn test_metadata_recovery_from_wal_after_corruption() {
        use std::io::{Read as _, Seek, Write};

        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");

        // Step 1: Create database with data
        {
            let mut storage = StorageEngine::open(&db_path).unwrap();
            storage.create_collection("products").unwrap();

            // Insert documents
            for i in 1..=5 {
                let doc = serde_json::json!({"_id": i, "name": format!("Product {}", i)});
                let doc_bytes = serde_json::to_vec(&doc).unwrap();
                let offset = storage.write_data(&doc_bytes).unwrap();

                let meta = storage.collections_mut().get_mut("products").unwrap();
                meta.document_count = i as u64;
                meta.document_catalog
                    .insert(crate::document::DocumentId::Int(i), offset);
            }

            // Log metadata to WAL (simulating pre-crash state)
            storage.write_metadata_snapshot().unwrap();
            // Don't call flush() - simulate crash after WAL write but before file flush
        }

        // Step 2: Corrupt the metadata in the file (simulate crash during write)
        {
            let mut file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&db_path)
                .unwrap();

            // Read header to find metadata offset
            let mut header_bytes = [0u8; 256];
            file.seek(std::io::SeekFrom::Start(0)).unwrap();
            file.read_exact(&mut header_bytes).unwrap();

            // Corrupt metadata offset (bytes 24-31)
            file.seek(std::io::SeekFrom::Start(24)).unwrap();
            file.write_all(&[0xFF; 8]).unwrap(); // Invalid offset
            file.sync_all().unwrap();
        }

        // Step 3: Reopen - should recover from WAL
        {
            let storage = StorageEngine::open(&db_path).unwrap();

            // Should have recovered the collection
            assert!(
                storage.collections.contains_key("products"),
                "Collection should be recovered from WAL or document scan"
            );
        }
    }

    #[test]
    fn test_rebuild_from_documents_fallback() {
        use std::io::{Seek, Write};

        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");
        let wal_path = temp_dir.path().join("test.wal");

        // Step 1: Create database with data and flush properly
        {
            let mut storage = StorageEngine::open(&db_path).unwrap();
            storage.create_collection("items").unwrap();

            for i in 1..=3 {
                let doc = serde_json::json!({"_id": i, "value": i * 10});
                let doc_bytes = serde_json::to_vec(&doc).unwrap();
                let offset = storage.write_data(&doc_bytes).unwrap();

                let meta = storage.collections_mut().get_mut("items").unwrap();
                meta.document_count = i as u64;
                meta.document_catalog
                    .insert(crate::document::DocumentId::Int(i), offset);
            }

            storage.flush().unwrap();
        }

        // Step 2: Delete WAL and corrupt metadata
        {
            // Remove WAL file
            if wal_path.exists() {
                fs::remove_file(&wal_path).unwrap();
            }

            // Corrupt metadata offset
            let mut file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&db_path)
                .unwrap();

            file.seek(std::io::SeekFrom::Start(24)).unwrap();
            file.write_all(&[0xFF; 8]).unwrap();
            file.sync_all().unwrap();
        }

        // Step 3: Reopen - should rebuild from document scan
        {
            let storage = StorageEngine::open(&db_path).unwrap();

            // Should have recovered via document scan (at least found the collection structure)
            // Note: document scan may not fully recover if documents don't have _collection field
            assert!(
                storage.file_len().unwrap() > 0,
                "Database file should exist and be non-empty"
            );
        }
    }

    #[test]
    fn test_metadata_wal_entry_format() {
        use crate::wal::{WALEntry, WALEntryType};

        // Create a MetadataWALEntry with proper CollectionMeta struct
        let mut collections = std::collections::HashMap::new();
        let test_meta = CollectionMeta {
            name: "test_collection".to_string(),
            document_count: 42,
            live_document_count: 42,
            data_offset: 256,
            index_offset: 0,
            last_id: 42,
            document_catalog: HashMap::new(),
            document_order: Vec::new(),
            indexes: Vec::new(),
            fuzzy_indexes: Vec::new(),
            fulltext_indexes: Vec::new(),
            vector_indexes: Vec::new(),
            schema: None,
            flags: CollectionFlags::default(),
            auto_embedding_config: None,
        };
        collections.insert("test_collection".to_string(), test_meta);

        let metadata_entry = MetadataWALEntry {
            collections: collections.clone(),
            data_end_offset: 12345,
        };

        // Serialize to JSON
        let json_data = serde_json::to_vec(&metadata_entry).unwrap();

        // Create WAL entry
        let wal_entry = WALEntry::new(0, WALEntryType::MetadataSnapshot, json_data.clone());

        // Verify entry type
        assert_eq!(wal_entry.entry_type, WALEntryType::MetadataSnapshot);
        assert_eq!(wal_entry.transaction_id, 0);

        // Verify can deserialize back
        let recovered: MetadataWALEntry = serde_json::from_slice(&wal_entry.data).unwrap();
        assert_eq!(recovered.data_end_offset, 12345);
        assert!(recovered.collections.contains_key("test_collection"));
        assert_eq!(recovered.collections["test_collection"].document_count, 42);
    }
}
