# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**MongoLite** is a lightweight embedded NoSQL document database written in Rust with Python bindings via PyO3. It provides a MongoDB-like API with SQLite's simplicity - a single-file, serverless, zero-configuration database.

**Technology Stack:**
- **Backend**: Rust (core library, storage engine, query processing)
- **Python Binding**: PyO3 (Rust-Python bridge)
- **Build System**: Maturin (builds Python wheels from Rust)
- **Storage**: Append-only file format with memory-mapped I/O
- **Serialization**: JSON (serde_json) + BSON for documents

## Build and Development Commands

### Initial Setup
```bash
# Install Maturin (required for building)
pip install maturin

# Development build (debug, fast iteration)
maturin develop

# Release build (optimized)
maturin build --release

# Install from wheel
pip install target/wheels/mongolite-*.whl
```

### Testing
```bash
# Rust unit tests
cargo test

# Rust tests with release optimizations
cargo test --release

# Run Python example
python example.py

# Python code formatting/linting
ruff check .
ruff format .
```

### Benchmarking
```bash
# Run performance benchmarks
cargo bench
```

### Cleaning
```bash
# Clean Rust build artifacts
cargo clean

# Remove generated Python packages
rm -rf target/
```

## Architecture

### High-Level Structure
```
Python API (PyO3 bindings)
         ↓
Rust Core Library (CRUD operations, query engine)
         ↓
Storage Engine (memory-mapped I/O, file management)
         ↓
.mlite File (single-file database)
```

### Module Responsibilities

**lib.rs** - Main entry point, PyO3 module definition, `MongoLite` database class exposed to Python

**storage.rs** - Core storage engine:
- File format: Header (128 bytes) → Collection metadata → Document data → Indexes
- Append-only write strategy for data integrity
- Memory-mapped I/O for files < 1GB (falls back to regular I/O for larger files)
- Manages `Header` (magic number "MONGOLTE", version, page size) and `CollectionMeta` (document count, offsets, last_id)

**collection.rs** - Collection operations (CRUD):
- `insert_one()` / `insert_many()` - Implemented ✅
- `find()` / `find_one()` - Stub implementation, needs query engine 🚧
- `update_one()` / `update_many()` - Stub implementation 🚧
- `delete_one()` / `delete_many()` - Stub implementation 🚧
- `count_documents()` - Implemented ✅
- Handles Python dict ↔ JSON conversion via `python_to_json()`

**document.rs** - Document structure and ID generation:
- `DocumentId` enum: Int, String, or ObjectId
- Auto-incrementing ID generation based on `CollectionMeta.last_id`

**query.rs** - Query engine (MongoDB operators):
- Currently placeholder for future implementation
- Should handle: `$eq`, `$ne`, `$gt`, `$gte`, `$lt`, `$lte`, `$in`, `$nin`, `$and`, `$or`, `$exists`, `$regex`

**index.rs** - Indexing system:
- Placeholder for B-tree based indexing
- Future: automatic `_id` index, `create_index()`, unique indexes

**error.rs** - Error types using `thiserror`:
- `MongoLiteError` variants: IoError, Corruption, CollectionNotFound, CollectionExists, Serialization, etc.

### Storage File Format (.mlite)

```
┌─────────────────────────────────────┐
│  Header (128 bytes, bincode)        │
│  - magic: "MONGOLTE" (8 bytes)      │
│  - version: u32                     │
│  - page_size: u32 (default 4096)    │
│  - collection_count: u32            │
│  - free_list_head: u64              │
├─────────────────────────────────────┤
│  Collection Metadata (JSON, length-prefixed) │
│  [u32 len][JSON bytes]              │
│  Repeated for each collection       │
├─────────────────────────────────────┤
│  Document Data (append-only)        │
│  [u32 len][JSON bytes]              │
│  [u32 len][JSON bytes]              │
│  ...                                │
├─────────────────────────────────────┤
│  Index Data (future)                │
└─────────────────────────────────────┘
```

**Important**: Metadata is rewritten on every collection create/drop/update. Document data is append-only.

## Current MVP Status (v0.1.0)

### ✅ Implemented
- Database open/create
- Collection management (create, list, drop)
- `insert_one()` - single document insert with auto ID generation
- `insert_many()` - bulk insert
- `count_documents()` - returns collection document count
- Persistent file storage with header and metadata
- Python API via PyO3

