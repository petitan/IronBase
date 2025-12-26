# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**IronBase** is a high-performance embedded NoSQL document database written in Rust with Python and C# bindings. It provides a MongoDB-compatible API with SQLite's simplicity - a single-file, serverless, zero-configuration database.

**Key Stats:**
- 760+ tests passing (unit + integration + doctest)
- Python (PyO3), C# (.NET 8), Rust APIs
- 21 query operators (including $fuzzy), 7 update operators
- Full aggregation pipeline with dot notation
- B+ tree indexing with compound index and fuzzy index support
- LRU query cache with collection-level invalidation
- MCP server for AI assistant integration (HTTP + stdio modes)
- Fuzzy text search with Jaro-Winkler, Levenshtein, Damerau-Levenshtein algorithms
- Full-text search with TF-IDF scoring, stemming, and multi-language support (Hungarian, English, German)

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
IronBase/
├── ironbase-core/           # Pure Rust core library
│   └── src/
│       ├── database.rs      # DatabaseCore, durability modes
│       ├── collection_core/ # CRUD, aggregation, indexes
│       ├── query/           # Query operators (strategy pattern)
│       ├── aggregation.rs   # Pipeline stages + accumulators
│       ├── find_options.rs  # Projection, sort, limit, skip
│       ├── index.rs         # B+ tree indexes
│       ├── storage/         # Append-only storage engine
│       ├── transaction.rs   # ACID transactions
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

**transaction.rs + wal.rs** - ACID transactions:
- **A**tomicity: WAL + rollback support
- **C**onsistency: Schema validation
- **I**solation: Read Committed (exclusive write lock, SQLite-style)
- **D**urability: fsync + WAL with CRC32 checksums
- Crash recovery with automatic replay
- begin_transaction/commit_transaction/rollback_transaction
- Only one write transaction at a time (5 sec timeout, 10ms polling)

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
- ACID transactions with WAL (Read Committed isolation)
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
- Rust: `Result<T>` with `IronBaseError` (thiserror)
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
- `index_create_fulltext` - Create full-text search indexes with language support
- `fulltext_search` - Execute full-text search with TF-IDF scoring and projection
- `schema_get`, `schema_set` - JSON schema validation
- `db_stats` - Database statistics
- `script_save`, `script_list`, `script_get`, `script_delete`, `script_run` - Rhai scripting
- `admin_apikey_create`, `admin_apikey_list`, `admin_apikey_revoke`, `admin_apikey_delete` - API key management
- `acl_list`, `acl_get`, `acl_set`, `acl_delete`, `acl_cleanup` - Access Control List management
- `listener_list`, `listener_get`, `listener_add`, `listener_delete`, `listener_enable`, `listener_disable` - Multi-interface configuration

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

### Access Control List (ACL)

Collection-level permission system based on client origin (interface type).

**Interface Types:**
| Type | Description | Example IPs |
|------|-------------|-------------|
| `localhost` | Loopback address | 127.0.0.1, ::1 |
| `internal` | Private network | 10.x.x.x, 172.16-31.x.x, 192.168.x.x |
| `external` | Public internet | Everything else |

**Permission Levels:**
| Permission | Includes | Operations |
|------------|----------|------------|
| `read` | - | find, count, aggregate, explain |
| `write` | read | insert, update, delete |
| `admin` | read, write | create/drop index, schema, compact |

**Default Permissions (no ACL defined):**
- `localhost`: all (read, write, admin)
- `internal`: read, write
- `external`: read only

**System Collections (`_system.*`):**
- Always localhost-only for write operations
- `_system.acl` - ACL rules storage
- `_system.api_keys` - API key storage
- `_system.scripts` - Rhai scripts (read allowed from internal/external)

**ACL Tools:**
```bash
# List all ACL rules
{"name": "acl_list"}

# Get ACL for specific collection
{"name": "acl_get", "arguments": {"collection": "users"}}

# Set ACL for collection (localhost only)
{"name": "acl_set", "arguments": {
  "collection": "users",
  "rules": [
    {"principal": "interface:localhost", "permissions": "all"},
    {"principal": "interface:internal", "permissions": "read,write"},
    {"principal": "interface:external", "permissions": "read"}
  ]
}}

# Delete ACL (revert to defaults)
{"name": "acl_delete", "arguments": {"collection": "users"}}

# Cleanup orphaned ACLs (collections that no longer exist)
{"name": "acl_cleanup"}
```

