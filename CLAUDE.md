# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**IronBase** is a high-performance embedded NoSQL document database written in Rust with Python and C# bindings. It provides a MongoDB-compatible API with SQLite's simplicity - a single-file, serverless, zero-configuration database.

**Key Stats:**
- 744+ tests passing (unit + integration + doctest)
- Python (PyO3), C# (.NET 8), Rust APIs
- 21 query operators (including $fuzzy), 7 update operators
- Full aggregation pipeline with dot notation
- B+ tree indexing with compound index and fuzzy index support
- LRU query cache with collection-level invalidation
- MCP server for AI assistant integration (HTTP + stdio modes)
- Fuzzy text search with Jaro-Winkler, Levenshtein, Damerau-Levenshtein algorithms

## Build and Development Commands

```bash
# Initial setup
pip install maturin
maturin develop              # Development build with Python bindings

# Testing
cargo test -p ironbase-core                    # All Rust tests (744+)
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
- Shared IndexManager per collection (Arc<RwLock>) - prevents stale index state

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

**index.rs** - B+ tree indexing (IndexManager + BPlusTree):
- Single-field indexes: `create_index("field", unique)`
- Compound indexes: `create_compound_index(["field1", "field2"], unique)`
- Automatic query optimization with index selection
- explain() and find_with_hint() for query planning
- Unique indexes enforce constraint on null/missing values (MongoDB behavior)

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

### Query Operators (21)
- **Comparison**: $eq, $ne, $gt, $gte, $lt, $lte, $in, $nin
- **Logical**: $and, $or, $not, $nor
- **Element**: $exists, $type
- **Array**: $all, $elemMatch, $size
- **String**: $regex
- **Fuzzy**: $fuzzy (Jaro-Winkler, Levenshtein, Damerau-Levenshtein algorithms)
- **Wildcard**: $** (recursive descent - finds field at any depth)

### Update Operators (7)
- $set, $inc, $unset, $push, $pull, $addToSet, $pop
- All support dot notation for nested fields

### Aggregation
- **Stages**: $match, $group, $project, $sort, $limit, $skip
- **Accumulators**: $sum, $avg, $min, $max, $first, $last
- **Dot notation**: Full support everywhere

### Other Features
- FindOptions: projection, sort, limit, skip, include_total (all with dot notation)
- B+ tree indexes: single-field, compound, unique, fuzzy
- Query planning: explain(), find_with_hint()
- ACD transactions with WAL
- Durability modes: Safe/Batch/Unsafe (see Durability section below)
- In-memory mode for testing
- Cursor/streaming for large results
- JSON schema validation
- Storage compaction
- Fuzzy text search with configurable algorithms and thresholds

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

### Python Database Closure (GC Timing Issue)

When using Python bindings, you **must call `db.close()`** before reopening the same database file. Python's garbage collector does not immediately call Rust's `Drop` trait when you use `del db`.

**Symptom**: "IO error: failed to fill whole buffer" when reopening a database

**Root Cause**:
- `del db` only marks the object for garbage collection
- The Rust `Drop` (which calls `flush()`) runs later when GC runs
- If you reopen the database before GC runs, the old instance still holds the file and unflushed data

**Correct usage**:
```python
from ironbase import IronBase

# Create and use database
db = IronBase("/tmp/test.mlite")
db.insert_one("users", {"name": "Alice"})
db.close()  # REQUIRED before reopening!

# Now safe to reopen
db2 = IronBase("/tmp/test.mlite")
print(db2.count_documents("users", {}))  # Works correctly
```

**Alternative** (not recommended):
```python
import gc
del db
gc.collect()  # Forces GC to run Drop immediately
```

**Note**: This issue does not affect Rust code, where scopes trigger `Drop` deterministically.

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
- `index_create_fuzzy` - Create fuzzy text indexes
- `fuzzy_search` - Execute fuzzy text queries
- `schema_get`, `schema_set` - JSON schema validation
- `db_stats` - Database statistics
- `script_save`, `script_list`, `script_get`, `script_delete`, `script_run` - Rhai scripting
- `admin_apikey_create`, `admin_apikey_list`, `admin_apikey_revoke`, `admin_apikey_delete` - API key management

### Testing HTTP Mode
```bash
# Health check
curl http://127.0.0.1:8080/health

