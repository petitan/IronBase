//! Admin and security prompts
//!
//! Contains: rhai-scripting, acl-guide, database-admin, security-guide, listener-config

use serde_json::{json, Value};

pub fn rhai_scripting() -> Value {
    json!({
        "messages": [
            {
                "role": "user",
                "content": {
                    "type": "text",
                    "text": r#"# IronBase Rhai Scripting Guide

Rhai is a lightweight scripting language for server-side operations. Scripts can be saved and reused, with version history and execution statistics.

## Running Scripts

### Execute Inline Code
```json
// script_exec tool
{"code": "let x = 1 + 2; x", "params": {"name": "test"}}
```

### Run Saved Script
```json
// script_run tool
{"name": "my_script", "params": {"count": 10}}

// With custom operation limit (DoS protection)
{"name": "my_script", "params": {}, "max_operations": 500000}
```

## Database Functions

| Function | Description | Example |
|----------|-------------|---------|
| Note | Use global functions (no `db` object) | Call `db_find(...)`, not `db.find(...)` |
| `db_find(coll, query)` | Find documents | `db_find("users", #{age: #{`$gt`: 18}})` → `#{documents: [...], count: n}` |
| `db_find_one(coll, query)` | Find single document (or null) | `db_find_one("users", #{name: "Alice"})` |
| `db_find_one_result(coll, query)` | Find with explicit result type | Returns `#{found: bool, doc: ..., error: ...}` |
| `db_insert_one(coll, doc)` | Insert document | `db_insert_one("users", #{name: "Bob"})` |
| `db_insert_many(coll, docs)` | Insert multiple | `db_insert_many("users", [#{...}, #{...}])` |
| `db_update_one(coll, filter, update)` | Update one | `db_update_one("users", #{name: "x"}, #{`$set`: #{age: 30}})` |
| `db_update_many(coll, filter, update)` | Update many | Same as update_one |
| `db_delete_one(coll, filter)` | Delete one | `db_delete_one("users", #{name: "x"})` |
| `db_delete_many(coll, filter)` | Delete many | Same as delete_one |
| `db_count(coll, query)` | Count documents | `db_count("users", #{active: true})` |
| `db_aggregate(coll, pipeline)` | Aggregation | `db_aggregate("sales", [#{`$group`: ...}])` |

## Index Management Functions

| Function | Description | Example |
|----------|-------------|---------|
| `db_create_index(coll, field, unique)` | Create single-field index | `db_create_index("users", "email", true)` |
| `db_create_compound_index(coll, fields, unique)` | Create compound index | `db_create_compound_index("orders", ["user_id", "date"], false)` |
| `db_list_indexes(coll)` | List all indexes | `db_list_indexes("users")` |
| `db_drop_index(coll, name)` | Drop index by name | `db_drop_index("users", "email_1")` |
| `db_explain(coll, query)` | Explain query plan | `db_explain("users", #{age: #{`$gt`: 18}})` |
| `db_distinct(coll, field, query)` | Get distinct values | `db_distinct("users", "city", #{active: true})` |

## Search Functions

| Function | Description | Example |
|----------|-------------|---------|
| `db_create_fuzzy_index(coll, field, algo, threshold)` | Create fuzzy index | `db_create_fuzzy_index("users", "name", "jaro_winkler", 0.8)` |
| `db_fuzzy_search(coll, field, query, threshold)` | Fuzzy text search | `db_fuzzy_search("users", "name", "john", 0.7)` |
| `db_create_fulltext_index(coll, field, lang)` | Create fulltext index | `db_create_fulltext_index("articles", "content", "english")` |
| `db_fulltext_search(coll, field, query, options)` | Fulltext search with options | `db_fulltext_search("articles", "content", "database", #{limit: 10, min_score: 0.2})` |

## Helper Functions

| Function | Description | Example |
|----------|-------------|---------|
| `is_error(v)` | Check if value is error string | `if is_error(result) { ... }` |
| `is_null(v)` | Check if value is null/unit | `if is_null(doc) { ... }` |
| `get_error(v)` | Extract error message | `let msg = get_error(result);` |
| `unwrap_or(v, default)` | Return value or default if error/null | `let x = unwrap_or(val, 0);` |

## db_find_one vs db_find_one_result

### db_find_one (Simple)
Returns the document or null. Errors return error string.
```rhai
let doc = db_find_one("users", #{name: "Alice"});
if is_null(doc) {
    print("Not found");
} else if is_error(doc) {
    print("Error: " + get_error(doc));
} else {
    print("Found: " + doc.name);
}
```

### db_find_one_result (Explicit)
Returns a result object with explicit fields - preferred for clarity.
```rhai
let result = db_find_one_result("users", #{name: "Alice"});
if result.error != () {
    print("Error: " + result.error);
} else if !result.found {
    print("Not found");
} else {
    print("Found: " + result.doc.name);
}
```

## Utility Functions

| Function | Description |
|----------|-------------|
| `base64_encode(str)` | Encode string to base64 |
| `base64_decode(b64)` | Decode base64 to string |
| `print(msg)` | Log message (captured in result.logs) |
| `uuid()` | Generate UUID v4 string |
| `timestamp()` | Current Unix timestamp (seconds) |
| `timestamp_ms()` | Current Unix timestamp (milliseconds) |
| `now_iso()` | Current datetime as ISO 8601 string |

## Rhai Syntax Basics

### Variables & Types
```rhai
let x = 42;           // Integer
let s = "hello";      // String
let arr = [1, 2, 3];  // Array
let map = #{a: 1, b: 2};  // Object map (use #{ } for maps!)
```

### Query Operators in Maps
Use backticks for operators starting with `$`:
```rhai
let query = #{age: #{`$gt`: 18, `$lt`: 65}};
let result = db_find("users", query);
let docs = result.documents;
```

### Control Flow
```rhai
// If/else
if x > 10 {
    print("big");
} else {
    print("small");
}

// For loop
for item in arr {
    print(item);
}

// While loop
while x > 0 {
    x -= 1;
}
```

### Functions
```rhai
fn greet(name) {
    "Hello, " + name + "!"
}

let msg = greet("World");
```

## Example Scripts

### Batch Insert with Logging
```rhai
let names = ["Alice", "Bob", "Carol"];
let inserted = 0;

for name in names {
    let doc = #{name: name, created: "2024-01-01"};
    let result = db_insert_one("users", doc);
    if !is_error(result) {
        inserted += 1;
        print("Inserted: " + name);
    }
}

#{inserted: inserted, total: names.len()}
```

### Safe Document Lookup
```rhai
let result = db_find_one_result("users", #{_id: params.user_id});

if result.error != () {
    #{success: false, error: result.error}
} else if !result.found {
    #{success: false, error: "User not found"}
} else {
    #{success: true, user: result.doc}
}
```

### Aggregation Report
```rhai
let pipeline = [
    #{`$match`: #{status: "completed"}},
    #{`$group`: #{
        _id: "$category",
        total: #{`$sum`: "$amount"},
        count: #{`$sum`: 1}
    }},
    #{`$sort`: #{total: -1}}
];

let results = db_aggregate("orders", pipeline);
if is_error(results) {
    #{error: get_error(results)}
} else {
    #{report: results, generated: "now"}
}
```

## Script Management

| Tool | Description |
|------|-------------|
| `script_save` | Save/update script with name, code, description |
| `script_list` | List all saved scripts |
| `script_get` | Get script code and metadata |
| `script_delete` | Delete a script |
| `script_history` | View version history |
| `script_rollback` | Restore previous version |
| `script_stats` | View execution statistics |

## Security & Limits

| Limit | Default | Description |
|-------|---------|-------------|
| Max operations | 1,000,000 | Prevents infinite loops (configurable via max_operations) |
| Max log entries | 10,000 | Prevents memory exhaustion |
| Max execution time | ~60s | Implicit via operation limit |

No file I/O or network access - scripts can only interact with the database."#
                }
            }
        ]
    })
}

