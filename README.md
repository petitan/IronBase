# IronBase

**High-performance embedded NoSQL document database** with MongoDB-compatible API.

Written in Rust with Python and C# bindings. Single-file, zero-configuration, serverless.

[![Crates.io](https://img.shields.io/crates/v/ironbase-core)](https://crates.io/crates/ironbase-core)
[![PyPI](https://img.shields.io/pypi/v/ironbase)](https://pypi.org/project/ironbase/)
[![NuGet](https://img.shields.io/nuget/v/IronBase)](https://www.nuget.org/packages/IronBase/)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Rust CI](https://github.com/petitan/IronBase/actions/workflows/rust.yml/badge.svg)](https://github.com/petitan/IronBase/actions/workflows/rust.yml)

## Table of Contents

- [Features](#features)
- [Quick Start](#quick-start)
- [MCP Server](#quick-install-mcp-server)
- [MCP Bridge](#mcp-bridge-for-claudechatgpt-desktop)
- [Backup CLI](#backup-cli-usage)
- [TUI](#tui-usage)
- [API Key Authentication](#api-key-authentication)
- [Environment Variables](#environment-variables)
- [HTTPS/TLS Support](#httpstls-support)
- [Manual Installation](#manual-installation)

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

// Open database
using var db = new IronBaseClient("myapp.mlite");
var users = db.GetCollection("users");

// Insert
users.InsertOne(new { name = "Alice", age = 30, city = "NYC" });

// Query
var adults = users.Find(new { age = new { _gte = 18 } });
var nyc = users.Find(new { city = "NYC" });

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

## Quick Install MCP Server (release v1.0.55)

Note: these commands pin downloads to the v1.0.55 release assets so documentation always matches the release.

### Windows (PowerShell)
```powershell
# Download installer and run (as Administrator)
Invoke-WebRequest -Uri https://github.com/petitan/IronBase/releases/download/v1.0.55/install.ps1 -OutFile install.ps1
Set-ExecutionPolicy -Scope Process -ExecutionPolicy Bypass
.\install.ps1
```

### Linux/macOS
```bash
# Download and run installer (pins to v1.0.55)
curl -sSL https://github.com/petitan/IronBase/releases/download/v1.0.55/install.sh | sudo bash
```

## MCP Server Downloads (v1.0.55)

| Platform | File |
|----------|------|
| Windows | `mcp-ironbase-server-windows.exe` |
| Linux | `mcp-ironbase-server-linux` |
| macOS | `mcp-ironbase-server-macos` |

Full release assets list and checksums available on the release page: https://github.com/petitan/IronBase/releases/tag/v1.0.55

## MCP Server Usage

```bash
# HTTP mode (default)
mcp-ironbase-server

# Custom port and host
mcp-ironbase-server -p 9090 -H 0.0.0.0

# Custom database path
mcp-ironbase-server -d /path/to/data.mlite

# stdio mode (for Claude Desktop direct integration)
mcp-ironbase-server --stdio

# Service commands (requires admin/root)
mcp-ironbase-server install    # Install as system service
mcp-ironbase-server uninstall  # Uninstall service
mcp-ironbase-server start      # Start service
mcp-ironbase-server stop       # Stop service
mcp-ironbase-server status     # Check service status
```

### MCP Server CLI Options

| Option | Environment Variable | Default | Description |
|--------|---------------------|---------|-------------|
| `-c, --config` | `MCP_CONFIG` | `config.toml` | Config file path |
| `-p, --port` | `MCP_PORT` | 8080 | Server port |
| `-H, --host` | `MCP_HOST` | `0.0.0.0` | Server host |
| `-d, --db` | `IRONBASE_PATH` | platform default | Database file path |
| `--admin-key` | `IRONBASE_ADMIN_KEY` | - | Admin key for protected operations |
| `--stdio` | - | - | Run in stdio mode (for Claude Desktop) |

**Platform Default Database Paths:**
- Windows: `%LOCALAPPDATA%\IronBase\data\ironbase_data.mlite`
- Linux: `/var/lib/ironbase/ironbase_data.mlite`
- macOS: `/usr/local/var/ironbase/ironbase_data.mlite`

## MCP Bridge (for Claude/ChatGPT Desktop)

The `ironbase-bridge` binary provides a STDIO to HTTP/HTTPS bridge for MCP clients.

**Compatible with:**
- Claude Desktop (Anthropic)
- ChatGPT Desktop (OpenAI)
- VS Code Copilot
- JetBrains AI Assistant
- Cursor
- Any MCP-compatible client

**Features:**
- Connection pooling with keep-alive
- Self-signed certificate support (`--insecure`)
- Graceful shutdown (SIGINT/SIGTERM)
- Health check with retry logic
- JSON-RPC 2.0 batch request support
- Cross-platform (Windows, Linux, macOS)

### CLI Options

| Option | Environment Variable | Default | Description |
|--------|---------------------|---------|-------------|
| `-s, --server` | `MCP_SERVER_URL` | `http://localhost:8080/mcp` | Server URL |
| `-k, --api-key` | `IRONBASE_API_KEY` | - | API key for authentication |
| `--insecure` | `MCP_INSECURE` | false | Accept self-signed certificates |
| `-d, --debug` | `MCP_DEBUG` | false | Enable debug logging |
| `--health-retries` | - | 3 | Health check retry count (0=skip) |

### Usage Examples

```bash
# Basic usage (localhost)
ironbase-bridge

# Remote server with HTTPS
ironbase-bridge --server https://192.168.0.136:8080/mcp --api-key sk-xxx

# Self-signed certificate (WSL/dev environment)
ironbase-bridge --server https://localhost:8080/mcp --insecure

# Using environment variables
MCP_SERVER_URL=https://myserver:8080/mcp IRONBASE_API_KEY=sk-xxx ironbase-bridge

# Debug mode
ironbase-bridge --debug
```

### Client Configuration

**Claude Desktop** (`claude_desktop_config.json`):
```json
{
  "mcpServers": {
    "ironbase": {
      "command": "C:\\Program Files\\IronBase\\ironbase-bridge.exe",
      "env": {
        "MCP_SERVER_URL": "http://localhost:8080/mcp",
        "IRONBASE_API_KEY": "sk-your-key"
      }
    }
  }
}
```

**ChatGPT Desktop** - same configuration format (MCP standard).

**Linux/macOS:**
```json
{
  "mcpServers": {
    "ironbase": {
      "command": "/usr/local/bin/ironbase-bridge",
      "env": {
        "MCP_SERVER_URL": "https://192.168.0.136:8080/mcp",
        "IRONBASE_API_KEY": "sk-your-key",
        "MCP_INSECURE": "1"
      }
    }
  }
}
```

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
# Connect to HTTP/HTTPS MCP server
ironbase-tui --url http://localhost:8080/mcp

# With API key
ironbase-tui --url https://192.168.0.136:8080/mcp -k sk-your-key

# Self-signed certificate (WSL/dev)
ironbase-tui --url https://localhost:8080/mcp --insecure

# Connect via stdio (spawns MCP server)
ironbase-tui --server ./mcp-ironbase-server mydata.mlite
```

### TUI CLI Options

| Option | Environment Variable | Description |
|--------|---------------------|-------------|
| `--url` (alias: `--http`) | - | MCP server URL |
| `-k, --api-key` | `IRONBASE_API_KEY` | API key for authentication |
| `--insecure` | - | Accept self-signed certificates |
| `--server` | - | MCP server executable (stdio transport) |

**Note:** Command line arguments override `~/.config/ironbase-tui/config.toml` settings.

### TUI with API Key Authentication

```bash
# Via CLI argument
ironbase-tui --url http://localhost:8080/mcp -k sk-your-api-key

# Via environment variable
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

### Environment Variables

| Variable | Description |
|----------|-------------|
| `IRONBASE_API_KEY` | API key for client authentication |
| `IRONBASE_ADMIN_KEY` | Admin key for managing API keys (create/revoke/delete) |

### Quick Start with API Keys

```bash
# 1. Start the MCP server with admin key
export IRONBASE_ADMIN_KEY="your-secret-admin-key"
./mcp-ironbase-server

# 2. Initialize session and create first API key
curl -X POST http://localhost:8080/mcp \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"cli","version":"1.0"}}}'

curl -X POST http://localhost:8080/mcp \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"admin_apikey_create","arguments":{"admin_key":"your-secret-admin-key","name":"production"}}}'

# Response: {"key": "sk-abc123...", "id": 1, "name": "production", ...}

# 3. Use the API key for authentication
curl -X POST http://localhost:8080/mcp \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer sk-abc123..." \
  -d '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"collection_list","arguments":{}}}'
```

### Server Configuration (`config.toml`)

```toml
[security]
require_api_key = true      # Enable API key requirement
api_key_cache_ttl = 60      # Cache TTL in seconds
```

### Managing API Keys via MCP Tools

| Tool | Description |
|------|-------------|
| `admin_apikey_create` | Create new API key (returns full key once) |
| `admin_apikey_list` | List all keys (shows masked preview only) |
| `admin_apikey_revoke` | Disable a key (can be re-enabled) |
| `admin_apikey_delete` | Permanently delete a key |

**Create API Key:**
```bash
curl -X POST http://localhost:8080/mcp \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc":"2.0","id":1,"method":"tools/call",
    "params":{"name":"admin_apikey_create","arguments":{"admin_key":"your-admin-key","name":"my-app"}}
  }'
# Response: {"key":"sk-cfc3e4633b6feb9056e382c2742d4170","id":1,"name":"my-app",...}
```

**List API Keys:**
```bash
curl -X POST http://localhost:8080/mcp \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc":"2.0","id":1,"method":"tools/call",
    "params":{"name":"admin_apikey_list","arguments":{"admin_key":"your-admin-key"}}
  }'
# Response: {"keys":[{"_id":1,"name":"my-app","key_preview":"sk-cfc...4170","enabled":true}],...}
```

**Revoke API Key:**
```bash
curl -X POST http://localhost:8080/mcp \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc":"2.0","id":1,"method":"tools/call",
    "params":{"name":"admin_apikey_revoke","arguments":{"admin_key":"your-admin-key","id":1}}
  }'
```

**Delete API Key:**
```bash
curl -X POST http://localhost:8080/mcp \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc":"2.0","id":1,"method":"tools/call",
    "params":{"name":"admin_apikey_delete","arguments":{"admin_key":"your-admin-key","id":1}}
  }'
```

### TUI API Key Management

The TUI provides a graphical interface for managing API keys:

```bash
# Start TUI with admin access
export IRONBASE_API_KEY="sk-your-api-key"
export IRONBASE_ADMIN_KEY="your-admin-key"
ironbase-tui --url http://localhost:8080/mcp
```

Press `Shift+K` to open the API Key modal:

```
┌─ API Keys ─────────────────────────────────────────┐
│                                                     │
│  ID   Name          Key              Enabled        │
│  ─────────────────────────────────────────────────  │
│ > 1   production    sk-cfc...4170    ✓             │
│   2   development   sk-296...d3a8    ✓             │
│   3   old-key       sk-abc...xyz     ✗             │
│                                                     │
│  [n]New [r]Revoke [d]Delete [Esc]Close             │
└─────────────────────────────────────────────────────┘
```

| Key | Action |
|-----|--------|
| `n` | Create new API key |
| `r` | Revoke selected key |
| `d` | Delete selected key |
| `j/k` | Navigate list |
| `Esc` | Close modal |

**Note:** When creating a new API key, it is automatically saved to `new_key.txt` for easy copying from another terminal:

| Platform | Path | Permissions |
|----------|------|-------------|
| Linux | `~/.config/ironbase-tui/new_key.txt` | `chmod 600` |
| macOS | `~/Library/Application Support/ironbase-tui/new_key.txt` | `chmod 600` |
| Windows | `%APPDATA%\ironbase-tui\new_key.txt` | Owner-only ACL |

## Environment Variables

All IronBase components support configuration via environment variables. CLI arguments take precedence over environment variables.

### Common Environment Variables

| Variable | Used By | Description |
|----------|---------|-------------|
| `IRONBASE_API_KEY` | TUI, Bridge | API key for authentication |
| `IRONBASE_ADMIN_KEY` | MCP Server | Admin key for protected operations |
| `IRONBASE_PATH` | MCP Server | Database file path |
| `MCP_SERVER_URL` | Bridge | MCP server URL |
| `MCP_INSECURE` | TUI, Bridge | Accept self-signed certificates |
| `MCP_DEBUG` | Bridge | Enable debug logging |
| `MCP_CONFIG` | MCP Server | Config file path |
| `MCP_PORT` | MCP Server | Server port |
| `MCP_HOST` | MCP Server | Server host |

### Boolean Environment Variables

Boolean environment variables accept flexible values (case-insensitive):

| True Values | False Values |
|-------------|--------------|
| `1`, `true`, `TRUE`, `True` | `0`, `false`, `FALSE`, `False` |
| `yes`, `YES`, `Yes` | `no`, `NO`, `No` |
| `on`, `ON`, `On` | `off`, `OFF`, `Off`, `""` (empty) |

**Examples:**
```bash
# All equivalent - enable insecure mode
MCP_INSECURE=1 ironbase-tui --url https://localhost:8080/mcp
MCP_INSECURE=true ironbase-tui --url https://localhost:8080/mcp
MCP_INSECURE=YES ironbase-tui --url https://localhost:8080/mcp

# Windows PowerShell
$env:MCP_INSECURE = "1"
ironbase-tui --url https://localhost:8080/mcp

# Windows CMD
set MCP_INSECURE=true
ironbase-tui --url https://localhost:8080/mcp
```

**Invalid values produce a clear error:**
```
error: invalid value 'maybe' for '--insecure': Invalid boolean value: 'maybe'. Use true/false/1/0/yes/no/on/off
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


**Full Changelog**: https://github.com/petitan/IronBase/compare/v1.0.54...v1.0.55