**Principal Types:**
- `interface:localhost` / `interface:internal` / `interface:external`
- `apikey:sk-xxx` - Match specific API key
- `ip:192.168.1.100` - Match exact IP
- `iprange:192.168.1.0/24` - Match IP range (CIDR)
- `anyone` - Match all clients

**Key Files:**
- `mcp-server/src/acl.rs` - ACL implementation
- `mcp-server/src/tools.rs:2358-2520` - ACL tool handlers

### JSON Schema Validation

Collection-level document validation using JSON Schema.

**Schema Tools:**
```bash
# Set schema for collection
{"name": "schema_set", "arguments": {
  "collection": "users",
  "schema": {
    "type": "object",
    "required": ["name", "email"],
    "properties": {
      "name": {"type": "string"},
      "email": {"type": "string", "pattern": "^[^@]+@[^@]+$"},
      "age": {"type": "integer"},
      "tags": {"type": "array", "minItems": 1, "maxItems": 10}
    }
  }
}}

# Get schema for collection
{"name": "schema_get", "arguments": {"collection": "users"}}
```

**Supported Constraints:**
| Constraint | Description | Example |
|------------|-------------|---------|
| `type` | Data type validation | `"string"`, `"integer"`, `"number"`, `"boolean"`, `"array"`, `"object"` |
| `required` | Required fields list | `["name", "email"]` |
| `properties` | Field type definitions | `{"name": {"type": "string"}}` |
| `pattern` | Regex pattern (strings) | `"^[A-Z][a-z]+$"` |
| `enum` | Allowed values list | `["active", "inactive", "pending"]` |
| `minItems` | Min array length | `1` |
| `maxItems` | Max array length | `100` |

**Validation Errors:**
```json
// Insert with missing required field
{"name": "insert_one", "arguments": {
  "collection": "users",
  "document": {"name": "Alice"}  // missing "email"
}}
// Error: "Validation error: Field 'email' is required"

// Insert with wrong type
{"name": "insert_one", "arguments": {
  "collection": "users",
  "document": {"name": "Alice", "email": "a@b.com", "age": "thirty"}
}}
// Error: "Validation error: Field 'age' type mismatch: expected integer"

// Insert with pattern mismatch
{"name": "insert_one", "arguments": {
  "collection": "users",
  "document": {"name": "Alice", "email": "invalid-email"}
}}
// Error: "Validation error: Field 'email' does not match pattern"
```

**Rust Usage:**
```rust
use ironbase_core::DatabaseCore;
use serde_json::json;

let db = DatabaseCore::open("data.mlite")?;
let coll = db.collection("users")?;

// Set schema
coll.set_schema(json!({
    "type": "object",
    "required": ["name"],
    "properties": {
        "name": {"type": "string"},
        "age": {"type": "integer"}
    }
}))?;

// Get schema
let schema = coll.get_schema()?;

// Clear schema (allow any document)
coll.clear_schema()?;
```

**Key Files:**
- `ironbase-core/src/collection_core/schema.rs` - Schema validation logic

### Listeners (Multi-Interface Support)

Configure multiple HTTP/HTTPS endpoints for the MCP server.

**Use Cases:**
- Separate internal (HTTP) and external (HTTPS) interfaces
- Multiple ports with different TLS configurations
- Interface-specific ACL rules

**Listener Tools:**
```bash
# List all listeners
{"name": "listener_list"}

# Get specific listener
{"name": "listener_get", "arguments": {"id": "internal"}}

# Add HTTP listener
{"name": "listener_add", "arguments": {
  "id": "internal",
  "bind": "192.168.1.100:8080",
  "tls": false,
  "description": "Internal API endpoint"
}}

# Add HTTPS listener
{"name": "listener_add", "arguments": {
  "id": "external",
  "bind": "0.0.0.0:443",
  "tls": true,
  "cert_path": "/etc/ssl/certs/server.crt",
  "key_path": "/etc/ssl/private/server.key",
  "description": "Public HTTPS endpoint"
}}

# Enable/disable listener
{"name": "listener_enable", "arguments": {"id": "external"}}
{"name": "listener_disable", "arguments": {"id": "internal"}}

# Delete listener
{"name": "listener_delete", "arguments": {"id": "old-listener"}}
```

**ListenerConfig Fields:**
| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `id` | string | Yes | Unique identifier |
| `bind` | string | Yes | Address:port (e.g., "0.0.0.0:8080") |
| `tls` | bool | No | Enable HTTPS (default: false) |
| `cert_path` | string | If tls=true | Path to TLS certificate |
| `key_path` | string | If tls=true | Path to TLS private key |
| `enabled` | bool | No | Active status (default: true) |
| `description` | string | No | Human-readable description |

