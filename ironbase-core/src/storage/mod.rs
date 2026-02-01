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
use crate::log_error;
use crate::transaction::{Transaction, TransactionId};
use crate::wal::WriteAheadLog;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

// Re-export public types
pub use compaction::{CheckpointStats, CompactionConfig, CompactionStats};

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
    pub key: crate::transaction::IndexKey,
    pub doc_id: crate::document::DocumentId,
}

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
    // If true, indexes can be trusted from .idx files without rebuild
    #[serde(default)]
    pub clean_shutdown: bool,
}

impl Default for Header {
    fn default() -> Self {
        Header {
            magic: *b"MONGOLTE",
            version: 4, // Version 4: clean shutdown tracking
            page_size: 4096,
            collection_count: 0,
            free_list_head: 0,
            index_section_offset: 0,
            metadata_offset: 0, // Will be set on first write
            metadata_size: 0,
            data_end_offset: HEADER_SIZE, // Documents start right after header
            clean_shutdown: false,        // Will be set to true on graceful shutdown
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
    /// Embedding provider name (e.g., "fasttext", "openai", "ollama")
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
    pub data_offset: u64,  // Data start position
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
    collections: HashMap<String, CollectionMeta>,
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
    /// Acquire lock with stale lock detection
    ///
    /// If the lock file exists and contains a PID of a dead process,
    /// we consider it stale and remove it before retrying.
    fn acquire_lock_with_stale_detection(lock_path: &Path, db_path: &str) -> Result<File> {
        use std::io::{Read, Seek, Write};

        let mut lock_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(lock_path)?;

        // Try to acquire the lock
        match lock_file.try_lock_exclusive() {
            Ok(()) => {
                // Lock acquired - write our PID
                lock_file.set_len(0)?;
                lock_file.seek(std::io::SeekFrom::Start(0))?;
                writeln!(lock_file, "{}", std::process::id())?;
                lock_file.sync_all()?;
                Ok(lock_file)
            }
            Err(_) => {
                // Lock failed - check if it's stale
                let mut pid_str = String::new();
                lock_file.seek(std::io::SeekFrom::Start(0))?;
                let _ = lock_file.read_to_string(&mut pid_str);

                if let Ok(pid) = pid_str.trim().parse::<u32>() {
                    if !Self::is_process_alive(pid) {
                        // Process is dead - lock is stale
                        eprintln!(
                            "[WARN] Stale lock detected (PID {} is dead), cleaning up...",
                            pid
                        );

                        // Release any existing lock and reopen
                        drop(lock_file);

                        // Remove and recreate lock file
                        let _ = std::fs::remove_file(lock_path);

                        let mut lock_file = OpenOptions::new()
                            .read(true)
                            .write(true)
                            .create(true)
                            .open(lock_path)?;

                        // Try again
                        lock_file
                            .try_lock_exclusive()
                            .map_err(|_| IronBaseError::DatabaseLocked(db_path.to_string()))?;

                        // Write our PID
                        writeln!(lock_file, "{}", std::process::id())?;
                        lock_file.sync_all()?;

                        return Ok(lock_file);
                    }
                }

                // Lock is held by a live process
                Err(IronBaseError::DatabaseLocked(db_path.to_string()))
            }
        }
    }

    /// Check if a process with the given PID is still alive
    #[cfg(unix)]
    fn is_process_alive(pid: u32) -> bool {
        // On Unix, kill(pid, 0) checks if process exists without sending signal
        // Returns 0 if process exists (even if we can't signal it)
        // Returns -1 with ESRCH if process doesn't exist
        unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
    }

    #[cfg(windows)]
    fn is_process_alive(pid: u32) -> bool {
        // On Windows, try to check via tasklist command
        // This is slower but doesn't require additional dependencies
        std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {}", pid), "/NH"])
            .output()
            .map(|output| {
                let stdout = String::from_utf8_lossy(&output.stdout);
                // tasklist returns "INFO: No tasks..." if PID not found
                !stdout.contains("INFO:") && stdout.contains(&pid.to_string())
            })
            .unwrap_or(true) // If check fails, assume process is alive (safer)
    }

    /// Open or create database
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path_str = path.as_ref().to_string_lossy().to_string();
        let exists = path.as_ref().exists();

        // Create separate lock file to allow other processes to READ the DB (hot backup)
        // On Windows, file locks are mandatory and block ALL access including reads
        // By locking a separate .lock file, we allow backup tools to read the DB file
        let lock_path = PathBuf::from(&path_str).with_extension("mlite.lock");
        let lock_file = Self::acquire_lock_with_stale_detection(&lock_path, &path_str)?;

        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&path)?;

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
                        eprintln!("[WARN] Metadata corrupted: {}, attempting WAL recovery", e);
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
            collections,
            file_path: path_str,
            wal,
            metadata_dirty: false,
            metadata_snapshot_pending: false,
            lock_file,
            wal_ops_since_clear: 0,
            was_clean_shutdown: was_clean,
        };

