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
- **Memory limits** to prevent OOM (see AggregationLimits below)

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

### HeaderWriter - KRITIKUS INVARIÁNS

A `data_end_offset` mező mutatja, hogy a következő adat HOVA íródjon.
**TILOS közvetlenül módosítani!** Mindig a `HeaderWriter` metódusokat használd:

| Helyzet | Metódus |
|---------|---------|
| Document write után | `HeaderWriter::new(&mut header, &mut file).advance_after_write()` |
| Metadata flush után | `HeaderWriter::new(&mut header, &mut file).set_after_metadata(offset, size)` |
| Compaction | `write_compaction_header(&mut file, &header, offset, size)` |

**Miért fontos?**
- 7+ kritikus bug volt korábban mert valaki elfelejtette frissíteni
- A HeaderWriter **AUTOMATIKUSAN** számolja a helyes értéket
- Ha `data_end_offset` rossz → sparse hole vagy metadata felülírás

**Invariáns:**
```
Document write után:  data_end_offset = file.stream_position()
Metadata flush után:  data_end_offset = metadata_offset + metadata_size
```

**Fájlok:**
- `storage/mod.rs` - HeaderWriter struct, write_compaction_header()
- `storage/io.rs` - advance_after_write() használat
- `storage/metadata.rs` - set_after_metadata() használat
- `storage/compaction.rs` - write_compaction_header() használat

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
- **Memory limits**: OOM protection (see below)

### Aggregation Memory Limits (OOM Protection)

Aggregation pipelines have built-in memory safety limits to prevent OOM on large collections:

| Limit | Default | Description |
|-------|---------|-------------|
| `max_docs_without_match` | 100,000 | Max documents to scan without `$match` |
| `max_group_count` | 50,000 | Max unique groups in `$group` stage |
| `max_memory_mb` | 512 | Max estimated memory usage |

**When limits are triggered:**
- Error: `"Aggregation exceeded document limit: X documents processed without $match"`
- Error: `"Aggregation exceeded group limit: X unique groups"`

**Best practices to avoid hitting limits:**
1. **Always start with `$match`** - filter documents before `$group`
2. **Use low-cardinality group keys** - don't group by unique IDs
3. **Add indexes on `$match` fields** - reduces scan to indexed lookup

**Example - BAD (will hit limit on large collections):**
```json
[{"$group": {"_id": "$email", "count": {"$sum": 1}}}]
```

**Example - GOOD (filtered first):**
```json
[
  {"$match": {"date": {"$gt": "2024-01-01"}}},
  {"$group": {"_id": "$email", "count": {"$sum": 1}}}
]
```

**Custom limits (Rust API):**
```rust
use ironbase_core::aggregation::AggregationLimits;

let limits = AggregationLimits {
    max_docs_without_match: 50_000,
    max_group_count: 10_000,
    max_memory_mb: 256,
};
let results = collection.aggregate_with_limits(&pipeline, limits)?;

// Or use presets:
let limits = AggregationLimits::low_memory();  // 10K docs, 5K groups, 128MB
let limits = AggregationLimits::unlimited();   // No limits (use with caution!)
```

### Top-K Optimization (Sort + Limit)

When an aggregation pipeline has `$sort` followed by `$limit`, IronBase automatically uses a bounded heap algorithm instead of full sorting:

**Before (naive):** Sort all N documents → O(n log n) time, O(n) memory
**After (optimized):** Maintain heap of K elements → O(n log k) time, O(k) memory

**Example - Query that triggered OOM before optimization:**
```json
[
  {"$group": {"_id": "$from.email", "count": {"$sum": 1}}},
  {"$sort": {"count": -1}},
  {"$limit": 5}
]
```

With 50,000 unique groups:
- **Before:** Sort all 50K groups → ~50MB memory
- **After:** Heap of 5 elements → ~500 bytes memory

**How it works:**
1. Pipeline optimizer detects `$sort` → `$limit` pattern
2. `SortStage` receives limit hint
3. Uses `BinaryHeap` to track only top K elements
4. Final sort of K elements for correct ordering

**Key files:**
- `ironbase-core/src/aggregation/optimizer.rs` - Pattern detection
- `ironbase-core/src/aggregation/stages/sort_stage.rs` - Top-K implementation

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

### OOM Prevention (KRITIKUS!)

**TILOS minták - Soha ne csináld nagy kollekciókra:**

| Pattern | Probléma | Megoldás |
|---------|----------|----------|
| `docs.iter().map(\|d\| load(d)).collect::<Vec<_>>()` | Összes doc memóriában | Streaming: egyesével |
| `Vec<Vec<u8>>` összes doc-ból | GB-ok memóriában | Iterator/for loop |
| `.collect()` catalog-on filter nélkül | 78K+ doc memóriában | Limit vagy streaming |

**KÖTELEZŐ minták:**

1. **Streaming Document Loading** - Egy doc egyszerre:
```rust
for doc_id in doc_ids {
    let doc = load_one(doc_id)?;  // ← EGY doc memóriában
    process(doc);
    // doc felszabadul itt
}
```

2. **try_reserve() használata MINDEN nagy Vec allokáció előtt:**
```rust
let mut results = Vec::new();
results.try_reserve(count).map_err(|e| IronBaseError::OutOfMemory(...))?;
```

