# IronBase MCP Server

MCP (Model Context Protocol) server for IronBase document database.

## Installation

### Pre-built Binaries (Recommended)

Download the latest release for your platform:

```bash
# Linux
curl -L https://github.com/petitan/IronBase/releases/latest/download/mcp-ironbase-server-linux-x64.tar.gz | tar xz
chmod +x mcp-ironbase-server
sudo mv mcp-ironbase-server /usr/local/bin/

# macOS (Intel)
curl -L https://github.com/petitan/IronBase/releases/latest/download/mcp-ironbase-server-macos-x64.tar.gz | tar xz
chmod +x mcp-ironbase-server
sudo mv mcp-ironbase-server /usr/local/bin/

# macOS (Apple Silicon)
curl -L https://github.com/petitan/IronBase/releases/latest/download/mcp-ironbase-server-macos-arm64.tar.gz | tar xz
chmod +x mcp-ironbase-server
sudo mv mcp-ironbase-server /usr/local/bin/

# Windows (PowerShell)
Invoke-WebRequest -Uri https://github.com/petitan/IronBase/releases/latest/download/mcp-ironbase-server-windows-x64.zip -OutFile mcp-server.zip
Expand-Archive mcp-server.zip -DestinationPath .
# Add to PATH or move to desired location
```

### Build from Source

```bash
git clone https://github.com/petitan/IronBase.git
cd IronBase/mcp-server
cargo build --release
# Binary: ./target/release/mcp-ironbase-server
```

## Features

- **HTTP and stdio modes** for flexible integration
- **Full CRUD operations** with MongoDB-compatible query syntax
- **Stored Scripts** with versioning, tags, dependencies, and execution tracking
- **Aggregation pipeline** support
- **Index management** including fuzzy and full-text indexes
- **Full-text search** with TF-IDF scoring and multi-language support (Hungarian, English, German)
- **JSON schema validation**
- **Access Control (ACL)** with interface-based permissions (localhost/internal/external)
- **Multi-listener support** for HTTP/HTTPS on multiple interfaces
- **API key authentication** with optional TLS encryption

## Running the Server

### HTTP Mode (default)
```bash
IRONBASE_PATH=/path/to/database.mlite ./mcp-ironbase-server
```

### stdio Mode (for Claude Desktop)
```bash
IRONBASE_PATH=/path/to/database.mlite ./mcp-ironbase-server --stdio
```

## MCP Tools Reference

### Database Management

| Tool | Description |
|------|-------------|
| `db_open` | Open or create a database file (switches current database) |
| `db_stats` | Get database statistics (collection count, names) |
| `db_compact` | Compact database file, remove deleted documents |
| `db_checkpoint` | Force checkpoint - flush pending writes to disk |

**Example - Open/Create Database:**
```json
{
  "method": "tools/call",
  "params": {
    "name": "db_open",
    "arguments": {
      "path": "/path/to/database.mlite",
      "create": true
    }
  }
}
```

### Collection Management

| Tool | Description |
|------|-------------|
| `collection_list` | List all collections in the database |
| `collection_create` | Create a new collection |
| `collection_drop` | Drop (delete) a collection and all its documents |

**Example - Create Collection:**
```json
{
  "method": "tools/call",
  "params": {
    "name": "collection_create",
    "arguments": {
      "collection": "users"
    }
  }
}
```

### Document CRUD

| Tool | Description |
|------|-------------|
| `insert_one` | Insert a single document |
| `insert_many` | Insert multiple documents |
| `find` | Find documents matching query (with pagination, sort, projection) |
| `find_one` | Find first matching document |
| `update_one` | Update first matching document |
| `update_many` | Update all matching documents |
| `delete_one` | Delete first matching document |
| `delete_many` | Delete all matching documents |

**Example - Insert Document:**
```json
{
  "method": "tools/call",
  "params": {
    "name": "insert_one",
    "arguments": {
      "collection": "users",
      "document": {"name": "Alice", "age": 30}
    }
  }
}
```