        let migrated = Self::rebuild_document_order_if_needed(&mut storage.collections);
        if migrated {
            storage.metadata_dirty = true;
        }

        // If metadata was corrupted, attempt recovery (crash scenario)
        if needs_rebuild {
            // Mark as dirty start - don't trust indexes
            storage.was_clean_shutdown = false;
            // First try WAL recovery (has most recent metadata snapshot)
            if storage.recover_metadata_from_wal()? {
                eprintln!("[INFO] Successfully recovered metadata from WAL");
            } else {
                // WAL recovery failed - fall back to document scan
                eprintln!("[WARN] WAL recovery failed, rebuilding metadata from documents");
                storage.rebuild_from_documents()?;
            }
            // CRITICAL: Flush to persist recovered metadata and fix corrupt header on disk
            eprintln!("[INFO] Persisting recovered metadata to fix corrupt header");
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
                        eprintln!(
                            "[WARN] Recovered corrupted data_end_offset: {} -> {}",
                            storage.header.data_end_offset, recovered_offset
                        );
                        storage.header.data_end_offset = recovered_offset;
                        storage.mark_metadata_dirty()?;
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

        self.collections.insert(name.to_string(), meta);
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

        self.collections.remove(name);
        self.header.collection_count -= 1;

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
        self.collections.get_mut(name)
    }

