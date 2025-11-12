# Phase 2: Storage Abstraction - DETAILED DESIGN

**Status**: PLANNING
**Priority**: HIGH (from REFACTORING_PLAN.md)
**Estimated Duration**: 8-12 hours
**Prerequisites**: Phase 1 Complete ✅

---

## Executive Summary

Phase 2 will extract the storage layer into abstract traits, enabling:
- **Dependency Injection**: Collection generic over storage backend
- **Better Testing**: MemoryStorage for fast unit tests
- **Future Extensibility**: Support for S3, Redis, etc.
- **SOLID Compliance**: Dependency Inversion Principle

---

## Problem Statement

### Current Architecture Issues

**Problem 1: Tight Coupling**
```rust
pub struct Collection {
    name: String,
    storage: Arc<RwLock<StorageEngine>>,  // ❌ Hardcoded to StorageEngine
}
```

**Problem 2: Hard to Test**
```rust
#[test]
fn test_collection_insert() {
    // ❌ Must create real file on disk for every test
    let storage = StorageEngine::create("test.mlite")?;
    // Slow, pollutes filesystem, cleanup required
}
```

**Problem 3: God Class**
- `StorageEngine` does EVERYTHING:
  - File I/O
  - Metadata management
  - Transaction coordination
  - WAL recovery
  - Compaction
- Violates Single Responsibility Principle

---

## Proposed Architecture

### New Trait Hierarchy

```
Storage (trait)
  ├── DocumentReader (sub-trait)
  │   ├── read_document(id) -> Result<Value>
  │   ├── scan_documents(collection) -> Iterator<Document>
  │   └── get_collection_meta(name) -> Option<&CollectionMeta>
  │
  ├── DocumentWriter (sub-trait)
  │   ├── write_document(collection, doc) -> Result<u64>
  │   ├── update_document(collection, id, doc) -> Result<()>
  │   └── delete_document(collection, id) -> Result<()>
  │
  └── MetadataManager (sub-trait)
      ├── create_collection(name) -> Result<()>
      ├── drop_collection(name) -> Result<()>
      ├── list_collections() -> Vec<String>
      └── flush_metadata() -> Result<()>
```

### Implementations

**1. FileStorage (production)**
```rust
pub struct FileStorage {
    inner: StorageEngine,  // Wraps existing implementation
}

impl Storage for FileStorage {
    // Delegates to StorageEngine
}
```

**2. MemoryStorage (testing)**
```rust
pub struct MemoryStorage {
    collections: HashMap<String, Vec<Document>>,
    metadata: HashMap<String, CollectionMeta>,
}

impl Storage for MemoryStorage {
    // In-memory implementation, no I/O
}
```

**3. Future: S3Storage, RedisStorage, etc.**

---

## Implementation Plan

### Step 1: Define Storage Traits (2-3 hours)

**File**: `ironbase-core/src/storage/traits.rs`

```rust
use serde_json::Value;
use crate::error::Result;
use crate::document::{Document, DocumentId};

/// Core storage abstraction for MongoLite
///
/// This trait defines the interface that all storage backends must implement.
/// It combines document I/O, metadata management, and transaction support.
pub trait Storage: Send + Sync {
    /// Read a document by its ID
    fn read_document(&self, collection: &str, id: &DocumentId) -> Result<Option<Value>>;

    /// Write a new document, returns offset
    fn write_document(&mut self, collection: &str, doc: &Value) -> Result<u64>;

    /// Update an existing document
    fn update_document(&mut self, collection: &str, id: &DocumentId, doc: &Value) -> Result<()>;

    /// Delete a document
    fn delete_document(&mut self, collection: &str, id: &DocumentId) -> Result<()>;

    /// Scan all documents in a collection
    fn scan_documents(&self, collection: &str) -> Result<Box<dyn Iterator<Item = Document> + '_>>;

    /// Get collection metadata
    fn get_collection_meta(&self, name: &str) -> Option<&CollectionMeta>;

    /// Get mutable collection metadata
    fn get_collection_meta_mut(&mut self, name: &str) -> Option<&mut CollectionMeta>;

    /// Create a new collection
    fn create_collection(&mut self, name: &str) -> Result<()>;

    /// Drop a collection
    fn drop_collection(&mut self, name: &str) -> Result<()>;

    /// List all collections
    fn list_collections(&self) -> Vec<String>;

    /// Flush metadata to disk (file-based storage only)
    fn flush(&mut self) -> Result<()>;

    /// Begin a transaction (optional, default: no-op)
    fn begin_transaction(&mut self) -> Result<TransactionId> {
        Ok(TransactionId::default())
    }

    /// Commit a transaction (optional, default: no-op)
    fn commit_transaction(&mut self, _tx_id: TransactionId) -> Result<()> {
        Ok(())
    }

    /// Rollback a transaction (optional, default: no-op)
    fn rollback_transaction(&mut self, _tx_id: TransactionId) -> Result<()> {
        Ok(())
    }
}

/// Optional sub-traits for specialized functionality

pub trait CompactableStorage: Storage {
    fn compact(&mut self) -> Result<CompactionStats>;
}

pub trait IndexableStorage: Storage {
    fn create_index(&mut self, collection: &str, field: &str, unique: bool) -> Result<String>;
    fn drop_index(&mut self, collection: &str, index_name: &str) -> Result<()>;
    fn list_indexes(&self, collection: &str) -> Vec<String>;
}
```