**Example - Find with Pagination:**
```json
{
  "method": "tools/call",
  "params": {
    "name": "find",
    "arguments": {
      "collection": "users",
      "query": {"age": {"$gte": 18}},
      "sort": {"name": 1},
      "skip": 0,
      "limit": 10,
      "include_total": true
    }
  }
}
```

### Query Features

| Tool | Description |
|------|-------------|
| `count_documents` | Count documents matching query |
| `distinct` | Get distinct values for a field |
| `aggregate` | Run aggregation pipeline |
| `fuzzy_search` | Fuzzy text search with configurable algorithm |
| `fulltext_search` | Full-text search with TF-IDF scoring |
| `fulltext_analyze` | Analyze text tokenization (debug stemming) |

**Example - Aggregation:**
```json
{
  "method": "tools/call",
  "params": {
    "name": "aggregate",
    "arguments": {
      "collection": "orders",
      "pipeline": [
        {"$match": {"status": "completed"}},
        {"$group": {"_id": "$customer_id", "total": {"$sum": "$amount"}}}
      ]
    }
  }
}
```

### Index Management

| Tool | Description |
|------|-------------|
| `index_create` | Create single-field or compound index |
| `index_create_fuzzy` | Create fuzzy text index |
| `index_create_fulltext` | Create full-text search index with language support |
| `index_list` | List indexes for a collection |
| `index_list_fulltext` | List fulltext indexes on a collection |
| `index_drop` | Drop an index |
| `index_stats` | Get index statistics (keys, distinct count, histogram) |
| `index_stats_refresh` | Recompute index statistics for query planner |
| `explain` | Explain query execution plan |
| `find_with_hint` | Find with index hint |

**Example - Create Index:**
```json
{
  "method": "tools/call",
  "params": {
    "name": "index_create",
    "arguments": {
      "collection": "users",
      "field": "email",
      "unique": true
    }
  }
}
```

**Example - Create Full-text Index:**
```json
{
  "method": "tools/call",
  "params": {
    "name": "index_create_fulltext",
    "arguments": {
      "collection": "articles",
      "field": "content",
      "language": "hungarian"
    }
  }
}
```

**Example - Full-text Search:**
```json
{
  "method": "tools/call",
  "params": {
    "name": "fulltext_search",
    "arguments": {
      "collection": "articles",
      "field": "content",
      "query": "keresett kifejezés",
      "limit": 10,
      "projection": {"title": 1, "_id": 1}
    }
  }
}
```

### Vector & Hybrid Search

| Tool | Description |
|------|-------------|
| `index_create_vector` | Create HNSW vector index (Cosine/Euclidean/DotProduct) |
| `index_list_vector` | List vector indexes on a collection |
| `index_drop_vector` | Drop a vector index |
| `vector_search` | Vector similarity search (requires HNSW index) |
| `vector_search_filter` | Vector similarity search with document filter |
| `hybrid_search` | RRF fusion of vector + fulltext results. Auto-embeds query if `vector` is omitted. Supports flat and grouped response modes. |

