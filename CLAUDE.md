# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**IronBase** is a high-performance embedded NoSQL document database written in Rust with Python and C# bindings. It provides a MongoDB-compatible API with SQLite's simplicity - a single-file, serverless, zero-configuration database.

**Key Stats:**
- 554+ tests passing (unit + integration + doctest)
- Python (PyO3), C# (.NET 8), Rust APIs
- 18 query operators, 7 update operators
- Full aggregation pipeline with dot notation
- B+ tree indexing with compound index support
- LRU query cache with collection-level invalidation
- MCP server for AI assistant integration (HTTP + stdio modes)

## Build and Development Commands

```bash
# Initial setup
pip install maturin
maturin develop              # Development build with Python bindings

# Testing
cargo test -p ironbase-core                    # All Rust tests (554+)
cargo test -p ironbase-core -- test_name       # Single test by name
cargo test -p ironbase-core -- --nocapture     # Tests with stdout
just run-dev-checks                            # Full CI: fmt + clippy + tests

# .NET
cd IronBase.NET && dotnet test                 # C# tests

# MCP Server (separate workspace)
cd mcp-server && cargo build --release
cd mcp-server && cargo test

# Fuzz Testing (requires nightly)
cd ironbase-core/fuzz && cargo +nightly fuzz run fuzz_query_parser -- -max_total_time=60
cd ironbase-core/fuzz && cargo +nightly fuzz run fuzz_wal_bytes -- -max_total_time=60
cd ironbase-core/fuzz && cargo +nightly fuzz run fuzz_document_parse -- -max_total_time=60
cd ironbase-core/fuzz && cargo +nightly fuzz run fuzz_json_ops -- -max_total_time=60
```

## Architecture

### Workspace Structure

```
MongoLite/
├── ironbase-core/           # Pure Rust core library
│   └── src/
│       ├── database.rs      # DatabaseCore, durability modes
│       ├── collection_core/ # CRUD, aggregation, indexes
│       ├── query/           # Query operators (strategy pattern)
│       ├── aggregation.rs   # Pipeline stages + accumulators
│       ├── find_options.rs  # Projection, sort, limit, skip
│       ├── index.rs         # B+ tree indexes
│       ├── storage/         # Append-only storage engine
│       ├── transaction.rs   # ACD transactions
│       └── wal.rs           # Write-Ahead Log
├── bindings/python/         # PyO3 Python bindings
├── IronBase.NET/            # C# .NET 8 bindings
└── mcp-server/              # MCP protocol server (DOCJL editing)
```

### Core Module Responsibilities

**database.rs** - Database lifecycle and durability:
- `DatabaseCore<S: Storage + RawStorage>` - generic over storage backend
- `DatabaseCore::open(path)` - File-based storage (production)
- `DatabaseCore::<MemoryStorage>::open_memory()` - In-memory (testing, 10-100x faster)
- Durability modes: Safe (auto-commit), Batch, Unsafe

**collection_core/mod.rs** - All CRUD and query operations:
- insert_one/many, find/find_one/find_with_options, update_one/many, delete_one/many
- Aggregation pipeline: $match, $group, $project, $sort, $limit, $skip
- Index management: create_index, create_compound_index, drop_index, explain, hint
- Cursor/streaming: find_streaming() for memory-efficient iteration

**query/operators.rs** - Query engine (strategy pattern):
- Comparison: $eq, $ne, $gt, $gte, $lt, $lte, $in, $nin
- Logical: $and, $or, $not, $nor
- Element: $exists, $type
- Array: $all, $elemMatch, $size
- String: $regex

**aggregation.rs** - Pipeline stages and accumulators:
- Stages: MatchStage, GroupStage, ProjectStage, SortStage, LimitStage, SkipStage
- Accumulators: $sum, $avg, $min, $max, $first, $last
- Full dot notation support for nested fields

**find_options.rs** - Query options:
- Projection (include/exclude mode)
- Sort (single and multi-field, dot notation)
- Limit, Skip (pagination)
- All support dot notation for nested fields