# MCP request
curl -X POST http://127.0.0.1:8080/mcp \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}'
```

### Rhai Scripting

The MCP server includes a Rhai scripting engine for server-side script execution. Scripts are stored persistently in the `_system.scripts` collection.

**Script Management Tools:**
- `script_save(name, code, description)` - Save or update a script
- `script_list()` - List all saved scripts
- `script_get(name)` - Get script code and metadata
- `script_delete(name)` - Delete a script
- `script_run(name, params)` - Execute a script with optional parameters

**Available DB Functions in Scripts:**
```rhai
db_find(collection, query)           // Find documents
db_find_one(collection, query)       // Find single document
db_insert_one(collection, doc)       // Insert document
db_update_one(collection, filter, update)   // Update one
db_update_many(collection, filter, update)  // Update many
db_delete_one(collection, filter)    // Delete one
db_delete_many(collection, filter)   // Delete many
db_count(collection, query)          // Count documents
db_aggregate(collection, pipeline)   // Aggregation pipeline
```

**Utility Functions:**
```rhai
base64_encode(string)    // Encode string to base64
base64_decode(base64)    // Decode base64 to string
print(message)           // Log message (captured in result.logs)
```

**Example Script:**
```rhai
// Create users with random ages
let names = ["Anna", "Bela", "Csaba"];
let count = 0;
for i in 0..params.count {
    let name = names[i % 3];
    let age = 18 + (i % 50);
    db_insert_one("users", #{ name: name, age: age });
    count += 1;
}
count  // Return value
```

**Security Limits:**
- Max execution time: 60 seconds
- Max operations: 1,000,000
- No file I/O or network access

### Security & Authentication

The MCP server supports API key authentication and HTTPS/TLS encryption (both optional).

**Configuration (config.toml):**
```toml
[server]
host = "0.0.0.0"
port = 8080

[database]
path = "ironbase_data.mlite"

[security]
require_api_key = true          # Enable API key authentication (default: false)
api_key_cache_ttl = 60          # Cache TTL in seconds (default: 60)

[tls]
enabled = true                  # Enable HTTPS (default: false)
cert_file = "/path/to/cert.pem"
key_file = "/path/to/key.pem"
```

**Environment Variables:**
- `IRONBASE_ADMIN_KEY` - Admin key for system table operations (create API keys, manage collections)
- `MCP_CONFIG` - Path to config.toml (default: "config.toml")

**API Key Management Tools (require admin_key):**
- `admin_apikey_create(name)` - Create new API key, returns full key (save it!)
- `admin_apikey_list()` - List all keys (masked preview)
- `admin_apikey_revoke(id)` - Disable an API key
- `admin_apikey_delete(id)` - Permanently delete an API key

**Using API Keys:**
```bash
# Via HTTP header (preferred)
curl -X POST http://127.0.0.1:8080/mcp \
  -H "Authorization: Bearer sk-your-api-key" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"find","arguments":{...}}}'

# Via JSON parameter (fallback)
curl -X POST http://127.0.0.1:8080/mcp \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"find","arguments":{"api_key":"sk-...","collection":"users"}}}'
```

**Notes:**
- API keys are stored in `_system.api_keys` collection
- Admin operations (admin_*) use `IRONBASE_ADMIN_KEY`, not API keys
- API key validation uses constant-time comparison to prevent timing attacks

## Testing Strategy

- **Test first** approach always
- Rust unit tests: `cargo test -p ironbase-core` (744+ tests)
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

### $fuzzy Operator (Fuzzy Text Search)
```rust
// Simple fuzzy search (default: jaro_winkler, threshold: 0.8)
coll.find(&json!({"name": {"$fuzzy": "john"}}))?;