pub fn acl_guide() -> Value {
    json!({
        "messages": [
            {
                "role": "user",
                "content": {
                    "type": "text",
                    "text": r#"# Access Control List (ACL) Guide

IronBase provides collection-level permissions based on client origin.

## Interface Types

| Type | Description | Example IPs |
|------|-------------|-------------|
| `localhost` | Loopback address | 127.0.0.1, ::1 |
| `internal` | Private network (RFC 1918) | 10.x.x.x, 172.16-31.x.x, 192.168.x.x |
| `external` | Public internet | Everything else |

## Permission Levels

| Permission | Includes | Operations |
|------------|----------|------------|
| `read` | - | find, count, aggregate, explain |
| `write` | read | insert, update, delete |
| `admin` | read, write | create/drop index, schema, compact |
| `all` | everything | All operations |

## Default Permissions (No ACL Set)

| Interface | Permissions |
|-----------|-------------|
| localhost | all |
| internal | read, write |
| external | read |

## Setting ACL Rules

```json
// acl_set tool
{
  "collection": "users",
  "rules": [
    {"principal": "interface:localhost", "permissions": "all"},
    {"principal": "interface:internal", "permissions": "read,write"},
    {"principal": "interface:external", "permissions": "read"}
  ]
}
```

## Principal Types

| Principal | Description | Example |
|-----------|-------------|---------|
| `interface:localhost` | Loopback connections | Local development |
| `interface:internal` | Private network | Backend services |
| `interface:external` | Public internet | Public API |
| `apikey:sk-xxx` | Specific API key | Privileged service |
| `ip:192.168.1.100` | Exact IP address | Specific server |
| `iprange:10.0.0.0/8` | CIDR range | Subnet |
| `anyone` | All clients | Public data |

## ACL Tools

### List All ACLs
```json
// acl_list tool
{}
```

### Get ACL for Collection
```json
// acl_get tool
{"collection": "users"}
```

### Set ACL
```json
// acl_set tool (localhost only)
{
  "collection": "users",
  "rules": [
    {"principal": "interface:localhost", "permissions": "all"},
    {"principal": "apikey:sk-admin-key", "permissions": "admin"},
    {"principal": "interface:internal", "permissions": "read,write"},
    {"principal": "interface:external", "permissions": "read"}
  ]
}
```

### Delete ACL (Revert to Defaults)
```json
// acl_delete tool (localhost only)
{"collection": "users"}
```

### Cleanup Orphaned ACLs
```json
// acl_cleanup tool (localhost only)
{}
```

## Example Scenarios

### 1. Public Read-Only API
```json
{
  "collection": "products",
  "rules": [
    {"principal": "interface:localhost", "permissions": "all"},
    {"principal": "anyone", "permissions": "read"}
  ]
}
```

### 2. Internal Service Write Access
```json
{
  "collection": "orders",
  "rules": [
    {"principal": "interface:localhost", "permissions": "all"},
    {"principal": "interface:internal", "permissions": "read,write"},
    {"principal": "interface:external", "permissions": ""}
  ]
}
```

### 3. API Key Based Access
```json
{
  "collection": "sensitive_data",
  "rules": [
    {"principal": "interface:localhost", "permissions": "all"},
    {"principal": "apikey:sk-trusted-service", "permissions": "read,write"},
    {"principal": "anyone", "permissions": ""}
  ]
}
```

### 4. IP Whitelist
```json
{
  "collection": "admin_logs",
  "rules": [
    {"principal": "interface:localhost", "permissions": "all"},
    {"principal": "ip:10.0.1.50", "permissions": "read"},
    {"principal": "iprange:10.0.2.0/24", "permissions": "read"}
  ]
}
```

## System Collections

System collections (`_system.*`) have special protection:

| Collection | Read Access | Write Access |
|------------|-------------|--------------|
| `_system.scripts` | localhost, internal, external | localhost only |
| `_system.api_keys` | localhost only | localhost only |
| `_system.acl` | localhost only | localhost only |
| `_system.listeners` | localhost only | localhost only |

## Rule Evaluation Order

1. Check if collection has explicit ACL rules
2. Find matching principal (most specific first):
   - `ip:exact` > `iprange:cidr` > `apikey:xxx` > `interface:xxx` > `anyone`
3. If no match found, use default permissions for interface type
4. Deny if no permission grants the operation

## Best Practices

1. **Always set localhost to `all`**: You need admin access from local machine
2. **Use API keys for services**: More granular than IP-based rules
3. **Deny by default for sensitive data**: Empty permissions = deny all
4. **Cleanup orphaned ACLs**: Run `acl_cleanup` after dropping collections
5. **Test from different interfaces**: Verify rules work as expected"#
                }
            }
        ]
    })
}