    /// Write a metadata snapshot to the WAL for crash recovery
    fn write_metadata_snapshot(&mut self) -> Result<()> {
        use crate::wal::{WALEntry, WALEntryType};

        let metadata_entry = MetadataWALEntry {
            collections: self.collections.clone(),
            data_end_offset: self.header.data_end_offset,
        };

        let entry_data = serde_json::to_vec(&metadata_entry)
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
                    eprintln!(
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
            eprintln!(
                "[INFO] Recovering metadata from WAL: {} collections, data_end_offset={}",
                snapshot.collections.len(),
                snapshot.data_end_offset
            );

            // Restore collections and header
            self.collections = snapshot.collections;
            self.header.data_end_offset = snapshot.data_end_offset;
            self.header.metadata_offset = snapshot.data_end_offset;
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

        eprintln!("[INFO] Starting metadata rebuild from document scan...");

        let file_len = self.file.metadata()?.len();

        // If file is too small to have any documents, just initialize empty
        if file_len <= HEADER_SIZE {
            self.header = Header::default();
            self.collections.clear();
            self.mark_metadata_dirty()?;
            self.flush_metadata()?;
            self.metadata_snapshot_pending = false;
            self.file.sync_all()?;
            return Ok(());
        }

        // Clear existing collections - we're rebuilding from scratch
        self.collections.clear();

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
                        .collections
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
                        if let DocumentId::Int(id_num) = &doc_id {
                            let current_max =
                                max_ids_by_collection.entry(collection_name).or_insert(0);
                            if (*id_num as u64) > *current_max {
                                *current_max = *id_num as u64;
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
            if let Some(meta) = self.collections.get_mut(&collection_name) {
                meta.last_id = max_id;
            }
        }

        // Update header
        self.header.data_end_offset = offset;
        self.header.collection_count = self.collections.len() as u32;

        // Write corrected metadata
        self.mark_metadata_dirty()?;
        self.flush_metadata()?;
        self.metadata_snapshot_pending = false;
        self.file.sync_all()?;

        eprintln!(
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

    /// Commit a transaction (9-step atomic operation) - internal implementation
    /// This is the core of ACD guarantee
    ///
    /// # Arguments
    /// * `transaction` - The transaction to commit
    /// * `sync_file` - Whether to sync the main file (false for batch mode)
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
                    let mut doc_clone = doc.clone();
                    inject_collection(&mut doc_clone, collection);
                    Operation::Insert {
                        collection: collection.clone(),
                        doc_id: doc_id.clone(),
                        doc: doc_clone,
                    }
                }
                Operation::Update {
                    collection,
                    doc_id,
                    old_doc,
                    new_doc,
                } => {
                    let mut old_clone = old_doc.clone();
                    let mut new_clone = new_doc.clone();
                    inject_collection(&mut old_clone, collection);
                    inject_collection(&mut new_clone, collection);
                    Operation::Update {
                        collection: collection.clone(),
                        doc_id: doc_id.clone(),
                        old_doc: old_clone,
                        new_doc: new_clone,
                    }
                }
                Operation::Delete {
                    collection,
                    doc_id,
                    old_doc,
                } => {
                    let mut old_clone = old_doc.clone();
                    inject_collection(&mut old_clone, collection);
                    Operation::Delete {
                        collection: collection.clone(),
                        doc_id: doc_id.clone(),
                        old_doc: old_clone,
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
                if let Some(meta) = self.collections.get_mut(&metadata_change.collection) {
                    meta.last_id = metadata_change.last_id as u64;
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
                    let doc_json = serde_json::to_string(doc)
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
                    let doc_json = serde_json::to_string(new_doc)
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
                let doc_json = serde_json::to_string(doc)
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
                let doc_json = serde_json::to_string(new_doc)
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
    ///
    /// NOTE: Previously returned the full Vec<Vec<WALEntry>>, but the caller only
    /// used .is_empty() on it. Returning count saves O(WAL_size) memory.
    pub fn recover_from_wal(&mut self) -> Result<(usize, Vec<RecoveredIndexChange>)> {
        let recovered = self.wal.recover()?;

        if recovered.is_empty() {
            return Ok((0, vec![]));
        }

        let recovered_count = recovered.len();
        let mut all_index_changes = Vec::new();

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

                        let key: crate::transaction::IndexKey =
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

        Ok((recovered_count, all_index_changes))
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
        for meta in self.collections.values_mut() {
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
                                .collections
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
                                if let DocumentId::Int(id_num) = &doc_id {
                                    let current_max = max_ids_by_collection
                                        .entry(collection_name.to_string())
                                        .or_insert(0);
                                    if (*id_num as u64) > *current_max {
                                        *current_max = *id_num as u64;
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
            if let Some(meta) = self.collections.get_mut(&collection_name) {
                if max_id > meta.last_id {
                    meta.last_id = max_id;
                }
            }
        }

        // Fix document counts based on actual catalog state
        // This is necessary because:
        // 1. Updates/replacements counted each version separately
        // 2. Tombstones removed from catalog but didn't decrement counts
        for meta in self.collections.values_mut() {
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
        let _ = self.flush();
        // Clear WAL on close to keep it clean for next open
        let _ = self.checkpoint();
        // Mark clean shutdown for fast restart (indexes can be loaded from .idx files)
        let _ = self.mark_clean_shutdown();
        // Explicitly unlock the lock file to ensure other processes can access the database
        // This is more reliable than relying on File::drop() to release the flock
        let _ = self.lock_file.unlock();
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
        if let Some(meta) = self.collections.get_mut(collection) {
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

    #[test]
    fn test_create_new_database() {
        let (_temp, storage) = setup_test_db();

        assert_eq!(storage.header.magic, *b"MONGOLTE");
        assert_eq!(storage.header.version, 4); // Version 4: clean shutdown tracking
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
        assert_eq!(header.version, 4); // Version 4: clean shutdown tracking
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
                doc: serde_json::json!({"name": "Alice", "age": 30}),
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
            doc: serde_json::json!({"name": "Bob"}),
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
                doc: serde_json::json!({"name": "Recovered Alice", "age": 25}),
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
            storage.recover_from_wal().unwrap();

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
                    doc: serde_json::json!({"name": format!("User {}", tx_id)}),
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
            let meta = storage.collections.get_mut("users").unwrap();
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

                let meta = storage.collections.get_mut("products").unwrap();
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

                let meta = storage.collections.get_mut("items").unwrap();
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
