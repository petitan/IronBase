# Phase 2 Continuation Guide - Steps 2-8

**Current Status**: Step 1 COMPLETE ✅
**Commit**: `3794d26 wip: Phase 2 Step 1 - Define Storage trait interface`

---

## Completed

### ✅ Step 1: Storage Trait Definition (DONE)

**Files Created:**
- `ironbase-core/src/storage/traits.rs` - Storage trait interface

**Key Changes:**
```rust
pub trait Storage: Send + Sync {
    fn write_document(&mut self, collection: &str, doc: &Value) -> Result<u64>;
    fn read_document(&self, collection: &str, id: &DocumentId) -> Result<Option<Value>>;
    fn scan_documents(&self, collection: &str) -> Result<Vec<Document>>;
    fn create_collection(&mut self, name: &str) -> Result<()>;
    fn get_collection_meta(&self, name: &str) -> Option<&CollectionMeta>;
    // + 5 more methods
}
```

---

## Remaining Steps (7)

### 🔄 Step 2: FileStorage Wrapper (1-2 hours)

**Goal**: Wrap existing StorageEngine with Storage trait

**Create**: `ironbase-core/src/storage/file_storage.rs`

**Implementation Sketch:**
```rust
use super::traits::Storage;
use super::mod::StorageEngine;

pub struct FileStorage {
    inner: StorageEngine,
}

impl FileStorage {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        Ok(FileStorage {
            inner: StorageEngine::open(path)?,
        })
    }
}

impl Storage for FileStorage {
    fn write_document(&mut self, collection: &str, doc: &Value) -> Result<u64> {
        // Delegate to self.inner
        // Need to:
        // 1. Get collection metadata
        // 2. Use io::write_data() to write document
        // 3. Update catalog
        // 4. Return offset
        todo!("Implement delegation to StorageEngine")
    }

    fn read_document(&self, collection: &str, id: &DocumentId) -> Result<Option<Value>> {
        // 1. Get collection metadata
        // 2. Look up document offset from catalog
        // 3. Use io::read_data() to read document
        // 4. Deserialize and return
        todo!()
    }

    fn scan_documents(&self, collection: &str) -> Result<Vec<Document>> {
        // Scan all documents in collection
        // Use StorageEngine's catalog to iterate
        todo!()
    }

    // ... implement other methods
}
```

**Challenges:**
- StorageEngine methods are private - may need to make some methods `pub(crate)`
- Need to understand catalog structure for read_document
- scan_documents needs to iterate over all documents efficiently

**Files to Modify:**
- Add `pub mod file_storage;` to `storage/mod.rs`
- Export `pub use file_storage::FileStorage;`

---

### 🔄 Step 3: MemoryStorage Implementation (2-3 hours)

**Goal**: Pure in-memory storage for testing

**Create**: `ironbase-core/src/storage/memory_storage.rs`

**Implementation Sketch:**
```rust
use super::traits::Storage;
use std::collections::HashMap;
use parking_lot::RwLock;

pub struct MemoryStorage {
    // Collection name -> List of documents
    collections: HashMap<String, Vec<Document>>,

    // Collection name -> Metadata
    metadata: HashMap<String, CollectionMeta>,

    // Auto-incrementing offset counter
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
    fn write_document(&mut self, collection: &str, doc: &Value) -> Result<u64> {
        // 1. Parse document to get/generate ID
        let document = Document::from_json(&serde_json::to_string(doc)?)?;

        // 2. Add to collection's document list
        let docs = self.collections
            .entry(collection.to_string())
            .or_insert_with(Vec::new);
        docs.push(document);

        // 3. Update metadata (document_count, last_id)
        let meta = self.metadata
            .entry(collection.to_string())
            .or_insert_with(|| CollectionMeta {
                name: collection.to_string(),
                document_count: 0,
                last_id: 0,
                // ... other fields
            });
        meta.document_count += 1;

        // 4. Return synthetic offset
        let offset = self.next_offset;
        self.next_offset += 1;
        Ok(offset)
    }

    fn read_document(&self, collection: &str, id: &DocumentId) -> Result<Option<Value>> {
        if let Some(docs) = self.collections.get(collection) {
            Ok(docs.iter()
                .find(|doc| doc.id == *id)
                .map(|doc| doc.to_json()))
        } else {
            Ok(None)
        }
    }

    fn scan_documents(&self, collection: &str) -> Result<Vec<Document>> {
        Ok(self.collections
            .get(collection)
            .map(|docs| docs.clone())
            .unwrap_or_default())
    }

    // ... implement remaining methods
}
```