pub fn database_admin() -> Value {
    json!({
        "messages": [
            {
                "role": "user",
                "content": {
                    "type": "text",
                    "text": r#"# Database Administration Guide

## Database Statistics

```json
// db_stats tool
{}
```

Returns:
```json
{
  "version": "1.0.68",
  "collections": 5,
  "total_documents": 12345,
  "database_size_bytes": 5242880,
  "uptime_seconds": 3600,
  "collections_detail": {
    "users": {"documents": 1000, "indexes": 3},
    "orders": {"documents": 5000, "indexes": 2}
  }
}
```

## Database Compaction

Removes tombstones and reclaims disk space:

```json
// db_compact tool
{"collection": "users"}
```

When to compact:
- After bulk deletes
- When database file is much larger than data
- During maintenance windows

## Database Checkpoint

Force WAL flush to disk:

```json
// db_checkpoint tool
{}
```

Use cases:
- Before backup
- After important batch operations
- Before server shutdown

## Collection Management

### List Collections
```json
// collection_list tool
{}
```

### Create Collection
```json
// insert_one tool (auto-creates collection)
{"collection": "new_collection", "document": {"_id": "init"}}
```

### Drop Collection
```json
// collection_drop tool
{"name": "old_collection"}
```

### Get Collection Stats
```json
// count tool
{"collection": "users", "query": {}}
```

## Index Management

### List Indexes
```json
// index_list tool
{"collection": "users"}
```

### Create Index
```json
// index_create tool
{"collection": "users", "field": "email", "unique": true}
```

### Create Compound Index
```json
// index_create tool (compound via fields)
{"collection": "orders", "fields": ["user_id", "created_at"], "unique": false}
```

### Drop Index
```json
// index_drop tool
{"collection": "users", "index_name": "email_1"}
```

## Query Analysis

### Explain Query Plan
```json
// explain tool
{
  "collection": "users",
  "query": {"email": "test@example.com"}
}
```

Returns:
```json
{
  "plan": "IndexScan",
  "index_used": "email_1",
  "estimated_docs": 1,
  "query_time_ms": 0.5
}
```

## Durability Modes

| Mode | Description | Use Case |
|------|-------------|----------|
| Safe | fsync after each write | Production (default) |
| Batch | fsync after N writes | Bulk imports |
| Unsafe | No auto-fsync | Testing only |

## Backup Operations

### Hot Backup (Lock-Free)

IronBase supports hot backup without stopping the server using the `ironbase-backup` CLI tool.

#### Full Backup
```bash
ironbase-backup backup --db /path/to/data.mlite --output ./backups --full
```

#### Split Backup (for large databases >10GB)
```bash
# Split into 5GB parts for easier transfer/storage
ironbase-backup backup --db /path/to/data.mlite --output ./backups --split 5G
```

#### Restore
```bash
# From single backup
ironbase-backup restore --backup ./backups/backup_xxx.ibak --output /path/to/restored.mlite

# From split backup (auto-reassembles parts)
ironbase-backup restore --backup ./backups/backup_xxx.ibak.001 --output /path/to/restored.mlite
```

#### Example: 39GB Database Backup
```
Time: ~6 minutes
Original: 39 GB
Compressed: 25 GB (1.5x compression)
Parts: 6 × 5GB files
Hash: CRC32 verification included
```

### Backup Verification
- Each backup includes CRC32 hash for integrity verification
- Split parts are automatically reassembled during restore
- Backup files use `.ibak` extension (IronBase Backup)

### Why Lock-Free Works

IronBase uses append-only storage:
- Documents are never modified in-place
- Updates create new versions + tombstones
- All data up to `data_end_offset` is immutable
- Backup reads immutable data safely without locking

## Performance Tuning

### Monitor Slow Queries
1. Use `explain` tool to check query plans
2. Create indexes for frequently queried fields
3. Use projections to reduce data transfer

### Optimize Storage
1. Run compaction after bulk deletes
2. Use appropriate data types (avoid storing large blobs)
3. Consider sharding for very large datasets

### Memory Usage
- Query cache: ~1000 entries (LRU eviction)
- Index cache: In-memory B+ trees
- Document cache: None (disk-based)

## Maintenance Checklist

### Daily
- [ ] Check `db_stats` for anomalies
- [ ] Monitor query response times

### Weekly
- [ ] Review slow query logs
- [ ] Check index usage with `explain`

### Monthly
- [ ] Run `db_compact` on high-churn collections
- [ ] Verify backup restoration
- [ ] Review and cleanup unused indexes

## Troubleshooting

### Database Won't Open
1. Check file permissions
2. Verify no other process holds the lock
3. Check WAL for corruption

### Slow Queries
1. Run `explain` to check plan
2. Create appropriate indexes
3. Use projections and limits

### High Disk Usage
1. Run `db_compact` on collections
2. Check for orphaned collections
3. Review document sizes"#
                }
            }
        ]
    })
}

