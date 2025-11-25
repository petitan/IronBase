# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**IronBase** is a lightweight embedded NoSQL document database written in Rust with Python bindings via PyO3. It provides a MongoDB-like API with SQLite's simplicity - a single-file, serverless, zero-configuration database.

## Build and Development Commands

```bash
# Initial setup
pip install maturin
maturin develop              # Development build with Python bindings

# Testing
cargo test -p ironbase-core                    # All Rust unit tests
cargo test -p ironbase-core -- test_name       # Single test by name
cargo test -p ironbase-core -- --nocapture     # Tests with stdout
just run-dev-checks                            # Full CI check: fmt + clippy + tests

# MCP Server (separate workspace)
cd mcp-server && cargo build --release
cd mcp-server && cargo test

# Python integration tests
source venv/bin/activate && python test_python_auto_commit.py
```

## Architecture

### Workspace Structure

The project is a Cargo workspace with the MCP server excluded (separate build):

```
MongoLite/
├── ironbase-core/           # Pure Rust core library
├── bindings/python/         # PyO3 Python bindings
└── mcp-server/              # MCP protocol server (excluded from workspace)
```

### Core Module Responsibilities

**database.rs** - Database lifecycle and durability:
- `DatabaseCore<S: Storage + RawStorage>` - generic over storage backend
- `DatabaseCore::open(path)` - File-based storage (production)
- `DatabaseCore::<MemoryStorage>::open_memory()` - In-memory storage (testing, 10-100x faster)
- Durability modes: Safe (auto-commit), Batch, Unsafe
- Transaction management and WAL recovery

**collection_core/mod.rs** - All CRUD and query operations:
- insert_one/many, find/find_one, update_one/many, delete_one/many
- Aggregation pipeline ($match, $group, $project, $sort, $limit, $skip)
- Index management (create_index, create_compound_index, drop_index, explain, hint)
- Cursor/streaming: find_streaming() returns FindCursor for memory-efficient iteration

**query.rs + query/operators.rs** - Query engine with strategy pattern:
- Comparison: $eq, $ne, $gt, $gte, $lt, $lte, $in, $nin
- Logical: $and, $or, $not, $nor
- Element: $exists, $type | Array: $all, $elemMatch, $size | Regex: $regex

**storage/** - Append-only storage engine:
- **traits.rs** - Storage trait hierarchy:
  - `Storage` - High-level document operations (read/write Document)
  - `RawStorage: Storage` - Low-level byte operations (offsets, catalog)
- **file_storage.rs** - File-based persistence (.mlite files)
- **memory_storage.rs** - In-memory backend (implements both Storage + RawStorage)
- **compaction.rs** - Garbage collection for tombstoned documents

**index.rs + btree.rs** - B+ tree indexing:
- Single-field indexes: `create_index("field", unique)`
- Compound indexes: `create_compound_index(["field1", "field2"], unique)`
- Automatic query optimization with index selection

**transaction.rs + wal.rs** - ACD transactions:
- Write-Ahead Log with CRC32 checksums
- Crash recovery with automatic replay

### MCP Server Architecture

The MCP server (`mcp-server/`) implements Model Context Protocol for DOCJL document editing:

```
mcp-server/src/
├── main.rs              # HTTP server + JSON-RPC handler
├── commands.rs          # MCP command implementations
├── domain/              # DOCJL business logic (block, document, label, validation)
├── host/                # Security (auth, rate-limit) and audit logging
└── adapters/            # Storage adapters (ironbase integration)
```

**Key MCP tools**: list_documents, get_document, search_blocks, insert_block, update_block, delete_block, get_section, estimate_tokens

### Storage File Format (.mlite)

```
┌─────────────────────────────────────┐
│  Header (128 bytes)                 │
│  - magic: "MONGOLTE", version       │
├─────────────────────────────────────┤
│  Collection Metadata (JSON)         │
│  - document_catalog, indexes        │
├─────────────────────────────────────┤
│  Document Data (append-only)        │
│  [u32 len][JSON bytes]...           │
└─────────────────────────────────────┘
```

## Development Guidelines

### When Adding Features
1. Implement in Rust first (ironbase-core)
2. Add PyO3 bindings (bindings/python/src/lib.rs)
3. Update tests (cargo test + Python tests)
4. Use `just run-dev-checks` before committing

### Thread Safety
- `Arc<RwLock<StorageEngine>>` for shared storage (parking_lot::RwLock)
- Write lock: insert, update, delete
- Read lock: find, count, list_collections

### Error Handling
- Rust: `Result<T>` with `MongoLiteError` (thiserror)
- Python: Map to PyIOError, PyRuntimeError, PyValueError

## Testing Strategy

- **Test first** approach
- Rust unit tests: `cargo test -p ironbase-core`
- Property tests: proptest in `ironbase-core/tests/property_tests.rs`
- Python integration: `test_*.py` files in project root
- MCP tests: `cd mcp-server && cargo test`

## Key Dependencies

- **serde/serde_json**: Serialization
- **parking_lot**: Fast RwLock
- **pyo3**: Python bindings
- **maturin**: Build Python wheels from Rust
- **ahash/dashmap**: Fast hashing

## Quick Reference

### Creating Tests with MemoryStorage (fast, no files)
```rust
use ironbase_core::{DatabaseCore, storage::MemoryStorage};

let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
let coll = db.collection("test").unwrap();
// ... test code - no cleanup needed
```

### Using Cursors for Large Results
```rust
let mut cursor = collection.find_streaming(&json!({}))?;
while cursor.remaining() > 0 {
    let batch = cursor.next_chunk(100)?;
    // process batch
}
```

### Creating Compound Indexes
```rust
collection.create_compound_index(
    vec!["country".to_string(), "city".to_string()],
    false  // unique
)?;
```

## Current Test Coverage

- **Total**: 85%+ coverage
- **schema.rs**: 99%
- **aggregation.rs**: 96%
- **operators.rs**: 93%
- **collection_core/mod.rs**: 70%

Run coverage: `cargo llvm-cov -p ironbase-core`