// With options
coll.find(&json!({"name": {"$fuzzy": {
    "value": "john",
    "algorithm": "levenshtein",  // jaro_winkler | levenshtein | damerau_levenshtein
    "threshold": 0.7
}}}))?;

// Create fuzzy index for faster queries
coll.create_fuzzy_index("name".to_string(), FuzzyAlgorithm::JaroWinkler, 0.8)?;
```

### $** Wildcard Operator (Recursive Descent)
```rust
// Find "name" field at ANY depth in the document
coll.find(&json!({"$**.name": "Alice"}))?;

// With regex - find all documents where any "content" field matches
coll.find(&json!({"$**.content": {"$regex": "sqrt"}}))?;

// With comparison operators
coll.find(&json!({"$**.score": {"$gte": 85}}))?;

// Multiple matches - returns docs where ANY occurrence matches
coll.find(&json!({"$**.status": "active"}}))?;
```

**Notes:**
- Syntax: `$**.fieldName` (only simple field names, no nested paths)
- `$**.a.b` is INVALID - use separate queries or dot notation
- Cannot use indexes - always performs collection scan
- MAX_DEPTH=100 to prevent stack overflow
- Works with arrays: searches inside array elements too

## Key Dependencies

- **serde/serde_json**: Serialization
- **parking_lot**: Fast RwLock
- **pyo3**: Python bindings
- **maturin**: Build Python wheels
- **ahash/dashmap**: Fast hashing
- **thiserror**: Error handling
- 192.168.0.136 az mcp cime általában

## Release folyamat (FONTOS!)

### Verzió frissítés KÖTELEZŐ lépései

Minden feature/bugfix commit után **KÖTELEZŐ** a verzió frissítése:

1. **mcp-server verzió** (fő verzió): `mcp-server/Cargo.toml` → `version = "1.0.XX"`
2. **core verzió**: `Cargo.toml` (workspace) → `version = "0.3.X"`

### CI/CD folyamat és ISMERT KORLÁTOZÁS

```
[Push to master]
    ↓
[auto-tag.yml] - Cargo.toml verzió változás → tag létrehozás (v1.0.XX)
    ↓
[release.yml] - Tag push → build + GitHub Release létrehozás
    ↓
[PROBLÉMA!] A release.yml NEM TRIGGERELŐDIK automatikusan!
```

**Gyökérok (Root Cause):**
A GitHub Actions biztonsági korlátozása: ha egy workflow (auto-tag.yml) GITHUB_TOKEN-nel hoz létre tag-et, az NEM triggerel másik workflow-t (release.yml). Ez rekurzió elleni védelem.

### MANUÁLIS release létrehozás (amíg nincs PAT beállítva)

Verzió bump után KÖTELEZŐ manuálisan létrehozni a release-t:

```bash
# 1. Letölteni az artifact-ot a Windows build CI-ból
gh run download <RUN_ID> -n ironbase-mcp-server-windows -D /tmp/win-pkg

# 2. Átnevezni és release létrehozni
mv /tmp/win-pkg/mcp-ironbase-server.exe /tmp/win-pkg/mcp-ironbase-server-windows.exe
gh release create v1.0.XX \
  --title "IronBase MCP Server v1.0.XX" \
  --generate-notes \
  /tmp/win-pkg/mcp-ironbase-server-windows.exe \
  /tmp/win-pkg/install.ps1
```

### Ellenőrzés

```bash
# Release létrejött-e?
gh release list --limit 3