**Design Decisions:**
- ✅ Single unified trait (not split) - simpler for users
- ✅ Optional sub-traits for advanced features
- ✅ Default implementations for optional methods
- ✅ Send + Sync for thread safety
- ✅ Iterator-based scanning for memory efficiency

---

### Step 2: Implement FileStorage Wrapper (1-2 hours)

**File**: `ironbase-core/src/storage/file_storage.rs`

```rust
use super::traits::Storage;
use super::mod::StorageEngine;
use crate::error::Result;

/// File-based storage implementation (production)
///
/// This is a thin wrapper around the existing StorageEngine.
/// It delegates all operations to StorageEngine for backward compatibility.
pub struct FileStorage {
    inner: StorageEngine,
}

impl FileStorage {
    /// Open an existing database file
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        Ok(FileStorage {
            inner: StorageEngine::open(path)?,
        })
    }

    /// Create a new database file
    pub fn create<P: AsRef<Path>>(path: P) -> Result<Self> {
        Ok(FileStorage {
            inner: StorageEngine::create(path)?,
        })
    }

    /// Get access to the underlying StorageEngine (for migration)
    pub fn inner(&self) -> &StorageEngine {
        &self.inner
    }

    /// Get mutable access to the underlying StorageEngine (for migration)
    pub fn inner_mut(&mut self) -> &mut StorageEngine {
        &mut self.inner
    }
}

impl Storage for FileStorage {
    fn read_document(&self, collection: &str, id: &DocumentId) -> Result<Option<Value>> {
        // Delegate to StorageEngine
        // Implementation: use existing StorageEngine methods
        todo!("Delegate to inner.read_document()")
    }

    fn write_document(&mut self, collection: &str, doc: &Value) -> Result<u64> {
        todo!("Delegate to inner.write_document()")
    }

    // ... delegate all other methods to self.inner
}
```

**Benefits:**
- ✅ Zero behavior change - just wraps existing code
- ✅ 100% backward compatible
- ✅ Gradual migration path

---

### Step 3: Implement MemoryStorage (2-3 hours)

**File**: `ironbase-core/src/storage/memory_storage.rs`

```rust
use super::traits::Storage;
use std::collections::HashMap;
use parking_lot::RwLock;

/// In-memory storage implementation (testing)
///
/// Fast, zero I/O, perfect for unit tests.
/// Data is lost when dropped (not persistent).
pub struct MemoryStorage {
    collections: HashMap<String, Vec<Document>>,
    metadata: HashMap<String, CollectionMeta>,
    next_offset: u64,
}

impl MemoryStorage {
    pub fn new() -> Self {
        MemoryStorage {
            collections: HashMap::new(),
            metadata: HashMap::new(),
            next_offset: 0,
        }
    }
}

impl Storage for MemoryStorage {
    fn read_document(&self, collection: &str, id: &DocumentId) -> Result<Option<Value>> {
        if let Some(docs) = self.collections.get(collection) {
            Ok(docs.iter()
                .find(|doc| doc.id == *id)
                .map(|doc| doc.to_json()))
        } else {
            Ok(None)
        }
    }

    fn write_document(&mut self, collection: &str, doc: &Value) -> Result<u64> {
        let docs = self.collections.entry(collection.to_string())
            .or_insert_with(Vec::new);

        let offset = self.next_offset;
        self.next_offset += 1;

        let document = Document::from_json(&serde_json::to_string(doc)?)?;
        docs.push(document);

        Ok(offset)
    }

    fn scan_documents(&self, collection: &str) -> Result<Box<dyn Iterator<Item = Document> + '_>> {
        if let Some(docs) = self.collections.get(collection) {
            Ok(Box::new(docs.iter().cloned()))
        } else {
            Ok(Box::new(std::iter::empty()))
        }
    }

    // ... implement all other methods
}
```