**Benefits:**
- No file I/O - pure in-memory
- 10-100x faster than FileStorage
- Perfect for unit tests

**Files to Modify:**
- Add `pub mod memory_storage;` to `storage/mod.rs`
- Export `pub use memory_storage::MemoryStorage;`

---

### 🔄 Step 4: Generic Collection (2-3 hours)

**Goal**: Make Collection generic over Storage

**File to Modify**: `ironbase-core/src/collection_core.rs`

**Changes:**
```rust
// BEFORE
pub struct Collection {
    name: String,
    storage: Arc<RwLock<StorageEngine>>,  // ❌ Hardcoded
}

// AFTER
pub struct Collection<S: Storage> {
    name: String,
    storage: Arc<RwLock<S>>,  // ✅ Generic
}

impl<S: Storage> Collection<S> {
    pub fn new(name: String, storage: Arc<RwLock<S>>) -> Result<Self> {
        Ok(Collection { name, storage })
    }

    pub fn insert_one(&self, mut fields: HashMap<String, Value>) -> Result<DocumentId> {
        let mut storage = self.storage.write();

        // Generate document
        let doc_json = /* ... */;

        // Use Storage trait method
        storage.write_document(&self.name, &doc_json)?;

        // ... rest of logic
    }

    // Update ALL methods to use S: Storage trait methods
}
```

**Type Aliases** (add at end of file):
```rust
/// File-based collection (production)
pub type FileCollection = Collection<FileStorage>;

/// Memory-based collection (testing)
pub type MemoryCollection = Collection<MemoryStorage>;
```

**Challenges:**
- 20+ methods need updating
- Must preserve exact behavior
- All tests must still pass

---

### 🔄 Step 5: Generic Database (1-2 hours)

**Goal**: Make Database generic over Storage

**File to Modify**: `ironbase-core/src/database.rs`

**Changes:**
```rust
// BEFORE
pub struct Database {
    storage: Arc<RwLock<StorageEngine>>,
    collections: DashMap<String, Arc<Collection>>,
}

// AFTER
pub struct Database<S: Storage> {
    storage: Arc<RwLock<S>>,
    collections: DashMap<String, Arc<Collection<S>>>,
}

impl Database<FileStorage> {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let storage = Arc::new(RwLock::new(FileStorage::open(path)?));
        Ok(Database {
            storage,
            collections: DashMap::new(),
        })
    }
}

impl Database<MemoryStorage> {
    pub fn memory() -> Self {
        let storage = Arc::new(RwLock::new(MemoryStorage::new()));
        Database {
            storage,
            collections: DashMap::new(),
        }
    }
}

impl<S: Storage> Database<S> {
    pub fn collection(&self, name: &str) -> Result<Arc<Collection<S>>> {
        // ... existing logic, but returns Collection<S>
    }
}
```

**Type Aliases:**
```rust
pub type FileDatabase = Database<FileStorage>;
pub type MemoryDatabase = Database<MemoryStorage>;
```

---

### 🔄 Step 6: Migrate Tests (1-2 hours)

**Goal**: Convert unit tests to use MemoryStorage

**Example Migration:**

**Before:**
```rust
#[test]
fn test_insert_and_find() {
    let temp = tempfile::NamedTempFile::new().unwrap();
    let db = Database::open(temp.path()).unwrap();  // ❌ Slow file I/O
    // ... test logic
}
```

**After:**
```rust
#[test]
fn test_insert_and_find() {
    let db = Database::memory();  // ✅ Fast in-memory
    // ... test logic (unchanged)
}
```