# Helyes verzió?
gh release view v1.0.XX
```

### Jövőbeli javítás

PAT (Personal Access Token) beállítása az auto-tag.yml-ben a GITHUB_TOKEN helyett → automatikus release triggerelés.

## Hot Backup

A backup rendszer **lock-free** módon működik, kihasználva az append-only storage előnyeit.

### Működési elv

```
1. Header olvasás (data_end_offset) - NINCS LOCK
2. Dokumentumok másolása data_end_offset-ig - NINCS LOCK
3. Header újraolvasás → concurrent_writes detektálás
```

### Miért biztonságos LOCK NÉLKÜL?

Az **append-only storage** garantálja:
- Dokumentumok SOHA nem módosulnak helyben (update = új doc + tombstone)
- `data_end_offset`-ig minden adat **IMMUTABLE**
- Backup közben írt új adatok → következő incremental backup-ba kerülnek
- Fsync garantálja, hogy lemezre írt adat konzisztens

```
DatabaseCore:     [doc1][doc2][doc3][NEW_DOC]
                   ↑               ↑
Backup reads:     |←── SAFE ──────→|
                  (immutable)       (new - next backup)
```

### Használat

```bash
# Full backup futó adatbázisról
ironbase-backup backup --db /path/to/data.mlite --output ./backups --full

# Incremental backup
ironbase-backup backup --db /path/to/data.mlite --output ./backups

# Restore
ironbase-backup restore --backup ./backups/backup_xxx.tar.zst --output /path/to/restored.mlite
```

### Lock architektúra (Windows kompatibilitás)

**Probléma:** Windows mandatory file lock-ok MINDEN hozzáférést blokkolnak (olvasást is!).

**Megoldás:** Külön lock fájl (.mlite.lock):

```
DatabaseCore:
  - .mlite fájl: dokumentumok (NINCS lock rajta!)
  - .mlite.lock fájl: exclusive lock (single-writer garantálás)

Backup tool:
  - .mlite fájl: szabadon olvasható
  - .mlite.lock: nem érinti
```

| Platform | DB file lock | Lock file lock | Backup olvashat? |
|----------|--------------|----------------|------------------|
| Linux    | Advisory     | Exclusive      | ✅ Igen |
| Windows  | -            | Exclusive      | ✅ Igen |

**Fájlok:**
- `ironbase-core/src/storage/mod.rs:171` - Lock file kezelés
- `ironbase-backup/src/backup.rs` - Lock-free backup

## Durability és fsync

### Durability Modes

| Mode | Leírás | fsync | Sebesség | Adatvesztés crash-nél |
|------|--------|-------|----------|----------------------|
| **Safe** (default) | Minden művelet után commit | ✅ Igen | ~1,000-5,000 op/sec | 0 |
| **Batch** | N művelet után commit | ✅ Igen | ~20,000-50,000 op/sec | Max N művelet |
| **Unsafe** | Nincs auto-commit | ❌ Nem | ~50,000-100,000 op/sec | Minden uncommitted |

### Safe Mode működése (default)

```
Insert/Update/Delete
    ↓
WAL Write (operation log)
    ↓
WAL fsync() ← KERNEL BUFFER → DISK
    ↓
Metadata flush
    ↓
Storage fsync() ← KERNEL BUFFER → DISK
    ↓
WAL clear
```

**Bizonyított:** Adat túléli a "crash"-t (process kill):
```rust
// 1. Insert, majd drop (no explicit close)
{ let db = DatabaseCore::open(path)?; db.insert_one(...)?; }
// 2. Reopen - adat MEGMARAD!
{ let db = DatabaseCore::open(path)?; assert_eq!(db.find(...).len(), 1); }
```

### fsync hívások helye

```rust
// WAL writer (wal/writer.rs:51)
self.file.sync_all()?;

// Storage commit (storage/mod.rs:297)
self.file.sync_all()?;

// Metadata flush (storage/metadata.rs:377)
file.sync_all()?;
```

### Teljesítmény

MCP HTTP benchmark (~64 insert/sec) bontása:
```
15ms/insert breakdown:
├── curl + HTTP round-trip: ~12-14ms (BOTTLENECK)
├── JSON parse: ~0.5ms
└── fsync + disk I/O: ~0.5-1ms (NVMe SSD)
```

**Megjegyzés:** Az MCP szerver teljesítményét a HTTP overhead dominálja, NEM az fsync!