**Benefits:**
- ✅ **10-100x faster** than file I/O in tests
- ✅ No filesystem pollution
- ✅ No cleanup required
- ✅ Deterministic (no I/O timing issues)

**Performance Comparison (estimated):**
```
File-based test:  200ms (create file, write, read, cleanup)
Memory-based test:  2ms (pure in-memory operations)
Speedup: 100x
```

---

### Step 4: Make Collection Generic (2-3 hours)

**File**: `ironbase-core/src/collection_core.rs`

**Before:**
```rust
pub struct Collection {
    name: String,
    storage: Arc<RwLock<StorageEngine>>,  // ❌ Hardcoded
}
```

**After:**
```rust
pub struct Collection<S: Storage> {
    name: String,
    storage: Arc<RwLock<S>>,  // ✅ Generic over any Storage
}

impl<S: Storage> Collection<S> {
    pub fn new(name: String, storage: Arc<RwLock<S>>) -> Result<Self> {
        Ok(Collection { name, storage })
    }

    pub fn insert_one(&self, fields: HashMap<String, Value>) -> Result<DocumentId> {
        let mut storage = self.storage.write();
        // ... same logic, but uses S: Storage trait methods
        storage.write_document(&self.name, &doc_json)?;
        Ok(doc_id)
    }

    // ... all other methods work the same
}
```

**Type Aliases for Convenience:**
```rust
/// File-based collection (production default)
pub type FileCollection = Collection<FileStorage>;

/// Memory-based collection (testing)
pub type MemoryCollection = Collection<MemoryStorage>;
```

---

### Step 5: Update Database (1-2 hours)

**File**: `ironbase-core/src/database.rs`

```rust
pub struct Database<S: Storage> {
    storage: Arc<RwLock<S>>,
    collections: DashMap<String, Arc<Collection<S>>>,
}

impl Database<FileStorage> {
    /// Open file-based database (production)
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let storage = Arc::new(RwLock::new(FileStorage::open(path)?));
        Ok(Database {
            storage,
            collections: DashMap::new(),
        })
    }
}

impl Database<MemoryStorage> {
    /// Create in-memory database (testing)
    pub fn memory() -> Self {
        let storage = Arc::new(RwLock::new(MemoryStorage::new()));
        Database {
            storage,
            collections: DashMap::new(),
        }
    }
}

/// Type aliases
pub type FileDatabase = Database<FileStorage>;
pub type MemoryDatabase = Database<MemoryStorage>;
```

---

### Step 6: Update Tests (1-2 hours)

**Before (slow):**
```rust
#[test]
fn test_insert_and_find() {
    let temp = tempfile::NamedTempFile::new().unwrap();
    let db = Database::open(temp.path()).unwrap();  // ❌ File I/O
    // ... test logic
}  // Cleanup required
```

**After (fast):**
```rust
#[test]
fn test_insert_and_find() {
    let db = Database::memory();  // ✅ Pure in-memory
    // ... test logic
}  // No cleanup needed
```

**Migration Strategy:**
1. Create new tests using MemoryStorage
2. Keep old file-based integration tests
3. Gradually migrate performance-critical tests

---

## File Structure

```
ironbase-core/src/
├── storage/
│   ├── mod.rs (existing StorageEngine)
│   ├── traits.rs (NEW - Storage trait)
│   ├── file_storage.rs (NEW - FileStorage wrapper)
│   ├── memory_storage.rs (NEW - MemoryStorage impl)
│   ├── compaction.rs (existing)
│   ├── metadata.rs (existing)
│   └── io.rs (existing)
├── collection_core.rs (MODIFIED - add generic param)
├── database.rs (MODIFIED - add generic param)
└── lib.rs (MODIFIED - export new types)
```

---

## Migration Path

### For Existing Code (Zero Breaking Changes)

**Python Bindings (no changes required):**
```rust
// bindings/python/src/lib.rs
use ironbase_core::FileDatabase;  // Just use the type alias

#[pyclass]
pub struct IronBase {
    db: FileDatabase,  // Same as before, just explicit now
}
```

**Rust Users:**
```rust
// Before (still works)
let db = Database::open("data.mlite")?;

// After (same behavior, more explicit)
let db = FileDatabase::open("data.mlite")?;
// or
let db: Database<FileStorage> = Database::open("data.mlite")?;
```