**Storage:** `_system.listeners` collection (localhost-only write access)

**ACL Integration:**
Listeners integrate with ACL - each interface type (localhost/internal/external) can have different permissions per collection.

**Key Files:**
- `mcp-server/src/listener.rs` - Listener configuration and management
- `mcp-server/src/tools.rs:2538-2690` - Listener tool handlers

### System Collections (`_system.*`)

Protected collections for internal server configuration.

**Available System Collections:**
| Collection | Purpose | Read Access | Write Access |
|------------|---------|-------------|--------------|
| `_system.scripts` | Rhai script storage | localhost, internal, external | localhost only |
| `_system.script_versions` | Script version history | localhost, internal, external | localhost only |
| `_system.api_keys` | API key storage | localhost only | localhost only |
| `_system.acl` | ACL rules | localhost only | localhost only |
| `_system.listeners` | Listener configuration | localhost only | localhost only |

**Protection Features:**
- Hidden from `collection_list` output (filtered by `_system.` prefix)
- Protected from `collection_drop` (requires `admin_drop_protected`)
- Write operations restricted to localhost interface
- `_system.scripts` readable from internal/external for script execution

**Admin Tools for System Collections:**
```bash
# List ALL collections including system
{"name": "admin_list_all_collections"}

# Create new system collection
{"name": "admin_create_system_collection", "arguments": {
  "admin_key": "your-admin-key",
  "name": "_system.custom"
}}

# Drop protected collection (DANGEROUS)
{"name": "admin_drop_protected", "arguments": {
  "admin_key": "your-admin-key",
  "collection": "_system.old_data"
}}

# Set collection flags
{"name": "admin_set_collection_flags", "arguments": {
  "admin_key": "your-admin-key",
  "collection": "users",
  "hidden": true,
  "protected": true
}}
```

**Collection Flags:**
| Flag | Effect |
|------|--------|
| `hidden` | Excluded from `collection_list` |
| `protected` | Cannot be dropped with `collection_drop` |

**Key Files:**
- `mcp-server/src/adapter.rs:56-75` - System collection constants
- `mcp-server/src/acl.rs:334-370` - System collection ACL rules

**System Collection JSON Schemas:**

All system collections have strict JSON schema validation enforced automatically on startup.

| Collection | Required Fields | Key Patterns |
|------------|-----------------|--------------|
| `_system.scripts` | `_id`, `code`, `version`, `tags`, `dependencies` | `_id`: `^[a-zA-Z_][a-zA-Z0-9_-]{0,63}$` |
| `_system.script_versions` | `script_name`, `version`, `code`, `created_at`, `tags`, `dependencies` | `script_name`: same as above |
| `_system.api_keys` | `_id`, `key`, `name`, `created_at`, `enabled` | `key`: `^sk-[a-zA-Z0-9]{32,64}$` |
| `_system.acl` | `collection`, `rules` | `collection`: `^[a-zA-Z_*][a-zA-Z0-9_.*-]{0,127}$` |
| `_system.listeners` | `_id`, `bind` | `bind`: `^[0-9a-fA-F.:]+:[0-9]{1,5}$` |

**Schema Implementation:**
- Schemas defined in `mcp-server/src/adapter.rs:77-208`
- Applied automatically via `ensure_system_collections()` on adapter initialization
- Invalid documents are rejected with detailed validation errors

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

### Full-Text Search (TF-IDF)
```rust
// Create fulltext index with language support
// Languages: "hungarian", "english", "german", "none"
coll.create_fulltext_index(
    "content".to_string(),
    "hungarian",      // language for stemming and stop words
    None,             // min_word_length (default: 2)
    None              // accent_folding (default: true)
)?;

// Basic fulltext search
let results = coll.fulltext_search(
    "content",        // field
    "király",         // query
    Some(10),         // limit
    None,             // skip
    None,             // min_score
    None              // projection
)?;
// Returns: Vec<(Document, score, matched_tokens)>

// Search with projection (reduces response size for large documents)
let mut projection = HashMap::new();
projection.insert("title".to_string(), 1);
projection.insert("_id".to_string(), 1);
let results = coll.fulltext_search(
    "content", "király", Some(10), None, None,
    Some(projection)  // Only return title and _id
)?;

// Exclude large fields
let mut proj = HashMap::new();
proj.insert("full_text".to_string(), 0);  // Exclude full_text field
let results = coll.fulltext_search("content", "query", Some(10), None, None, Some(proj))?;
```

