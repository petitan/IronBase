# IronBase

**High-performance embedded NoSQL document database** with MongoDB-compatible query API.

Written in Rust. Single-file storage, zero-configuration, serverless. Bindings for Python and C#.

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Rust CI](https://github.com/petitan/IronBase/actions/workflows/rust.yml/badge.svg)](https://github.com/petitan/IronBase/actions/workflows/rust.yml)

## Table of Contents

- [Features](#features)
- [Quick Start](#quick-start)
- [Indexing](#indexing)
- [Search](#search)
- [Aggregation](#aggregation)
- [Transactions](#transactions)
- [Durability Modes](#durability-modes)
- [MCP Server](#mcp-server)
- [MCP Bridge](#mcp-bridge)
- [Backup CLI](#backup-cli)
- [TUI](#tui)
- [API Key Authentication](#api-key-authentication)
- [Environment Variables](#environment-variables)
- [Architecture](#architecture)
- [Building from Source](#building-from-source)
- [License](#license)

## Features

| Category | Details |
|----------|---------|
| **Storage** | Single `.mlite` file, append-only, zero-config |
| **Query Operators (25)** | `$eq` `$ne` `$gt` `$gte` `$lt` `$lte` `$in` `$nin` `$and` `$or` `$not` `$nor` `$exists` `$type` `$all` `$elemMatch` `$size` `$regex` `$fuzzy` `$text` `$startsWith` `$endsWith` `$contains` `$expr` `$**` |
| **Update Operators (7)** | `$set` `$inc` `$unset` `$push` `$pull` `$addToSet` `$pop` |
| **Aggregation** | 8 stages + 8 accumulators with Top-K optimization |
| **Indexes** | B+ tree, compound, case-insensitive, fuzzy, fulltext (TF-IDF), HNSW vector |
| **Search** | Fuzzy (Jaro-Winkler/Levenshtein/Damerau), fulltext (TF-IDF + stemming), RAG (FastText + HNSW), hybrid (RRF score fusion) |
| **Durability** | ACID transactions, WAL, crash recovery, 3 durability modes |
| **OOM Protection** | Dynamic RAM-based limits, streaming, `try_reserve()`, Top-K heap |
| **Languages** | Rust, Python (PyO3), C# (.NET 8 FFI) |
| **Tooling** | MCP server (HTTP/stdio, 94 tools), TUI, backup CLI, STDIO bridge |
| **Testing** | 2,000+ tests (unit, integration, property-based, fuzz) |

## Quick Start

### Python

```bash
pip install ironbase
```

```python
from ironbase import IronBase

db = IronBase("myapp.mlite")
users = db.collection("users")

# Insert
users.insert_one({"name": "Alice", "age": 30, "city": "NYC"})
users.insert_many([
    {"name": "Bob", "age": 25, "city": "LA"},
    {"name": "Carol", "age": 35, "city": "NYC"}
])

# Query with operators
adults = users.find({"age": {"$gte": 18}})
nyc_users = users.find({"city": "NYC", "age": {"$lt": 40}})

# Projection, sort, limit
results = users.find(
    {"city": "NYC"},
    projection={"name": 1, "age": 1, "_id": 0},
    sort=[("age", -1)],
    limit=10
)

# Update
users.update_one({"name": "Alice"}, {"$set": {"age": 31}})
users.update_many({"city": "NYC"}, {"$inc": {"visits": 1}})

# Delete
users.delete_one({"name": "Bob"})

# Aggregation
stats = users.aggregate([
    {"$match": {"age": {"$gte": 18}}},
    {"$group": {"_id": "$city", "count": {"$sum": 1}, "avgAge": {"$avg": "$age"}}},
    {"$sort": {"count": -1}}
])

db.close()
```

### Rust

```rust
use ironbase_core::database::DatabaseCore;
use ironbase_core::storage::StorageEngine;
use serde_json::json;

fn main() -> ironbase_core::error::Result<()> {
    let db = DatabaseCore::<StorageEngine>::open("myapp.mlite")?;

    // Insert
    db.insert_one("users", json!({"name": "Alice", "age": 30, "city": "NYC"}))?;

    // Query
    let results = db.find("users", &json!({"age": {"$gte": 18}}), None)?;

    // Update
    db.update_one("users", &json!({"name": "Alice"}), &json!({"$set": {"age": 31}}))?;

    // Aggregate
    let stats = db.aggregate("users", vec![
        json!({"$group": {"_id": "$city", "count": {"$sum": 1}}}),
    ])?;

    Ok(())
}
```

### C# (.NET 8)

```csharp
using IronBase;

using var db = new IronBaseClient("myapp.mlite");
var users = db.GetCollection("users");

// Insert
users.InsertOne(new { name = "Alice", age = 30, city = "NYC" });

// Query
var adults = users.Find(new { age = new { _gte = 18 } });

// Update
users.UpdateOne(
    new { name = "Alice" },
    new { _set = new { age = 31 } }
);

// Aggregation
var stats = users.Aggregate(new[] {
    new { _match = new { age = new { _gte = 18 } } },
    new { _group = new { _id = "$city", count = new { _sum = 1 } } }
});
```

## Indexing

IronBase supports 5 index types for different access patterns:

| Type | Use Case | Complexity | File Extension |
|------|----------|------------|----------------|
| **B+ Tree** | Equality, range queries | O(log n) lookup | `.idx` |
| **Compound** | Multi-field queries (prefix matching) | O(log n) lookup | `.idx` |
| **Fuzzy** | Similarity search (typo-tolerant) | O(n) scan | `.fzidx` |
| **Fulltext** | TF-IDF text search with stemming | O(terms) lookup | `.ftidx` |
| **HNSW Vector** | Nearest neighbor / semantic search | O(log n) approx | `.hnsw` |

```python
# B+ tree (single field)
users.create_index("email", unique=True)

# Compound index
users.create_compound_index(["country", "city"])

# Fuzzy index (Jaro-Winkler, Levenshtein, Damerau-Levenshtein)
users.create_fuzzy_index("name", algorithm="jaro_winkler", threshold=0.8)

# Fulltext index (Hungarian, English, German stemming)
articles.create_fulltext_index("content", language="hungarian")

# HNSW vector index
docs.create_hnsw_index("embedding", dim=300, metric="cosine")

# Query plan analysis
plan = users.explain({"email": "alice@example.com"})  # Shows IndexScan vs CollectionScan
```

All indexes support **lazy loading** — large indexes are loaded on-demand to reduce startup time and memory usage. Thresholds scale with available RAM (50 MB on <2 GB systems up to 500 MB on 32 GB+).

## Search

### Fuzzy Search

Three similarity algorithms with configurable threshold (0.0-1.0):

```python
# Via query operator (requires fuzzy index)
results = users.find({"name": {"$fuzzy": "jonh"}})  # Finds "John"

# Direct search with scoring
results = users.fuzzy_search("name", "jonh", threshold=0.7, limit=10)
```

### Fulltext Search (TF-IDF)

Tokenization + stemming + stop word removal. Languages: Hungarian, English, German.

```python
articles.create_fulltext_index("content", language="hungarian")
results = articles.fulltext_search("content", "adatbazis", limit=10)
```

### Text Operators

All support case-insensitive matching by default and work on array fields:

```python
users.find({"name": {"$startsWith": "Al"}})
users.find({"email": {"$endsWith": ".hu"}})
users.find({"bio": {"$contains": "Rust"}})
users.find({"content": {"$text": "embedded database"}})  # AND logic, stemmed
users.find({"content": {"$regex": "^iron.*db$"}})
```

### Wildcard Deep Match

Search a field at any nesting depth (max 100 levels):

```python
results = db.find("data", {"$**.name": "Alice"})  # Matches name at any level
```

### Hybrid Search (via MCP Server)

```
hybrid_search      → RRF fusion of fulltext + vector results with reranking
                     If 'vector' omitted → auto-embeds query (FastText/Ollama/OpenAI)
                     Supports flat (per-chunk) and grouped (per-document) response modes
```

| Parameter | Default | Description |
|-----------|---------|-------------|
| `search_mode` | `"balanced"` | Preset: `"balanced"` (0.5/0.5), `"semantic"` (0.8/0.2), `"keyword"` (0.2/0.8) |
| `mode` | `"and"` | Fulltext: `"and"` = all words required, `"or"` = any word |
| `rerank` | `true` | Phrase match (1.5x), keyword density (1.0–1.3x), title boost (1.5x), short penalty (0.8x) |
| `deduplicate` | `false` | Enable MMR diversity reranking |
| `mmr_lambda` | `0.7` | Relevance vs diversity: `1.0` = pure relevance, `0.0` = pure diversity |
| `merge_chunks` | `true` | Merge adjacent chunks from same document (overlap dedup) |
| `group_by_document` | `false` | Group results by source document with all relevant chunks |
| `filter` | - | MongoDB-style pre-filter |

**`group_by_document` mode:** When `true`, results are grouped by source document. Document selection uses AND (all query words somewhere in document), chunk retrieval uses OR (any query word). The `limit` applies to document count, not chunk count.

## Aggregation

### Pipeline Stages

| Stage | Description |
|-------|-------------|
| `$match` | Filter documents |
| `$group` | Group by field with accumulators |
| `$project` | Include/exclude/compute fields |
| `$sort` | Sort results |
| `$limit` | Limit output count |
| `$skip` | Skip N documents |
| `$unwind` | Deconstruct array field |
| `$count` | Count documents |

### Accumulators

`$sum` `$avg` `$min` `$max` `$first` `$last` `$push` `$addToSet`

### Example

```python
pipeline = [
    {"$match": {"status": "active"}},
    {"$unwind": "$tags"},
    {"$group": {
        "_id": "$tags",
        "count": {"$sum": 1},
        "avgScore": {"$avg": "$score"},
        "topUser": {"$first": "$name"}
    }},
    {"$sort": {"count": -1}},
    {"$limit": 10}
]
results = collection.aggregate(pipeline)
```

**Optimization:** `$sort` + `$limit` patterns use Top-K heap selection — O(k) memory instead of O(n).

## Transactions

ACID transactions with Read Committed isolation (SQLite-style exclusive write lock):

```python
db.begin_transaction()
try:
    db.insert_one_tx("accounts", {"_id": "A", "balance": 900})
    db.update_one_tx("accounts", {"_id": "B"}, {"$inc": {"balance": 100}})
    db.commit_transaction()
except:
    db.rollback_transaction()
```

All changes are journaled to WAL before commit. On crash, uncommitted transactions are rolled back on next startup.

## Durability Modes

| Mode | fsync | Throughput | Crash Loss |
|------|-------|------------|------------|
| **Safe** (default) | Every op | 1K-5K ops/sec | Zero |
| **Batch** | Every N ops | 20K-50K ops/sec | Max N ops |
| **Unsafe** | Manual | 50K-100K ops/sec | Since last checkpoint |

```python
# Python
db = IronBase("app.mlite", durability="batch", batch_size=100)
```

```rust
// Rust
let db = DatabaseCore::<StorageEngine>::open_with_durability(
    "app.mlite",
    DurabilityMode::Batch { batch_size: 100 },
)?;
```

**In-memory mode** (no disk I/O, ideal for tests):

```python
db = IronBase.open_memory()  # ~200K inserts/sec, ~500K indexed finds/sec
```

## MCP Server

IronBase includes an [MCP](https://modelcontextprotocol.io/) (Model Context Protocol) server with **94 tools** for AI assistant integration.

### Install

**Linux/macOS:**
```bash
curl -sSL https://github.com/petitan/IronBase/releases/latest/download/install.sh | sudo bash
```

**Windows (PowerShell as Admin):**
```powershell
Invoke-WebRequest -Uri https://github.com/petitan/IronBase/releases/latest/download/install.ps1 -OutFile install.ps1
Set-ExecutionPolicy -Scope Process -ExecutionPolicy Bypass
.\install.ps1
```

### Usage

```bash
# HTTP mode (default, port 8080)
mcp-ironbase-server

# Custom settings
mcp-ironbase-server -p 9090 -H 0.0.0.0 -d /path/to/data.mlite

# stdio mode (for Claude Desktop / ChatGPT Desktop)
mcp-ironbase-server --stdio

# System service management (requires admin/root)
mcp-ironbase-server install | uninstall | start | stop | status
```

### CLI Options

| Option | Env Variable | Default | Description |
|--------|-------------|---------|-------------|
| `-p, --port` | `MCP_PORT` | `8080` | Server port |
| `-H, --host` | `MCP_HOST` | `0.0.0.0` | Bind address |
| `-d, --db` | `IRONBASE_PATH` | Platform default | Database file path |
| `-c, --config` | `MCP_CONFIG` | `config.toml` | Config file |
| `--admin-key` | `IRONBASE_ADMIN_KEY` | - | Admin key for protected ops |
| `--stdio` | - | - | stdio transport mode |

**Default database paths:**
- Linux: `/var/lib/ironbase/ironbase_data.mlite`
- macOS: `/usr/local/var/ironbase/ironbase_data.mlite`
- Windows: `%LOCALAPPDATA%\IronBase\data\ironbase_data.mlite`

### Available Tools (94)

<details>
<summary><strong>Database & Collections (7)</strong></summary>

| Tool | Description |
|------|-------------|
| `db_open` | Open or create database file |
| `db_stats` | Get database metrics and statistics |
| `db_compact` | Reclaim disk space from tombstones |
| `db_checkpoint` | Force flush pending writes |
| `collection_list` | List all user collections |
| `collection_create` | Create empty collection |
| `collection_drop` | Delete collection and data |

</details>

<details>
<summary><strong>Document CRUD (11)</strong></summary>

| Tool | Description |
|------|-------------|
| `insert_one` | Insert single document |
| `insert_many` | Bulk insert documents |
| `find` | Query documents with filters |
| `find_one` | Find single matching document |
| `update_one` | Update single document |
| `update_many` | Bulk update matching documents |
| `delete_one` | Delete single document |
| `delete_many` | Bulk delete matching documents |
| `count_documents` | Count matching documents |
| `distinct` | Get unique field values |
| `aggregate` | Execute aggregation pipeline |

</details>

<details>
<summary><strong>Indexes & Search (15)</strong></summary>

| Tool | Description |
|------|-------------|
| `index_create` | Create B+ tree index |
| `index_list` | List all indexes |
| `index_drop` | Remove index |
| `index_stats` | Get index statistics |
| `index_stats_refresh` | Recompute index statistics |
| `explain` | Analyze query execution plan |
| `find_with_hint` | Query with forced index hint |
| `index_create_fuzzy` | Create fuzzy search index |
| `fuzzy_search` | Approximate string matching |
| `index_create_fulltext` | Create TF-IDF fulltext index |
| `index_list_fulltext` | List fulltext indexes |
| `fulltext_search` | Search with TF-IDF scoring |
| `fulltext_analyze` | Debug text tokenization |
| `index_create_vector` | Create HNSW vector index |
| `index_list_vector` | List vector indexes |

</details>

<details>
<summary><strong>Vector & Hybrid Search (6)</strong></summary>

| Tool | Description |
|------|-------------|
| `index_drop_vector` | Drop vector index |
| `vector_search` | Vector similarity search |
| `vector_search_filter` | Vector search with document filters |
| `hybrid_search` | RRF fusion of vector + text results with reranking. Auto-embeds if vector omitted. Flat or grouped-by-document modes. |
| `schema_set` | Define JSON Schema validation |
| `schema_get` | Get collection schema |

</details>

<details>
<summary><strong>Script Engine (12)</strong></summary>

| Tool | Description |
|------|-------------|
| `script_save` | Save Rhai script |
| `script_list` | List saved scripts |
| `script_get` | Get script source code |
| `script_delete` | Delete script |
| `script_run` | Execute saved script by name |
| `script_exec` | Execute inline Rhai code |
| `script_history` | View script version history |
| `script_rollback` | Restore previous script version |
| `script_version_get` | Get specific script version |
| `script_tags_add` | Add tags to script |
| `script_tags_remove` | Remove tags from script |
| `script_stats` | Get script execution statistics |

</details>

<details>
<summary><strong>Transactions (7)</strong></summary>

| Tool | Description |
|------|-------------|
| `begin_transaction` | Start ACID transaction |
| `commit_transaction` | Commit transaction atomically |
| `rollback_transaction` | Discard transaction changes |
| `insert_one_tx` | Transactional insert |
| `update_one_tx` | Transactional update |
| `delete_one_tx` | Transactional delete |
| `transaction_status` | Check transaction state |

</details>

<details>
<summary><strong>Admin & Security (13)</strong></summary>

| Tool | Description |
|------|-------------|
| `admin_list_all_collections` | List all collections including system |
| `admin_create_system_collection` | Create protected system collection |
| `admin_set_collection_flags` | Modify collection protection flags |
| `admin_drop_protected` | Force delete protected collection |
| `admin_apikey_create` | Generate API key |
| `admin_apikey_list` | List API keys (masked) |
| `admin_apikey_revoke` | Disable API key |
| `admin_apikey_delete` | Permanently delete API key |
| `acl_list` | List all ACL rules |
| `acl_get` | Get collection ACL |
| `acl_set` | Define ACL rules |
| `acl_delete` | Remove ACL rules |
| `acl_cleanup` | Remove orphaned ACLs |

</details>

<details>
<summary><strong>Listeners (6)</strong></summary>

| Tool | Description |
|------|-------------|
| `listener_list` | List HTTP/HTTPS listeners |
| `listener_get` | Get listener configuration |
| `listener_add` | Add HTTP/HTTPS listener |
| `listener_delete` | Remove listener |
| `listener_enable` | Activate listener |
| `listener_disable` | Deactivate listener |

</details>

<details>
<summary><strong>Embeddings & RAG (17)</strong></summary>

| Tool | Description |
|------|-------------|
| `embed_text` | Generate single text embedding |
| `embed_batch` | Batch text embedding |
| `embed_document` | Chunk and embed document |
| `embed_list_models` | List available embedding models |
| `embed_cache_stats` | Get embedding cache statistics |
| `embed_cache_clear` | Clear embedding cache |
| `auto_embed_enable` | Enable auto-embedding on insert |
| `auto_embed_disable` | Disable auto-embedding |
| `auto_embed_status` | Get auto-embedding configuration |
| `auto_embed_backfill` | Generate embeddings for existing docs |
| `embed_job_status` | Get background job status |
| `embed_job_list` | List background jobs |
| `embed_job_cancel` | Cancel running job |
| `rag_collection_create` | Create RAG-optimized collection |
| `rag_document_import` | Import document with auto-chunking |
| `rag_collection_stats` | Get RAG collection statistics |

</details>

### Downloads

Pre-built binaries for every release:

| Platform | Server | Bridge | TUI | Backup |
|----------|--------|--------|-----|--------|
| Linux | `mcp-ironbase-server-linux` | `ironbase-bridge-linux` | `ironbase-tui-linux` | `ironbase-backup-linux` |
| macOS | `mcp-ironbase-server-macos` | `ironbase-bridge-macos` | `ironbase-tui-macos` | `ironbase-backup-macos` |
| Windows | `mcp-ironbase-server-windows.exe` | `ironbase-bridge-windows.exe` | `ironbase-tui-windows.exe` | `ironbase-backup-windows.exe` |

Windows MSI installer also available: `IronBase-Setup-{version}.msi`

Browse all releases: [github.com/petitan/IronBase/releases](https://github.com/petitan/IronBase/releases)

## MCP Bridge

The `ironbase-bridge` binary provides a STDIO-to-HTTP/HTTPS bridge for MCP clients that only support stdio transport.

**Compatible with:** Claude Desktop, ChatGPT Desktop, VS Code Copilot, Cursor, JetBrains AI, any MCP client.

```bash
# Local server
ironbase-bridge

# Remote HTTPS with API key
ironbase-bridge --server https://myserver:8080/mcp --api-key sk-xxx

# Self-signed certificate (dev/WSL)
ironbase-bridge --server https://localhost:8080/mcp --insecure
```

| Option | Env Variable | Default | Description |
|--------|-------------|---------|-------------|
| `-s, --server` | `MCP_SERVER_URL` | `http://localhost:8080/mcp` | Server URL |
| `-k, --api-key` | `IRONBASE_API_KEY` | - | API key |
| `--insecure` | `MCP_INSECURE` | `false` | Accept self-signed certs |
| `-d, --debug` | `MCP_DEBUG` | `false` | Debug logging |

### Client Configuration

**Claude Desktop** (`claude_desktop_config.json`):

```json
{
  "mcpServers": {
    "ironbase": {
      "command": "/usr/local/bin/ironbase-bridge",
      "env": {
        "MCP_SERVER_URL": "http://localhost:8080/mcp",
        "IRONBASE_API_KEY": "sk-your-key"
      }
    }
  }
}
```

## Backup CLI

Lock-free hot backup leveraging the append-only storage format:

```bash
# Full backup
ironbase-backup backup --db mydata.mlite --output ./backups --full

# Incremental backup
ironbase-backup backup --db mydata.mlite --output ./backups

# Restore
ironbase-backup restore --dir ./backups --output restored.mlite

# Verify integrity
ironbase-backup verify --dir ./backups
```

## TUI

Terminal UI for interactive database management:

```bash
# Connect to MCP server (HTTP)
ironbase-tui --url http://localhost:8080/mcp

# With API key and self-signed cert
ironbase-tui --url https://myserver:8080/mcp -k sk-your-key --insecure

# Connect via stdio (spawns server process)
ironbase-tui --server ./mcp-ironbase-server mydata.mlite
```

**Keyboard shortcuts:** `Shift+K` API key management, `j/k` navigate, `n` new key, `r` revoke, `d` delete.

## API Key Authentication

```bash
# Start server with admin key
IRONBASE_ADMIN_KEY="your-admin-key" mcp-ironbase-server

# Create API key (via MCP tool call)
curl -X POST http://localhost:8080/mcp \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{
    "name":"admin_apikey_create",
    "arguments":{"admin_key":"your-admin-key","name":"production"}
  }}'
# Response: {"key": "sk-abc123...", ...}

# Use API key for authenticated requests
curl -X POST http://localhost:8080/mcp \
  -H "Authorization: Bearer sk-abc123..." \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{
    "name":"collection_list","arguments":{}
  }}'
```

**Management tools:** `admin_apikey_create`, `admin_apikey_list`, `admin_apikey_revoke`, `admin_apikey_delete`

**Config** (`config.toml`):
```toml
[security]
require_api_key = true
api_key_cache_ttl = 60

[tls]
enabled = true
cert_file = "/path/to/cert.pem"
key_file = "/path/to/key.pem"
```

## Environment Variables

| Variable | Used By | Description |
|----------|---------|-------------|
| `IRONBASE_PATH` | Server | Database file path |
| `IRONBASE_API_KEY` | TUI, Bridge | Client API key |
| `IRONBASE_ADMIN_KEY` | Server | Admin key for key management |
| `MCP_SERVER_URL` | Bridge | MCP server URL |
| `MCP_PORT` | Server | Server port |
| `MCP_HOST` | Server | Bind address |
| `MCP_CONFIG` | Server | Config file path |
| `MCP_INSECURE` | TUI, Bridge | Accept self-signed certs |
| `MCP_DEBUG` | Bridge | Enable debug logging |

Boolean env vars accept: `1`/`true`/`yes`/`on` and `0`/`false`/`no`/`off`.

## Architecture

```
ironbase-core/          Core database engine (Rust library)
  src/
  ├── database.rs         Lifecycle, open/close, durability
  ├── collection_core/    CRUD, aggregation, constraints
  ├── query/              Query parser + 25 operator matchers
  ├── aggregation/        Pipeline stages + accumulators
  ├── index/              B+ tree, fuzzy, HNSW, manager
  ├── fulltext.rs         TF-IDF search with stemming
  ├── storage/            Append-only engine (.mlite format)
  ├── transaction.rs      ACID transactions
  ├── wal.rs              Write-ahead log
  └── upsert.rs           Upsert logic (filter → doc conversion)

bindings/python/        PyO3 bindings (pip install ironbase)
IronBase.NET/           C# / .NET 8 bindings (FFI)
mcp-server/             MCP server (HTTP + stdio, 94 tools)
ironbase-bridge/        STDIO ↔ HTTP bridge for MCP clients
ironbase-tui/           Terminal UI
ironbase-backup/        Hot backup CLI (lock-free)
ironbase-cli/           Command-line interface
```

### Storage Format (`.mlite`)

```
Header (256 bytes)     magic: "MONGOLTE", data_end_offset, metadata_offset
Document Region        [u32 len][JSON bytes]... (append-only, immutable once written)
Collection Metadata    document_catalog, indexes, schemas (at end of file)
```

Metadata at end of file prevents race conditions and truncation issues.

### Thread Safety

- `Arc<RwLock<StorageEngine>>` per collection (parking_lot `RwLock`)
- Write lock: insert, update, delete
- Read lock: find, count, list_collections
- Per-index flush for checkpoint (no collection-level blocking)

## Building from Source

**Prerequisites:** Rust 1.75+, Python 3.8+ (for bindings), .NET 8 SDK (for C# bindings)

```bash
# Core library tests
cargo test -p ironbase-core

# Python development build
cd bindings/python && maturin develop

# MCP server
cd mcp-server && cargo build --release

# C# tests
cd IronBase.NET && dotnet test

# Full CI checks
just run-dev-checks
```

## License

[MIT](LICENSE)