### For New Test Code

```rust
// Fast in-memory tests
#[test]
fn test_query_performance() {
    let db = MemoryDatabase::memory();
    let coll = db.collection("users");

    // 100x faster than file-based tests!
    for i in 0..10000 {
        coll.insert_one(json!({"id": i})).unwrap();
    }
}
```

---

## Risks and Mitigation

### Risk 1: Generic Type Complexity

**Risk**: Generic parameters can make code harder to read
```rust
Collection<FileStorage> vs Collection
```

**Mitigation**:
- Provide type aliases: `FileCollection`, `MemoryCollection`
- Use default type parameters where possible
- Excellent rustdoc documentation

### Risk 2: Breaking Changes

**Risk**: Changing Collection signature breaks all usage sites

**Mitigation**:
- Use type aliases for backward compat
- Gradual migration over multiple commits
- Comprehensive testing before merging

### Risk 3: Performance Regression

**Risk**: Trait indirection adds overhead

**Mitigation**:
- Trait methods will likely inline
- Benchmark before/after
- Optimize hot paths if needed

---

## Success Metrics

### Code Quality
- ✅ Dependency Inversion Principle satisfied
- ✅ Single Responsibility: Storage vs Business Logic
- ✅ 100+ tests converted to MemoryStorage (10-100x faster)

### Testing
- ✅ Test suite runs in <5 seconds (down from ~15s)
- ✅ No filesystem pollution in unit tests
- ✅ Easier to write isolated tests

### Extensibility
- ✅ New storage backend can be added without modifying existing code
- ✅ Clear trait contract for implementations

---

## Timeline

| Step | Duration | Description |
|------|----------|-------------|
| 1. Define Storage trait | 2-3h | Design and implement trait interface |
| 2. FileStorage wrapper | 1-2h | Wrap StorageEngine with trait impl |
| 3. MemoryStorage impl | 2-3h | In-memory storage for testing |
| 4. Generic Collection | 2-3h | Add generic param, update all methods |
| 5. Update Database | 1-2h | Add generic param, type aliases |
| 6. Migrate tests | 1-2h | Convert tests to MemoryStorage |
| **Total** | **9-15h** | Aligns with REFACTORING_PLAN.md (8-12h estimate) |

---

## Dependencies

**New Crates**: None required (uses existing dependencies)

**Prerequisites**:
- Phase 1 complete ✅
- All tests passing ✅
- Clean git state ✅

---

## Future Work (Phase 3+)

After Phase 2, we can implement:

### Alternative Storage Backends

**S3Storage** (cloud storage):
```rust
pub struct S3Storage {
    client: S3Client,
    bucket: String,
}

impl Storage for S3Storage {
    // Implement trait using S3 API
}
```

**RedisStorage** (cache layer):
```rust
pub struct RedisStorage {
    client: RedisClient,
    fallback: Box<dyn Storage>,  // Fallback to file storage
}
```

**HybridStorage** (write-through cache):
```rust
pub struct HybridStorage {
    memory: MemoryStorage,  // Fast reads
    file: FileStorage,      // Persistent writes
}
```

---

## Appendix: Design Alternatives Considered

### Alternative 1: Multiple Small Traits

**Considered:**
```rust
trait DocumentReader { ... }
trait DocumentWriter { ... }
trait MetadataManager { ... }

impl DocumentReader + DocumentWriter + MetadataManager for FileStorage { ... }
```

**Rejected**: Too many trait bounds, harder to use

### Alternative 2: Enum-Based Dispatch

**Considered:**
```rust
enum Storage {
    File(FileStorage),
    Memory(MemoryStorage),
}
```

**Rejected**: Not extensible, violates Open/Closed Principle

### Alternative 3: Async Traits

**Considered:**
```rust
#[async_trait]
trait Storage {
    async fn read_document(...) -> Result<Value>;
}
```

**Rejected**: Not needed yet, adds complexity. Can add later if needed.

---

## Conclusion

Phase 2 will significantly improve the architecture by:
- ✅ Decoupling storage from business logic
- ✅ Enabling fast in-memory testing
- ✅ Following SOLID principles
- ✅ Preparing for future extensibility

**Estimated ROI**: Test suite speedup alone (10-100x) pays for the 8-12 hour investment within weeks.

**Ready to implement**: All design decisions made, risks identified, migration path clear.

---

**Document Version**: 1.0
**Created**: 2025-01-12
**Author**: Claude Code (Anthropic)
**Status**: READY FOR IMPLEMENTATION
