# IronBase MCP Server

MCP (Model Context Protocol) server for IronBase document database.

## Installation

### Pre-built Binaries (Recommended)

Download the latest release for your platform:

```bash
# Linux
curl -L https://github.com/peti12345/MongoLite/releases/latest/download/mcp-ironbase-server-linux-x64.tar.gz | tar xz
chmod +x mcp-ironbase-server
sudo mv mcp-ironbase-server /usr/local/bin/

# macOS (Intel)
curl -L https://github.com/peti12345/MongoLite/releases/latest/download/mcp-ironbase-server-macos-x64.tar.gz | tar xz
chmod +x mcp-ironbase-server
sudo mv mcp-ironbase-server /usr/local/bin/

# macOS (Apple Silicon)
curl -L https://github.com/peti12345/MongoLite/releases/latest/download/mcp-ironbase-server-macos-arm64.tar.gz | tar xz
chmod +x mcp-ironbase-server
sudo mv mcp-ironbase-server /usr/local/bin/

# Windows (PowerShell)
Invoke-WebRequest -Uri https://github.com/peti12345/MongoLite/releases/latest/download/mcp-ironbase-server-windows-x64.zip -OutFile mcp-server.zip
Expand-Archive mcp-server.zip -DestinationPath .
# Add to PATH or move to desired location
```

### Build from Source

```bash
git clone https://github.com/peti12345/MongoLite.git
cd MongoLite/mcp-server
cargo build --release
# Binary: ./target/release/mcp-ironbase-server
```

## Features

- **HTTP and stdio modes** for flexible integration
- **Full CRUD operations** with MongoDB-compatible query syntax
- **Stored Scripts** with versioning, tags, dependencies, and execution tracking
- **Aggregation pipeline** support
- **Index management** including fuzzy text indexes
- **JSON schema validation**

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
      "name": "users"
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
| `index_list` | List indexes for a collection |
| `index_drop` | Drop an index |
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

### Schema Validation

| Tool | Description |
|------|-------------|
| `schema_set` | Set JSON schema for collection validation |
| `schema_get` | Get current schema for a collection |

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
      "code": "let sum = 0; for item in db_find(\"orders\", #{}) { sum += item.amount; } sum",
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
| `db_find(collection, query)` | Find documents matching query |
| `db_find_one(collection, query)` | Find first matching document |
| `db_insert_one(collection, document)` | Insert a document |
| `db_update_one(collection, filter, update)` | Update first matching document |
| `db_update_many(collection, filter, update)` | Update all matching documents |
| `db_delete_one(collection, filter)` | Delete first matching document |
| `db_delete_many(collection, filter)` | Delete all matching documents |
| `db_count(collection, query)` | Count matching documents |
| `db_aggregate(collection, pipeline)` | Run aggregation pipeline |

## Script Tools Reference

| Tool | Description |
|------|-------------|
| `script_save` | Save a script (with versioning) |
| `script_get` | Get a script by name |
| `script_list` | List scripts (with optional tag filter) |
| `script_delete` | Delete a script |
| `script_run` | Run a script |
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
let users = db_find("users", #{ status: "active" });
print(`Found ${users.len()} active users`);
users
```

### With Parameters
```rhai
// Script that accepts parameters
let min_age = params.min_age;
let max_age = params.max_age;
db_find("users", #{
    age: #{ "$gte": min_age, "$lte": max_age }
})
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
let orders = db_find("orders", #{ status: "completed" });
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

## Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `IRONBASE_PATH` | Path to database file | `./ironbase.mlite` |
| `IRONBASE_ADMIN_KEY` | Admin key for protected operations | (none) |
| `IRONBASE_PORT` | HTTP server port | `8080` |

## Testing

```bash
cd mcp-server
cargo test
```