### 🚧 Under Development (Next Steps)
- `find()` / `find_one()` - requires full scan implementation and query engine
- Query operators (`$gt`, `$lt`, `$in`, `$eq`, etc.)
- `update_one()` / `update_many()` - document modification
- `delete_one()` / `delete_many()` - document deletion
- Indexing (B-tree based)

### Known Limitations
- No transactions (only atomic single writes)
- No cursors (all results loaded into memory)
- No aggregation pipeline
- No replication or sharding
- Files > 1GB don't use memory-mapped I/O (performance impact)

## Development Guidelines

### When Adding Features

1. **Start with Rust implementation** in the appropriate module (collection.rs, query.rs, etc.)
2. **Add PyO3 bindings** in collection.rs `#[pymethods]` block
3. **Update metadata** if collection state changes (document count, offsets)
4. **Test with example.py** before considering complete
5. **Use Ruff** for Python code formatting and linting

### Thread Safety

The codebase uses:
- `Arc<RwLock<StorageEngine>>` - shared storage across collections
- `parking_lot::RwLock` - faster RwLock implementation
- Write lock needed for: insert, update, delete, metadata changes
- Read lock sufficient for: find, count, list_collections

### Error Handling

Always propagate errors properly:
- Rust: Use `Result<T>` with `MongoLiteError`
- Python bindings: Map to appropriate `PyErr` types (`PyIOError`, `PyRuntimeError`, `PyValueError`)

### Memory-Mapped I/O

Currently enabled for files < 1GB:
```rust
let mmap = if file.metadata()?.len() < 1_000_000_000 {
    unsafe { MmapOptions::new().map_mut(&file).ok() }
} else {
    None
};
```

Future optimization: use mmap for reads, update strategy for larger files.

## Testing Strategy

- **Rust unit tests**: `cargo test` - test storage engine, serialization, error handling
- **Python integration tests**: Run `example.py` to verify end-to-end workflows
- **Benchmarks**: `cargo bench` for performance regression testing

## Detailed Implementation Documentation

**IMPORTANT:** For detailed implementation plans with algorithms, pseudocode, and trade-off analysis, refer to these documents:

### Core Implementation Guides
- **`IMPLEMENTATION_UPDATE.md`** - Update operations ($set, $inc, $push, $pull, etc.)
  - Append-only vs. in-place strategy decision
  - All MongoDB update operators with pseudocode
  - Tombstone pattern for versioning
  - Edge cases and error handling

- **`IMPLEMENTATION_DELETE.md`** - Delete operations and compaction
  - Tombstone-based logical deletion
  - Full compaction algorithm (garbage collection)
  - Trigger strategies and performance analysis
  - Crash recovery mechanisms

- **`IMPLEMENTATION_INDEX.md`** - B+ tree indexing system
  - B+ tree vs. alternatives comparison
  - Node structure and file format
  - Insert/Search/Delete/Range-scan algorithms
  - Unique constraints and sparse indexes

- **`IMPLEMENTATION_QUERY_OPTIMIZER.md`** - Query optimization
  - Index selection heuristics
  - Cost-based optimization model
  - Query rewrite rules
  - Execution plan generation

### Reference Documentation
- **`ALGORITHMS.md`** - Complete algorithm reference
  - All critical algorithms with pseudocode
  - Complexity analysis (time/space/I/O)
  - Cross-references to implementation guides
  - Performance characteristics summary

### When to Consult These Docs
- **Before implementing** any CRUD operation beyond insert
- **When designing** query optimization logic
- **When debugging** performance issues
- **When making** architectural decisions about storage/indexing

## File Locations

- Rust source: `*.rs` files in root directory (flat structure, no src/ subdirectory)
- Documentation: `README.md`, `ARCHITECTURE.md`, `BUILD.md`, `PROJECT_OVERVIEW.md`, `START_HERE.md`, `SUMMARY.md`
- Implementation guides: `IMPLEMENTATION_*.md`, `ALGORITHMS.md`
- Build config: `Cargo.toml` (Rust), `pyproject.toml` (Python package)
- Python example: `example.py`
- test first modszer mindig