**storage/** - Append-only storage engine:
- **file_storage.rs** - File-based persistence (.mlite files)
- **memory_storage.rs** - In-memory backend for testing
- **metadata.rs** - Metadata flush/load with dynamic offset (v2+ format)
- **compaction.rs** - Garbage collection for tombstones

**index.rs + btree.rs** - B+ tree indexing:
- Single-field indexes: `create_index("field", unique)`
- Compound indexes: `create_compound_index(["field1", "field2"], unique)`
- Automatic query optimization with index selection
- explain() and find_with_hint() for query planning

**transaction.rs + wal.rs** - ACD transactions:
- Write-Ahead Log with CRC32 checksums
- Crash recovery with automatic replay
- begin_transaction/commit_transaction/rollback_transaction

**query_cache.rs** - Query result caching:
- LRU cache with configurable capacity (default: 1000)
- Collection-level invalidation via reverse index
- Thread-safe with parking_lot::RwLock

### Storage File Format (.mlite)

**Version 2+ (dynamic metadata at end of file):**
```
┌─────────────────────────────────────┐
│  Header (256 bytes)                 │
│  - magic: "MONGOLTE", version=2     │
│  - metadata_offset, metadata_size   │
├─────────────────────────────────────┤
│  Document Data (append-only)        │
│  [u32 len][JSON bytes]...           │
├─────────────────────────────────────┤
│  Collection Metadata (JSON)         │  ← Dynamic offset (end of file)
│  - document_catalog, indexes        │
└─────────────────────────────────────┘
```

**Design notes:**
- Metadata at END of file prevents race conditions during concurrent reads
- No file truncation - append-only design for safety
- `flush_metadata()` uses idempotent offset calculation

## Implemented Features

### Query Operators (18)
- **Comparison**: $eq, $ne, $gt, $gte, $lt, $lte, $in, $nin
- **Logical**: $and, $or, $not, $nor
- **Element**: $exists, $type
- **Array**: $all, $elemMatch, $size
- **String**: $regex

### Update Operators (7)
- $set, $inc, $unset, $push, $pull, $addToSet, $pop
- All support dot notation for nested fields

### Aggregation
- **Stages**: $match, $group, $project, $sort, $limit, $skip
- **Accumulators**: $sum, $avg, $min, $max, $first, $last
- **Dot notation**: Full support everywhere

### Other Features
- FindOptions: projection, sort, limit, skip (all with dot notation)
- B+ tree indexes: single-field, compound, unique
- Query planning: explain(), find_with_hint()
- ACD transactions with WAL
- Durability modes: Safe/Batch/Unsafe
- In-memory mode for testing
- Cursor/streaming for large results
- JSON schema validation
- Storage compaction

## Development Guidelines

### When Adding Features
1. Implement in Rust first (ironbase-core)
2. Add PyO3 bindings (bindings/python/src/lib.rs)
3. Add C# bindings if needed (IronBase.NET)
4. Update tests
5. Use `just run-dev-checks` before committing

### Thread Safety
- `Arc<RwLock<StorageEngine>>` for shared storage (parking_lot::RwLock)
- Write lock: insert, update, delete
- Read lock: find, count, list_collections

### Error Handling
- Rust: `Result<T>` with `MongoLiteError` (thiserror)
- Python: Map to PyIOError, PyRuntimeError, PyValueError
- C#: Map to appropriate .NET exceptions

### C# / .NET Native Library Caching Issue
When rebuilding the Rust FFI library (`libironbase_ffi.so`), .NET caches the native library in `Demo/bin/Debug/net8.0/`. Even if you copy the updated library to `runtimes/linux-x64/native/`, .NET continues using the cached version.

**Solution**: Copy directly to the bin folder:
```bash
# After building the FFI library
cargo build --release -p ironbase-ffi

# Copy to .NET's actual load location
cp target/release/libironbase_ffi.so IronBase.NET/Demo/bin/Debug/net8.0/libironbase_ffi.so
```

This is especially important when debugging FFI issues - if debug logging doesn't appear, check that the correct library version is being loaded.

## MCP Server

The `mcp-server/` directory contains a standalone MCP (Model Context Protocol) server that exposes IronBase as an AI assistant tool.

### Running the MCP Server
```bash
# Build
cd mcp-server && cargo build --release

# HTTP mode (port 8080)
./target/release/mcp-ironbase-server

# stdio mode (for Claude Desktop integration)
./target/release/mcp-ironbase-server --stdio
```

### Key MCP Tools
- `insert_one`, `insert_many` - Insert documents
- `find`, `find_one` - Query documents
- `update_one`, `update_many` - Update documents
- `delete_one`, `delete_many` - Delete documents
- `aggregate` - Run aggregation pipelines
- `create_index`, `drop_index` - Index management
- `schema_get`, `schema_set` - JSON schema validation
- `db_stats` - Database statistics

### Testing HTTP Mode
```bash
# Health check
curl http://127.0.0.1:8080/health

# MCP request
curl -X POST http://127.0.0.1:8080/mcp \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}'
```

## Testing Strategy

- **Test first** approach always
- Rust unit tests: `cargo test -p ironbase-core` (554+ tests)
- Property tests: proptest in `ironbase-core/tests/property_tests.rs`
- Integration tests: `ironbase-core/tests/`
- Python tests: `test_*.py`, `run_all_tests.py`
- C# tests: `IronBase.NET/src/IronBase.Tests/`
- MCP tests: `cd mcp-server && cargo test`

## Quick Reference

### Creating Tests with MemoryStorage (fast, no files)
```rust
use ironbase_core::{DatabaseCore, storage::MemoryStorage};

let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
let coll = db.collection("test").unwrap();
// ... test code - no cleanup needed
```

### Dot Notation for Nested Fields
```rust
// Query
coll.find(&json!({"address.city": "NYC"}))?;

// Update
coll.update_one(
    &json!({"name": "Alice"}),
    &json!({"$set": {"address.city": "Boston"}})
)?;

// Aggregation
coll.aggregate(&json!([
    {"$group": {"_id": "$address.city", "count": {"$sum": 1}}}
]))?;

// Sort
let options = FindOptions::new().with_sort(vec![("address.zip".to_string(), 1)]);
coll.find_with_options(&json!({}), options)?;
```

### Creating Compound Indexes
```rust
collection.create_compound_index(
    vec!["country".to_string(), "city".to_string()],
    false  // unique
)?;
```

## Key Dependencies

- **serde/serde_json**: Serialization
- **parking_lot**: Fast RwLock
- **pyo3**: Python bindings
- **maturin**: Build Python wheels
- **ahash/dashmap**: Fast hashing
- **thiserror**: Error handling