pub fn security_guide() -> Value {
    json!({
        "messages": [
            {
                "role": "user",
                "content": {
                    "type": "text",
                    "text": r#"# Security Guide

Comprehensive guide for securing your IronBase deployment.

## Security Layers

```
┌─────────────────────────────────────────┐
│           TLS/HTTPS (Encryption)         │
├─────────────────────────────────────────┤
│         API Key Authentication           │
├─────────────────────────────────────────┤
│      ACL (Collection Permissions)        │
├─────────────────────────────────────────┤
│     Interface Type (localhost/etc)       │
└─────────────────────────────────────────┘
```

## 1. API Key Authentication

### Enable API Keys
Set in config.toml:
```toml
[security]
require_api_key = true
api_key_cache_ttl = 60
```

Set admin key via environment:
```bash
export IRONBASE_ADMIN_KEY="your-secure-admin-key"
```

### Create API Key
```json
// admin_apikey_create tool (requires admin_key)
{
  "admin_key": "your-admin-key",
  "name": "backend-service"
}
```

Response:
```json
{
  "id": "key_abc123",
  "key": "sk-xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
  "name": "backend-service",
  "created_at": "2024-01-15T10:30:00Z"
}
```

**⚠️ IMPORTANT**: Save the `key` value! It's only shown once.

### List API Keys
```json
// admin_apikey_list tool
{"admin_key": "your-admin-key"}
```

Returns masked keys:
```json
[
  {"id": "key_abc123", "name": "backend-service", "key_preview": "sk-xxxx...xxxx", "enabled": true}
]
```

### Revoke/Delete API Key
```json
// Disable (can re-enable)
{"admin_key": "your-admin-key", "id": "key_abc123"}

// Delete permanently
{"admin_key": "your-admin-key", "id": "key_abc123"}
```

### Using API Keys

Via HTTP Header (recommended):
```bash
curl -X POST https://server:8080/mcp \
    -H "Authorization: Bearer sk-your-api-key" \
    -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/call",...}'
```

Via JSON parameter:
```json
{
  "name": "find",
  "arguments": {
    "api_key": "sk-your-api-key",
    "collection": "users",
    "query": {}
  }
}
```

## 2. TLS/HTTPS Configuration

### Generate Certificates
```bash
# Self-signed (development)
openssl req -x509 -newkey rsa:4096 -keyout key.pem -out cert.pem -days 365 -nodes

# Let's Encrypt (production)
certbot certonly --standalone -d your-domain.com
```

### Enable TLS in config.toml
```toml
[tls]
enabled = true
cert_file = "/path/to/cert.pem"
key_file = "/path/to/key.pem"
```

### Multiple Listeners with TLS
```json
// Add HTTPS listener
{
  "id": "external-https",
  "bind": "0.0.0.0:443",
  "tls": true,
  "cert_path": "/etc/ssl/certs/server.crt",
  "key_path": "/etc/ssl/private/server.key"
}

// Add HTTP listener (internal only)
{
  "id": "internal-http",
  "bind": "192.168.1.100:8080",
  "tls": false
}
```

## 3. ACL (Access Control Lists)

See `acl-guide` prompt for detailed ACL configuration.

Quick example:
```json
{
  "collection": "sensitive_data",
  "rules": [
    {"principal": "interface:localhost", "permissions": "all"},
    {"principal": "apikey:sk-trusted", "permissions": "read,write"},
    {"principal": "interface:internal", "permissions": "read"},
    {"principal": "interface:external", "permissions": ""}
  ]
}
```

## 4. Network Security

### Firewall Rules
```bash
# Allow only internal network
iptables -A INPUT -p tcp --dport 8080 -s 10.0.0.0/8 -j ACCEPT
iptables -A INPUT -p tcp --dport 8080 -j DROP

# Or use UFW
ufw allow from 10.0.0.0/8 to any port 8080
```

### Bind to Specific Interface
```toml
[server]
host = "192.168.1.100"  # Don't use 0.0.0.0 unless needed
port = 8080
```

## Security Checklist

### Deployment
- [ ] Enable TLS for all external connections
- [ ] Set strong `IRONBASE_ADMIN_KEY`
- [ ] Enable `require_api_key` in production
- [ ] Bind to internal interfaces only
- [ ] Configure firewall rules

### API Keys
- [ ] Create separate keys for each service
- [ ] Use meaningful names for tracking
- [ ] Rotate keys periodically
- [ ] Revoke unused keys
- [ ] Store keys securely (vault, env vars)

### ACL
- [ ] Set explicit ACLs on sensitive collections
- [ ] Use `interface:external` with minimal permissions
- [ ] Prefer API key principals over IP-based rules
- [ ] Run `acl_cleanup` periodically

### Monitoring
- [ ] Log authentication failures
- [ ] Monitor for unusual access patterns
- [ ] Set up alerts for admin operations

## Common Security Mistakes

### ❌ Exposing Without Auth
```toml
# WRONG - No authentication on public interface
[server]
host = "0.0.0.0"
[security]
require_api_key = false
```

### ✅ Secure Configuration
```toml
[server]
host = "0.0.0.0"
[security]
require_api_key = true
[tls]
enabled = true
cert_file = "/path/to/cert.pem"
key_file = "/path/to/key.pem"
```

### ❌ Weak Admin Key
```bash
export IRONBASE_ADMIN_KEY="admin"  # WRONG
```

### ✅ Strong Admin Key
```bash
export IRONBASE_ADMIN_KEY="$(openssl rand -hex 32)"  # RIGHT
```

## Security Best Practices

1. **Defense in depth**: Use all security layers
2. **Least privilege**: Grant minimum required permissions
3. **Secure by default**: Enable all security features
4. **Audit regularly**: Review logs and access patterns
5. **Keep updated**: Apply security patches promptly"#
                }
            }
        ]
    })
}