`hybrid_search` combines vector similarity and fulltext search using **Reciprocal Rank Fusion (RRF)**. Optionally uses **MMR (Maximal Marginal Relevance)** for diversity reranking.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `collection` | string | *(required)* | Collection to search |
| `query` | string | *(required)* | Text query for fulltext search (and auto-embedding if vector omitted) |
| `vector` | array | *(optional)* | Query embedding vector. If omitted, query is auto-embedded using collection's provider. |
| `provider` | string | *(auto)* | Embedding provider for auto-embed mode (uses collection RAG config if not specified). |
| `limit` | integer | `10` | Maximum results (chunks in flat mode, documents in grouped mode) |
| `search_mode` | string | `"balanced"` | Preset: `"balanced"` (0.5/0.5), `"semantic"` (0.8/0.2), `"keyword"` (0.2/0.8) |
| `vector_weight` | number | *(from mode)* | Explicit vector weight override (0.0–1.0) |
| `fulltext_weight` | number | *(from mode)* | Explicit fulltext weight override (0.0–1.0) |
| `mode` | string | `"and"` | Fulltext mode: `"and"` = ALL words required, `"or"` = any word (deprecated) |
| `rerank` | boolean | `true` | Enable reranking: phrase match (1.5x), keyword density (1.0–1.3x), short content penalty (0.8x) |
| `deduplicate` | boolean | `false` | Enable MMR diversity reranking |
| `mmr_lambda` | number | `0.7` | MMR balance: relevance (`1.0`) vs diversity (`0.0`) |
| `merge_chunks` | boolean | `true` | Merge adjacent chunks from same document (overlap dedup) |
| `group_by_document` | boolean | `false` | Group results by source document (see below) |
| `match_scope` | string | `"chunk"` | AND match scope: `"chunk"` or `"document"` (all words across doc's chunks) |
| `text_fields` | array | *(optional)* | Multiple fulltext fields to search (overrides `text_field`) |
| `title_field` | string | *(optional)* | Field for title match boost (up to 1.5x) |
| `filter` | object | *(optional)* | MongoDB-style pre-filter |
| `rrf_k` | number | `20` | RRF K constant (lower = wider score spread) |

**`group_by_document` mode:**

When `true`, results are grouped by source document. Each document includes ALL relevant chunks (not just the best one). Document selection uses AND logic (all query words must appear somewhere in the document), while chunk retrieval uses OR logic (any query word makes a chunk relevant). The `limit` parameter applies to document count, not chunk count.

Response format:
```json
{
  "results": [
    {"doc_id": "abc", "best_score": 0.039, "chunk_count": 9, "chunks": [...]},
    {"doc_id": "def", "best_score": 0.028, "chunk_count": 4, "chunks": [...]}
  ],
  "count": 2,
  "total_chunks": 13,
  "group_by_document": true,
  "qualified_doc_ids": 359
}
```

**Example - Flat mode (default):**
```json
{
  "method": "tools/call",
  "params": {
    "name": "hybrid_search",
    "arguments": {
      "collection": "articles",
      "query": "keresett kifejezés",
      "limit": 10
    }
  }
}
```

**Example - Grouped by document:**
```json
{
  "method": "tools/call",
  "params": {
    "name": "hybrid_search",
    "arguments": {
      "collection": "articles",
      "query": "keresett kifejezés",
      "group_by_document": true,
      "limit": 5
    }
  }
}
```

**Example - Explicit vector with semantic mode:**
```json
{
  "method": "tools/call",
  "params": {
    "name": "hybrid_search",
    "arguments": {
      "collection": "articles",
      "query": "keresett kifejezés",
      "vector": [0.1, 0.2, 0.3],
      "search_mode": "semantic",
      "limit": 10
    }
  }
}
```

### Schema Validation

| Tool | Description |
|------|-------------|
| `schema_set` | Set JSON schema for collection validation |
| `schema_get` | Get current schema for a collection |

### Access Control (ACL)

| Tool | Description |
|------|-------------|
| `acl_list` | List all ACL rules |
| `acl_get` | Get ACL for specific collection |
| `acl_set` | Set ACL for collection (localhost only) |
| `acl_delete` | Delete ACL (revert to defaults) |
| `acl_cleanup` | Cleanup orphaned ACLs |

See [ACL Documentation](docs/ACL.md) for details.

### Listener Management

| Tool | Description |
|------|-------------|
| `listener_list` | List all configured listeners |
| `listener_get` | Get specific listener configuration |
| `listener_add` | Add HTTP/HTTPS listener |
| `listener_delete` | Delete a listener |
| `listener_enable` | Enable a listener |
| `listener_disable` | Disable a listener |

### API Key Management

| Tool | Description |
|------|-------------|
| `admin_apikey_create` | Create new API key (requires admin_key) |
| `admin_apikey_list` | List all API keys (masked) |
| `admin_apikey_revoke` | Disable an API key |
| `admin_apikey_delete` | Permanently delete an API key |

**Example - Set Schema:**
```json
{
  "method": "tools/call",
  "params": {
    "name": "schema_set",
    "arguments": {
      "collection": "users",
      "schema": {
        "type": "object",
        "required": ["name", "email"],
        "properties": {
          "name": {"type": "string"},
          "email": {"type": "string", "format": "email"},
          "age": {"type": "integer", "minimum": 0}
        }
      }
    }
  }
}
```

### Admin Operations

| Tool | Description |
|------|-------------|
| `admin_list_all_collections` | List all collections including system/hidden |
| `admin_create_system_collection` | Create protected system collection |
| `admin_set_collection_flags` | Modify collection protection/visibility flags |
| `admin_drop_protected` | Force delete a protected collection |

### Transactions

| Tool | Description |
|------|-------------|
| `begin_transaction` | Start ACID transaction (exclusive write lock) |
| `commit_transaction` | Commit all changes atomically |
| `rollback_transaction` | Discard all changes |
| `insert_one_tx` | Insert document within active transaction |
| `update_one_tx` | Update document within active transaction |
| `delete_one_tx` | Delete document within active transaction |
| `transaction_status` | Check if an active transaction exists |

### Embedding Generation

| Tool | Description |
|------|-------------|
| `embed_text` | Generate single text embedding |
| `embed_batch` | Batch text embedding (max 100 texts) |
| `embed_document` | Chunk document, embed chunks, store with vector index |
| `embed_list_models` | List available embedding models and providers |
| `embed_cache_stats` | Get embedding cache hit rate and memory usage |
| `embed_cache_clear` | Clear all embedding cache entries |

**Supported providers** (configured via `[embedding]` section in `config.toml`):

| Provider | Type | Example model |
|----------|------|---------------|
| `ollama` | Local HTTP (Ollama daemon) | `bge-m3`, `nomic-embed-text` |
| `vllm` | Local HTTP (vLLM, OpenAI-compatible) | `bge-m3` |
| `openai` | Cloud API | `text-embedding-3-small` |

### Auto-Embedding

| Tool | Description |
|------|-------------|
| `auto_embed_enable` | Auto-generate embeddings on insert/update (auto-backfills existing docs) |
| `auto_embed_disable` | Disable auto-embedding for collection |
| `auto_embed_status` | Get auto-embedding configuration |

### Background Jobs

| Tool | Description |
|------|-------------|
| `embed_job_status` | Get background job status by ID |
| `embed_job_list` | List all background jobs (active + recent) |
| `embed_job_cancel` | Cancel a running background job |

### RAG (Retrieval-Augmented Generation)

| Tool | Description |
|------|-------------|
| `rag_collection_create` | Create RAG-optimized collection (auto: vector + fulltext indexes) |
| `rag_document_import` | Import document with auto-chunking and embedding |
| `rag_collection_stats` | Get RAG collection statistics (chunks, sources, indexes) |

---

## Stored Scripts System

The MCP server includes a powerful stored scripts feature using Rhai scripting language.

### Basic Operations

**Save a script:**
```json
{
  "method": "tools/call",
  "params": {
    "name": "script_save",
    "arguments": {
      "name": "calculate_total",
      "code": "let sum = 0; for item in db_find(\"orders\", #{}).documents { sum += item.amount; } sum",
      "description": "Calculate total order amount",
      "tags": ["utility", "finance"],
      "dependencies": []
    }
  }
}
```

**Run a script:**
```json
{
  "method": "tools/call",
  "params": {
    "name": "script_run",
    "arguments": {
      "name": "calculate_total",
      "params": {}
    }
  }
}
```

### Versioning

Every save creates a new version. Access version history:

```json
{
  "method": "tools/call",
  "params": {
    "name": "script_history",
    "arguments": {
      "name": "calculate_total",
      "limit": 10
    }
  }
}
```

Rollback to a previous version:
```json
{
  "method": "tools/call",
  "params": {
    "name": "script_rollback",
    "arguments": {
      "name": "calculate_total",
      "version": 1
    }
  }
}
```

### Tags

Filter scripts by tags:
```json
{
  "method": "tools/call",
  "params": {
    "name": "script_list",
    "arguments": {
      "tags": ["utility", "finance"],
      "match_all": false
    }
  }
}
```

Add/remove tags dynamically:
```json
{
  "method": "tools/call",
  "params": {
    "name": "script_tags_add",
    "arguments": {
      "name": "calculate_total",
      "tags": ["new_tag"]
    }
  }
}
```

### Dependencies

Scripts can depend on other scripts. Dependencies are automatically resolved and executed in topological order:

```json
{
  "method": "tools/call",
  "params": {
    "name": "script_save",
    "arguments": {
      "name": "helper_functions",
      "code": "fn add(a, b) { a + b } fn multiply(a, b) { a * b }"
    }
  }
}
```

```json
{
  "method": "tools/call",
  "params": {
    "name": "script_save",
    "arguments": {
      "name": "main_calculation",
      "code": "add(10, multiply(5, 3))",
      "dependencies": ["helper_functions"]
    }
  }
}
```

### Execution Statistics

Get script execution statistics:
```json
{
  "method": "tools/call",
  "params": {
    "name": "script_stats",
    "arguments": {
      "name": "calculate_total"
    }
  }
}
```

Returns:
```json
{
  "name": "calculate_total",
  "execution_count": 42,
  "last_run_at": "2024-01-15T10:30:00Z",
  "last_run_success": true,
  "total_execution_time_ms": 1500,
  "avg_execution_time_ms": 35.7
}
```

## Available Database Functions in Scripts

Scripts have access to these database functions:

| Function | Description |
|----------|-------------|
| `db_find(collection, query)` | Find documents matching query (returns `#{documents: [...], count: n}`) |
| `db_find_one(collection, query)` | Find first matching document |
| `db_insert_one(collection, document)` | Insert a document |
| `db_update_one(collection, filter, update)` | Update first matching document |
| `db_update_many(collection, filter, update)` | Update all matching documents |
| `db_delete_one(collection, filter)` | Delete first matching document |
| `db_delete_many(collection, filter)` | Delete all matching documents |
| `db_count(collection, query)` | Count matching documents |
| `db_aggregate(collection, pipeline)` | Run aggregation pipeline |
| `db_hybrid_search(collection, query)` | RRF hybrid search (vector + fulltext fusion via fusion.rs) |
| `db_hybrid_search(collection, query, options)` | Hybrid search with options (limit, rrf_k, rerank, search_mode, filter, etc.) |
| `db_rag_import(collection, text, metadata)` | Import RAG document with auto-chunking and embedding |
| `db_rag_create(collection)` | Create RAG-optimized collection (vector + fulltext indexes) |
| `db_rag_create(collection, options)` | Create RAG collection with custom options (provider, chunk_size, etc.) |
| `db_rag_stats(collection)` | Get RAG collection statistics |

## Script Tools Reference

| Tool | Description |
|------|-------------|
| `script_save` | Save a script (with versioning) |
| `script_get` | Get a script by name |
| `script_list` | List scripts (with optional tag filter) |
| `script_delete` | Delete a script |
| `script_run` | Run a saved script by name |
| `script_exec` | Execute inline Rhai code (no save) |
| `script_history` | Get version history |
| `script_rollback` | Rollback to previous version |
| `script_version_get` | Get specific version |
| `script_tags_add` | Add tags |
| `script_tags_remove` | Remove tags |
| `script_stats` | Get execution statistics |

## Example Scripts

### Basic Query
```rhai
// Find all active users
let result = db_find("users", #{ status: "active" });
let users = result.documents;
print(`Found ${users.len()} active users`);
users
```

### With Parameters
```rhai
// Script that accepts parameters
let min_age = params.min_age;
let max_age = params.max_age;
let result = db_find("users", #{
    age: #{ "$gte": min_age, "$lte": max_age }
});
result.documents
```

### Data Aggregation
```rhai
// Calculate order totals by status
let pipeline = [
    #{ "$group": #{
        "_id": "$status",
        "total": #{ "$sum": "$amount" },
        "count": #{ "$sum": 1 }
    }},
    #{ "$sort": #{ "total": -1 }}
];
db_aggregate("orders", pipeline)
```

### Helper Functions
```rhai
// helper_utils.rhai - reusable utility functions
fn format_currency(amount) {
    `$${amount.to_string()}`
}

fn calculate_tax(amount, rate) {
    amount * rate
}

fn safe_divide(a, b) {
    if b == 0 { 0 } else { a / b }
}
```

### Report with Dependencies
```rhai
// Depends on: helper_utils
let orders = db_find("orders", #{ status: "completed" }).documents;
let total = 0;
for order in orders {
    total += order.amount;
}
let tax = calculate_tax(total, 0.08);
#{
    total_orders: orders.len(),
    gross_total: format_currency(total),
    tax: format_currency(tax),
    net_total: format_currency(total + tax)
}
```

### Hybrid Search (RAG)
```rhai
// Search knowledge base with hybrid vector + fulltext fusion
let results = db_hybrid_search("knowledge_base", "search query", #{
    limit: 5,
    search_mode: "semantic",
    rerank: true,
    merge_chunks: true,
    title_field: "title",
    filter: #{ year: 2026 }
});

for doc in results {
    print(`${doc.title}: ${doc._final_score}`);
}
```

## Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `IRONBASE_PATH` | Path to database file | `./ironbase.mlite` |
| `IRONBASE_ADMIN_KEY` | Admin key for protected operations | (none) |
| `MCP_PORT` | HTTP server port | `8080` |

## Concurrent Access

The MCP server safely handles multiple concurrent clients:

### Thread Safety Model

```
┌─────────────────────────────────────────────┐
│           MCP Server (HTTP)                 │
├─────────────────────────────────────────────┤
│  Request 1 ──┐                              │
│  Request 2 ──┼──→ Arc<RwLock<DatabaseCore>> │
│  Request 3 ──┘                              │
└─────────────────────────────────────────────┘

Read operations (find, count): Parallel (RwLock read)
Write operations (insert, update, delete): Serialized (RwLock write)
```

### Testing Concurrent Access

The server includes concurrent access test scripts:

```bash
# Linux/macOS
./tests/concurrent_insert_read.sh

# Windows (PowerShell)
./tests/concurrent_insert_read.ps1
```

**Test behavior:**
- Inserter: 100 sequential inserts
- Reader: 200 parallel reads
- Verifies all documents are visible and consistent

**Expected output:**
```
=== Concurrent Insert/Read Test ===
INSERTER: 100 success, 0 fail
READER: 200 success, 0 fail, max docs seen: 100
Final document count: 100 (expected: 100)
=== TEST PASSED ===
```

### Performance Under Load

| Scenario | Throughput | Notes |
|----------|------------|-------|
| Single client reads | ~500-1000 req/s | HTTP overhead dominates |
| Single client writes | ~50-100 req/s | fsync per operation (Safe mode) |
| Concurrent reads | ~2000-3000 req/s | Parallel RwLock reads |
| Mixed read/write | ~300-500 req/s | Write serialization |

### Durability Guarantees

By default, the MCP server uses **Safe mode**:
- Every write operation is fsync'd to disk
- Data survives server crash/restart
- ~50-100 writes/second throughput

For higher throughput (at cost of durability), configure Batch mode in the database initialization.

## Testing

```bash
cd mcp-server
cargo test

# Concurrent access test
./tests/concurrent_insert_read.sh
```