**Strategy:**
1. Keep integration tests using FileStorage
2. Convert unit tests to MemoryStorage
3. Add new MemoryStorage-specific tests

---

### 🔄 Step 7: Full Test Suite (30 min)

**Commands:**
```bash
# Run all tests
cargo test --lib -p ironbase-core

# Should see:
# - All existing tests pass
# - Faster test execution
# - No filesystem pollution

# Build Python bindings
maturin build --release
pip install --force-reinstall target/wheels/*.whl
```

**Success Criteria:**
- ✅ 168+ tests passing
- ✅ Test suite runs in <5 seconds (down from ~15s)
- ✅ Python bindings build successfully

---

### 🔄 Step 8: Final Commit

**Commit Message:**
```
feat: Phase 2 - Storage Abstraction Complete

## Summary

Complete storage layer abstraction using trait-based design.
Enables dependency injection, fast in-memory testing, and
future extensibility for alternative backends (S3, Redis, etc.).

## New Implementations

- **FileStorage**: Production wrapper around StorageEngine
- **MemoryStorage**: Fast in-memory storage for testing

## Architecture Changes

- Collection<S: Storage>: Generic over storage backend
- Database<S: Storage>: Generic over storage backend
- Type aliases: FileCollection, MemoryCollection, FileDatabase, MemoryDatabase

## Benefits

- ✅ **10-100x faster tests** (MemoryStorage vs file I/O)
- ✅ **SOLID compliance** (Dependency Inversion Principle)
- ✅ **Future extensibility** (easy to add S3Storage, RedisStorage)
- ✅ **100% backward compatible** via type aliases

## Metrics

- Test suite: ~15s → <5s (66% speedup)
- Zero breaking changes
- 165+ tests passing

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>
```

---

## Quick Reference

### File Locations

```
ironbase-core/src/
├── storage/
│   ├── mod.rs (MODIFIED - exports)
│   ├── traits.rs (DONE - Storage trait)
│   ├── file_storage.rs (TODO - FileStorage wrapper)
│   └── memory_storage.rs (TODO - MemoryStorage impl)
├── collection_core.rs (TODO - make generic)
├── database.rs (TODO - make generic)
└── lib.rs (TODO - update exports)
```

### Key Imports to Add

```rust
// In collection_core.rs
use crate::storage::Storage;

// In database.rs
use crate::storage::{Storage, FileStorage, MemoryStorage};
```

---

## Estimated Timeline

| Step | Duration | Description |
|------|----------|-------------|
| 2. FileStorage | 1-2h | Wrap StorageEngine |
| 3. MemoryStorage | 2-3h | In-memory implementation |
| 4. Generic Collection | 2-3h | Add generic param to Collection |
| 5. Generic Database | 1-2h | Add generic param to Database |
| 6. Migrate tests | 1-2h | Convert to MemoryStorage |
| 7. Test suite | 0.5h | Run full tests |
| 8. Final commit | 0.5h | Commit everything |
| **Total** | **8.5-13h** | Full Phase 2 completion |

---

## Troubleshooting

### If Build Fails

```bash
# Check for type errors
cargo check --lib -p ironbase-core

# Build with details
cargo build --lib -p ironbase-core 2>&1 | less
```

### If Tests Fail

```bash
# Run specific test
cargo test --lib -p ironbase-core test_name -- --nocapture

# Run with backtrace
RUST_BACKTRACE=1 cargo test --lib -p ironbase-core
```

### If Generic Types Are Confusing

Use type aliases everywhere:
```rust
// Instead of Database<FileStorage>
use FileDatabase;

// Instead of Collection<MemoryStorage>
use MemoryCollection;
```

---

## Notes

- Step 1 is DONE ✅ (commit `3794d26`)
- All tests currently pass ✅
- PHASE2_DESIGN.md has full design details
- Can resume anytime from Step 2

---

**Last Updated**: 2025-01-12
**Status**: Step 1 Complete, Ready for Step 2