3. **Chunked Parallel Processing** - Ha párhuzamosítás kell:
```rust
const CHUNK_SIZE: usize = 1000;  // Max ~500MB memória
for chunk in catalog_entries.chunks(CHUNK_SIZE) {
    let batch = load_batch(chunk);  // Max 1000 doc
    process_parallel(batch);        // rayon par_iter
    // batch felszabadul itt
}
```

### Egységes Range Query API (KÖTELEZŐ!)

**2025-01-től a `range_query()` az EGYETLEN ajánlott belépési pont minden B+ tree range művelethez.**

```rust
use crate::index::{RangeQueryMode, RangeQueryResult, ScanOrder};

// ✅ HELYES: Count O(1) memóriával
let result = btree.range_query(
    &start, &end, true, true,
    RangeQueryMode::Count
);
let count = result.unwrap_count();

// ✅ HELYES: Scan limittel O(limit) memóriával
let result = btree.range_query(
    &start, &end, true, true,
    RangeQueryMode::Scan { skip: 0, limit: Some(10), order: ScanOrder::Asc }
);
let docs = result.unwrap_docs();

// ❌ TILOS: Régi metódusok közvetlen használata (wrapper-ek, OOM kockázat limit nélkül!)
// btree.range_scan() - NE HASZNÁLD új kódban!
// btree.range_scan_reversed_with_limit() - NE HASZNÁLD új kódban!
```

**Memória garanciák:**

| Művelet | Mód | Memória |
|---------|-----|---------|
| Count | `RangeQueryMode::Count` | O(1) |
| Scan + limit | `RangeQueryMode::Scan { limit: Some(k) }` | O(k) |
| Scan unlimited | `RangeQueryMode::Scan { limit: None }` | O(n) ⚠️ |

**Top-K dokumentum rendezés (sort + limit):**

```rust
use crate::collection_core::{topk_documents, compare_docs_by_sort};

// ✅ HELYES: Top-K O(k) memóriával
let sort_spec = vec![("age".to_string(), 1)]; // ASC
let top10 = topk_documents(docs.into_iter(), 0, 10, &sort_spec);

// ❌ TILOS: Teljes rendezés majd limit
// docs.sort_by(...); docs.truncate(10); - NE CSINÁLD!
```

**Törölt dead code (e445b44e):**
- `scan_documents_via_catalog()` - törölve
- `batch_read_documents_by_ids()` - törölve
- `count_live_docs_from_ids()` - törölve
- `parallel.rs` modul - törölve

**Korábbi OOM hibák (tanulság):**
- `4904ccc9` - scan_documents_via_catalog() összes doc betöltése
- `567e0d11` - aggregation pipeline összes doc memóriában
- `e0001bbe` - count_with_scan párhuzamos verzió → chunked parallel fix
- `49f27a77` - update_one bulk load → streaming fix
- `88f0a79c` - update_many bulk load → streaming fix
- `e445b44e` - range_query + Top-K egységesítés

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

Részletes dokumentáció: **`mcp-server/README.md`**

```bash
# Build & Run
cd mcp-server && cargo build --release
./target/release/mcp-ironbase-server          # HTTP mode (port 8080)
./target/release/mcp-ironbase-server --stdio  # stdio mode (Claude Desktop)
```

**Környezeti változók:**
- `IRONBASE_PATH` - Adatbázis fájl útvonala
- `IRONBASE_ADMIN_KEY` - Admin kulcs rendszer műveletekhez
- `IRONBASE_PORT` - HTTP port (default: 8080)

**Gyakori MCP cím:** 192.168.0.136:8080

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

Részletes dokumentáció: **`ironbase-backup/README.md`**

```bash
# Full backup futó adatbázisról
ironbase-backup backup --db /path/to/data.mlite --output ./backups --full

# Split backup (>10 GB adatbázisokhoz)
ironbase-backup backup --db /path/to/data.mlite --output ./backups --split 2G

# Restore
ironbase-backup restore --dir ./backups --output /path/to/restored.mlite
```

**Lock-free működés:** Az append-only storage garantálja, hogy `data_end_offset`-ig minden adat immutable → backup olvashat LOCK NÉLKÜL.

## Durability

| Mode | fsync | Sebesség | Adatvesztés crash-nél |
|------|-------|----------|----------------------|
| **Safe** (default) | ✅ | ~1,000-5,000 op/sec | 0 |
| **Batch** | ✅ | ~20,000-50,000 op/sec | Max N művelet |
| **Unsafe** | ❌ | ~50,000-100,000 op/sec | Minden uncommitted |

**Crash safety:** WAL + metadata snapshot → automatikus recovery `open()` hívásnál.

## Performance (MemoryStorage)

| Művelet | Sebesség |
|---------|----------|
| INSERT (batch) | ~200K ops/sec |
| INSERT (single) | ~50K ops/sec |
| FIND (indexed) | ~500K ops/sec |
| FIND (scan) | ~50K ops/sec |
| UPDATE | ~30K ops/sec |
| AGGREGATION | ~100K docs/sec |

**Fulltext index overhead:** +2x lassulás írási műveleteknél (~30µs/doc)

```bash
cargo test -p ironbase-core --release speed_benchmark_full_suite -- --nocapture --ignored
```