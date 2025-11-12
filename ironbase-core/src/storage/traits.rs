// src/storage/traits.rs
//! Storage abstraction traits for MongoLite
//!
//! This module defines the core storage interface that all storage backends
//! must implement. This enables:
//! - Dependency injection
//! - Easy testing with MemoryStorage
//! - Future extensibility (S3, Redis, etc.)
//!
//! # Architecture
//!
//! ```text
//! Storage trait (unified interface)
//!   ├── FileStorage (production, wraps StorageEngine)
//!   ├── MemoryStorage (testing, in-memory HashMap)
//!   └── Future: S3Storage, RedisStorage, etc.
//! ```

use serde_json::Value;
use crate::error::Result;
use crate::document::{Document, DocumentId};
use crate::storage::CollectionMeta;  // CollectionMeta is in storage::mod.rs
use std::path::Path;

/// Core storage abstraction for MongoLite
///
/// This trait defines the unified interface that all storage backends must implement.
/// It provides document CRUD operations, collection management, and metadata access.
///
/// # Implementations
///
/// - **FileStorage**: Production storage backed by .mlite files
/// - **MemoryStorage**: Fast in-memory storage for testing
///
/// # Examples
///
/// ```ignore
/// // File-based storage (production)
/// let storage = FileStorage::open("data.mlite")?;
///
/// // Memory storage (testing)
/// let storage = MemoryStorage::new();
/// ```
pub trait Storage: Send + Sync {
    // ========================================================================
    // DOCUMENT OPERATIONS
    // ========================================================================

    /// Write a document to the collection
    ///
    /// # Arguments
    ///
    /// * `collection` - Collection name
    /// * `doc` - Document as JSON Value
    ///
    /// # Returns
    ///
    /// The file offset where the document was written (for file-based storage)
    /// or a synthetic offset (for memory-based storage)
    fn write_document(&mut self, collection: &str, doc: &Value) -> Result<u64>;

    /// Read a document by its ID
    ///
    /// # Arguments
    ///
    /// * `collection` - Collection name
    /// * `id` - Document ID
    ///
    /// # Returns
    ///
    /// The document as JSON, or None if not found
    fn read_document(&self, collection: &str, id: &DocumentId) -> Result<Option<Value>>;

    /// Scan all documents in a collection
    ///
    /// Returns an iterator over all documents. This is used by `find()` operations.
    ///
    /// # Performance
    ///
    /// - FileStorage: Memory-mapped sequential scan
    /// - MemoryStorage: Iterator over Vec
    fn scan_documents<'a>(&'a self, collection: &str) -> Result<Vec<Document>>;

    // ========================================================================
    // COLLECTION MANAGEMENT
    // ========================================================================

    /// Create a new collection
    fn create_collection(&mut self, name: &str) -> Result<()>;

    /// Drop (delete) a collection
    fn drop_collection(&mut self, name: &str) -> Result<()>;

    /// List all collection names
    fn list_collections(&self) -> Vec<String>;

    // ========================================================================
    // METADATA ACCESS
    // ========================================================================

    /// Get immutable reference to collection metadata
    fn get_collection_meta(&self, name: &str) -> Option<&CollectionMeta>;

    /// Get mutable reference to collection metadata
    fn get_collection_meta_mut(&mut self, name: &str) -> Option<&mut CollectionMeta>;

    // ========================================================================
    // PERSISTENCE & FLUSHING
    // ========================================================================

    /// Flush any pending writes to persistent storage
    ///
    /// For FileStorage, this writes metadata to disk.
    /// For MemoryStorage, this is a no-op.
    fn flush(&mut self) -> Result<()>;
}

// ============================================================================
// OPTIONAL TRAITS FOR SPECIALIZED FEATURES
// ============================================================================

/// Storage that supports compaction (garbage collection)
pub trait CompactableStorage: Storage {
    /// Compact the storage to reclaim space from deleted documents
    fn compact(&mut self) -> Result<crate::storage::compaction::CompactionStats>;
}

/// Storage that supports indexing
pub trait IndexableStorage: Storage {
    /// Create an index on a field
    fn create_index(&mut self, collection: &str, field: &str, unique: bool) -> Result<String>;

    /// Drop an index
    fn drop_index(&mut self, collection: &str, index_name: &str) -> Result<()>;

    /// List all indexes for a collection
    fn list_indexes(&self, collection: &str) -> Vec<String>;
}

// ============================================================================
// HELPER TYPES
// ============================================================================

/// Storage-specific configuration
pub trait StorageConfig {
    /// Open existing storage at path
    fn open<P: AsRef<Path>>(path: P) -> Result<Self>
    where
        Self: Sized;

    /// Create new storage at path
    fn create<P: AsRef<Path>>(path: P) -> Result<Self>
    where
        Self: Sized;
}