pub fn listener_config() -> Value {
    json!({
        "messages": [
            {
                "role": "user",
                "content": {
                    "type": "text",
                    "text": r#"# Listener Configuration Guide

Configure multiple HTTP/HTTPS endpoints for your IronBase MCP server.

## Why Multiple Listeners?

- Separate internal (HTTP) and external (HTTPS) interfaces
- Different ports for different services
- Interface-specific TLS configurations
- ACL rules based on interface type

## Listener Tools

### List All Listeners
```json
// listener_list tool
{}
```

### Get Listener Details
```json
// listener_get tool
{"id": "internal"}
```

### Add HTTP Listener
```json
// listener_create tool
{
  "id": "internal",
  "bind": "192.168.1.100:8080",
  "tls": false,
  "description": "Internal API endpoint"
}
```

### Add HTTPS Listener
```json
// listener_create tool
{
  "id": "external",
  "bind": "0.0.0.0:443",
  "tls": true,
  "cert_path": "/etc/ssl/certs/server.crt",
  "key_path": "/etc/ssl/private/server.key",
  "description": "Public HTTPS endpoint"
}
```

### Enable/Disable Listener
```json
// listener_enable tool
{"id": "external"}

// listener_disable tool
{"id": "internal"}
```

### Delete Listener
```json
// listener_delete tool
{"id": "old-listener"}
```

## Listener Configuration Fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `id` | string | Yes | Unique identifier |
| `bind` | string | Yes | Address:port (e.g., "0.0.0.0:8080") |
| `tls` | bool | No | Enable HTTPS (default: false) |
| `cert_path` | string | If tls=true | Path to TLS certificate |
| `key_path` | string | If tls=true | Path to TLS private key |
| `enabled` | bool | No | Active status (default: true) |
| `description` | string | No | Human-readable description |

## Common Configurations

### 1. Development Setup
```json
// Single HTTP listener on localhost
{
  "id": "dev",
  "bind": "127.0.0.1:8080",
  "tls": false,
  "description": "Development server"
}
```

### 2. Production with Separate Interfaces
```json
// Internal HTTP (for backend services)
{
  "id": "internal",
  "bind": "10.0.0.50:8080",
  "tls": false,
  "description": "Internal service API"
}

// External HTTPS (for public access)
{
  "id": "external",
  "bind": "0.0.0.0:443",
  "tls": true,
  "cert_path": "/etc/letsencrypt/live/example.com/fullchain.pem",
  "key_path": "/etc/letsencrypt/live/example.com/privkey.pem",
  "description": "Public API endpoint"
}
```

### 3. Microservices Architecture
```json
// Service A endpoint
{
  "id": "service-a",
  "bind": "10.0.1.10:8080",
  "tls": false,
  "description": "Service A database endpoint"
}

// Service B endpoint
{
  "id": "service-b",
  "bind": "10.0.1.20:8080",
  "tls": false,
  "description": "Service B database endpoint"
}

// Admin endpoint (localhost only)
{
  "id": "admin",
  "bind": "127.0.0.1:9090",
  "tls": false,
  "description": "Admin/maintenance endpoint"
}
```

## Integration with ACL

Listeners determine the interface type for ACL:

| Bind Address | Interface Type |
|--------------|----------------|
| 127.0.0.1:* | localhost |
| 10.*.*.* | internal |
| 172.16-31.*.* | internal |
| 192.168.*.* | internal |
| 0.0.0.0:* | depends on client IP |

### Example: Different Permissions per Interface
```json
// ACL for users collection
{
  "collection": "users",
  "rules": [
    {"principal": "interface:localhost", "permissions": "all"},
    {"principal": "interface:internal", "permissions": "read,write"},
    {"principal": "interface:external", "permissions": "read"}
  ]
}
```

## TLS Certificate Setup

### Self-Signed (Development)
```bash
openssl req -x509 -newkey rsa:4096 \
    -keyout key.pem -out cert.pem \
    -days 365 -nodes \
  -subj "/CN=localhost"
```

### Let's Encrypt (Production)
```bash
# Install certbot
apt install certbot

# Get certificate
certbot certonly --standalone -d your-domain.com

# Certificate paths
# /etc/letsencrypt/live/your-domain.com/fullchain.pem
# /etc/letsencrypt/live/your-domain.com/privkey.pem
```

### Certificate Renewal
```bash
# Auto-renewal with certbot
certbot renew --quiet

# Reload server after renewal
systemctl reload ironbase-mcp
```

## Storage

Listener configurations are stored in `_system.listeners` collection:
- Localhost-only write access
- Persisted across server restarts
- Can be managed via listener_* tools

## Best Practices

1. **Separate internal/external**: Use different listeners for different trust levels
2. **Always TLS for external**: Never expose HTTP to the internet
3. **Bind to specific IPs**: Avoid 0.0.0.0 unless necessary
4. **Use meaningful IDs**: "internal-api" not "listener1"
5. **Document with descriptions**: Helps with maintenance
6. **Disable unused listeners**: Don't leave test listeners active

## Troubleshooting

### Port Already in Use
```bash
# Find process using port
lsof -i :8080
netstat -tlnp | grep 8080

# Kill if needed
kill -9 <PID>
```

### Certificate Errors
```bash
# Verify certificate
openssl x509 -in cert.pem -text -noout

# Check key matches cert
openssl x509 -noout -modulus -in cert.pem | md5sum
openssl rsa -noout -modulus -in key.pem | md5sum
# Should match!
```

### Listener Not Starting
1. Check bind address is valid
2. Verify port is available
3. Ensure cert/key paths are correct
4. Check file permissions on certificates"#
                }
            }
        ]
    })
}
