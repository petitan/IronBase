# IronBase

**High-performance embedded NoSQL document database** with MongoDB-compatible API.

Written in Rust with Python and C# bindings. Single-file, zero-configuration, serverless.

[![Crates.io](https://img.shields.io/crates/v/ironbase-core)](https://crates.io/crates/ironbase-core)
[![PyPI](https://img.shields.io/pypi/v/ironbase)](https://pypi.org/project/ironbase/)
[![NuGet](https://img.shields.io/nuget/v/IronBase)](https://www.nuget.org/packages/IronBase/)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Rust CI](https://github.com/petitan/IronBase/actions/workflows/rust.yml/badge.svg)](https://github.com/petitan/IronBase/actions/workflows/rust.yml)

## Features

| Category | Features |
|----------|----------|
| **Core** | MongoDB-compatible API, Single-file storage, Zero-config, Embedded |
| **Query** | 21 operators: comparison, logical, element, array, regex, fuzzy |
| **Update** | 7 operators: `$set`, `$inc`, `$unset`, `$push`, `$pull`, `$addToSet`, `$pop` |
| **Aggregation** | 6 stages + 6 accumulators with dot notation support |
| **Indexing** | B+ tree indexes, compound indexes, fuzzy indexes, explain(), hint() |
| **Durability** | ACD transactions, WAL, crash recovery, 3 durability modes |
| **Performance** | ~1M+ inserts/sec, O(log n) index lookups |
| **Languages** | Rust, Python (PyO3), C# (.NET 8) |
| **Testing** | 744+ tests, property-based testing, fuzz testing |

## Quick Start

### Python
```bash
pip install ironbase
```

```python
from ironbase import IronBase

# Open database (creates if not exists)
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

# Query with options
results = users.find(
    {"city": "NYC"},
    projection={"name": 1, "age": 1, "_id": 0},
    sort=[("age", -1)],
    limit=10
)

# Aggregation
stats = users.aggregate([
    {"$match": {"age": {"$gte": 18}}},
    {"$group": {"_id": "$city", "count": {"$sum": 1}, "avgAge": {"$avg": "$age"}}},
    {"$sort": {"count": -1}}
])

# Indexing
users.create_index("age")
users.create_compound_index(["city", "age"])
plan = users.explain({"age": 25})  # Shows IndexScan

db.close()
```

### C# (.NET)
```csharp
using IronBase;

```

## Quick Install MCP Server (release v1.0.17)

Note: these commands pin downloads to the v1.0.17 release assets so documentation always matches the release.

### Windows (PowerShell)
```powershell
# Download installer and run (as Administrator)
Invoke-WebRequest -Uri https://github.com/petitan/IronBase/releases/download/v1.0.17/install.ps1 -OutFile install.ps1
Set-ExecutionPolicy -Scope Process -ExecutionPolicy Bypass
.\install.ps1
```

### Linux/macOS
```bash
# Download and run installer (pins to v1.0.17)
curl -sSL https://github.com/petitan/IronBase/releases/download/v1.0.17/install.sh | sudo bash
```

## MCP Server Downloads (v1.0.17)

| Platform | File |
|----------|------|
| Windows | `mcp-ironbase-server-windows.exe` |
| Linux | `mcp-ironbase-server-linux` |
| macOS | `mcp-ironbase-server-macos` |

Full release assets list and checksums available on the release page: https://github.com/petitan/IronBase/releases/tag/v1.0.17

## Backup CLI Usage

```bash
# Create full backup
ironbase-backup backup --db mydata.mlite --output ./backups --full

# Create incremental backup
ironbase-backup backup --db mydata.mlite --output ./backups

# Restore from backups
ironbase-backup restore --dir ./backups --output restored.mlite

# Verify backup integrity
ironbase-backup verify --dir ./backups
```

## TUI Usage

```bash
# Connect to HTTP mode MCP server
ironbase-tui --http http://localhost:8080

# Connect via stdio (spawns MCP server)
ironbase-tui --server ./mcp-ironbase-server --db mydata.mlite
```

### TUI with API Key Authentication

```bash
# Set API key via environment variable
export IRONBASE_API_KEY="sk-your-api-key"
ironbase-tui

# Or configure in ~/.config/ironbase-tui/config.toml
# mcp_api_key = "sk-your-api-key"
```

**Keyboard shortcuts:**
- `Shift+K` - Open API Key management modal
- `n` - Create new API key (requires `IRONBASE_ADMIN_KEY`)
- `r` - Revoke key
- `d` - Delete key
- `c` - Copy new key to clipboard

## API Key Authentication

The MCP server supports API key authentication for secure access.

### Server Configuration (`config.toml`)

```toml
[security]
require_api_key = true      # Enable API key requirement
api_key_cache_ttl = 60      # Cache TTL in seconds
```

### Environment Variables

| Variable | Description |
|----------|-------------|
| `IRONBASE_API_KEY` | API key for client authentication |
| `IRONBASE_ADMIN_KEY` | Admin key for managing API keys |

### Managing API Keys

```bash
# Set admin key
export IRONBASE_ADMIN_KEY="your-secret-admin-key"

# API keys are managed via MCP tools:
# - admin_apikey_create: Create new API key
# - admin_apikey_list: List all keys (masked)
# - admin_apikey_revoke: Disable a key
# - admin_apikey_delete: Permanently delete a key
```

### HTTP Requests with API Key

```bash
curl -X POST http://localhost:8080/mcp \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer sk-your-api-key" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}'
```

## HTTPS/TLS Support

```toml
# config.toml
[tls]
enabled = true
cert_file = "/path/to/cert.pem"
key_file = "/path/to/key.pem"
```

## Manual Installation

1. Download the appropriate binary for your platform from the release page
2. Run `./mcp-ironbase-server install` (as admin/root) if available, or follow the platform-specific installer steps
3. Start the service: `sc start IronBaseService` (Windows) or `systemctl start ironbase` (Linux)


**Full Changelog**: https://github.com/petitan/IronBase/compare/v1.0.16...v1.0.17