**Features:**
- TF-IDF scoring (term frequency × inverse document frequency)
- Hungarian, English, German stop words
- Snowball stemming (15+ languages via rust-stemmers)
- Unicode accent folding (áéíóú → aeiou)
- Pagination (limit/skip) and min_score filtering
- Projection support for reduced response size

**MCP Usage:**
```json
// Create index
{"name": "index_create_fulltext", "arguments": {
  "collection": "articles",
  "field": "content",
  "language": "hungarian"
}}

// Search with projection
{"name": "fulltext_search", "arguments": {
  "collection": "articles",
  "field": "content",
  "query": "király",
  "limit": 10,
  "projection": {"title": 1, "_id": 1}
}}
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

### Metadata WAL Crash Safety

A metadata változások is WAL-ba kerülnek a crash-safe recovery érdekében:

**Write Path (storage/mod.rs:334-349):**
```
flush():
  1. log_metadata_to_wal()  ← MetadataSnapshot WAL entry
  2. wal.flush()            ← fsync WAL
  3. flush_metadata()       ← Write to .mlite file
  4. file.sync_all()        ← fsync .mlite
  5. wal.clear()            ← Clear WAL (success)
```

**Recovery Path (storage/mod.rs:207-265):**
```
open() → load_metadata() fails:
  1. Detect corruption (NOT magic number - that's unrecoverable)
  2. recover_metadata_from_wal() ← Find latest MetadataSnapshot
  3. If no WAL → rebuild_from_documents() ← Document scan fallback
```

**WAL Entry Types (wal/entry.rs):**
- `MetadataSnapshot = 0x06` - Contains full collections HashMap + data_end_offset

**Key Files:**
- `storage/mod.rs:149-153` - MetadataWALEntry struct
- `storage/mod.rs:309-332` - log_metadata_to_wal()
- `storage/mod.rs:370-461` - recover_metadata_from_wal()
- `storage/mod.rs:463-605` - rebuild_from_documents()

### Teljesítmény

MCP HTTP benchmark (~64 insert/sec) bontása:
```
15ms/insert breakdown:
├── curl + HTTP round-trip: ~12-14ms (BOTTLENECK)
├── JSON parse: ~0.5ms
└── fsync + disk I/O: ~0.5-1ms (NVMe SSD)
```

**Megjegyzés:** Az MCP szerver teljesítményét a HTTP overhead dominálja, NEM az fsync!

## Performance Benchmarks

### Fulltext Index Write Overhead

A fulltext index fenntartása extra költséggel jár írási műveleteknél. Az alábbi mérések 10,000 dokumentummal készültek, MemoryStorage-al (tiszta CPU overhead, disk I/O nélkül):

| Művelet | FTS nélkül | FTS-sel | Lassulás | Overhead/doc |
|---------|-----------|---------|----------|--------------|
| **INSERT** | 215ms (46K ops/s) | 516ms (19K ops/s) | **2.40x** | ~30µs |
| **UPDATE** | 309ms (32K ops/s) | 586ms (17K ops/s) | **1.90x** | ~28µs |
| **DELETE** | 63ms (157K ops/s) | 133ms (75K ops/s) | **2.10x** | ~7µs |

**Mit jelent ez a gyakorlatban:**
- Egyedi `insert_one`: észrevehetetlen (~30µs << ~35ms hálózati latencia)
- 1000 dokumentum batch: +30ms overhead
- 10,000 dokumentum batch: +300ms overhead

**Az overhead összetevői:**
1. **INSERT**: Tokenizálás + stop words szűrés + TF-IDF számítás + inverted index frissítés
2. **UPDATE**: Régi tokenek törlése + új tokenek hozzáadása (dupla munka)
3. **DELETE**: Tokenek eltávolítása az inverted indexből

**Benchmark futtatása:**
```bash
cargo test -p ironbase-core --release speed_benchmark_fulltext_overhead -- --nocapture --ignored
```

### General Performance (100K documents)

```bash
cargo test -p ironbase-core --release speed_benchmark_full_suite -- --nocapture --ignored
```

Tipikus eredmények (MemoryStorage):
- **INSERT**: ~200K ops/sec (batch), ~50K ops/sec (single)
- **FIND (indexed)**: ~500K ops/sec
- **FIND (scan)**: ~50K ops/sec
- **UPDATE**: ~30K ops/sec
- **AGGREGATION**: ~100K docs/sec