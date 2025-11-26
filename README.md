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
| **Query** | 18 operators: comparison, logical, element, array, regex |
| **Update** | 7 operators: `$set`, `$inc`, `$unset`, `$push`, `$pull`, `$addToSet`, `$pop` |
| **Aggregation** | 6 stages + 6 accumulators with dot notation support |
| **Indexing** | B+ tree indexes, compound indexes, explain(), hint() |
| **Durability** | ACD transactions, WAL, crash recovery, 3 durability modes |
| **Performance** | ~1M+ inserts/sec, O(log n) index lookups |
| **Languages** | Rust, Python (PyO3), C# (.NET 8) |
| **Testing** | 554+ tests, property-based testing, fuzz testing |

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

var client = new IronBaseClient("myapp.mlite");
var users = client.GetCollection<User>("users");

// Insert
users.InsertOne(new User { Name = "Alice", Age = 30 });

// Query
var adults = users.Find(Builders<User>.Filter.Gte("Age", 18));

// Update
users.UpdateOne(
    Builders<User>.Filter.Eq("Name", "Alice"),
    Builders<User>.Update.Set("Age", 31)
);

client.Dispose();
```

### Rust
```rust
use ironbase_core::{DatabaseCore, storage::StorageEngine};
use serde_json::json;

let db = DatabaseCore::<StorageEngine>::open("myapp.mlite")?;
let users = db.collection("users")?;

users.insert_one(&json!({"name": "Alice", "age": 30}))?;
let results = users.find(&json!({"age": {"$gte": 18}}))?;

db.close()?;
```

## Installation

### Python (PyPI)
```bash
pip install ironbase
```

Supported: Linux (x86_64, aarch64), Windows (x64), macOS (Intel, Apple Silicon)

### C# (NuGet)
```bash
dotnet add package IronBase
```

### Rust (From Source)
```bash
git clone https://github.com/petitan/MongoLite.git
cd MongoLite
cargo build --release -p ironbase-core
```

## Query Operators

### Comparison
| Operator | Description | Example |
|----------|-------------|---------|
| `$eq` | Equal | `{"age": {"$eq": 25}}` or `{"age": 25}` |
| `$ne` | Not equal | `{"status": {"$ne": "inactive"}}` |
| `$gt` | Greater than | `{"age": {"$gt": 18}}` |
| `$gte` | Greater or equal | `{"score": {"$gte": 90}}` |
| `$lt` | Less than | `{"price": {"$lt": 100}}` |
| `$lte` | Less or equal | `{"count": {"$lte": 10}}` |
| `$in` | In array | `{"city": {"$in": ["NYC", "LA"]}}` |
| `$nin` | Not in array | `{"status": {"$nin": ["deleted", "banned"]}}` |

### Logical
| Operator | Description | Example |
|----------|-------------|---------|
| `$and` | Logical AND | `{"$and": [{"age": {"$gte": 18}}, {"city": "NYC"}]}` |
| `$or` | Logical OR | `{"$or": [{"city": "NYC"}, {"city": "LA"}]}` |
| `$not` | Logical NOT | `{"age": {"$not": {"$gt": 30}}}` |
| `$nor` | Logical NOR | `{"$nor": [{"deleted": true}, {"banned": true}]}` |

### Element
| Operator | Description | Example |
|----------|-------------|---------|
| `$exists` | Field exists | `{"email": {"$exists": true}}` |
| `$type` | Type check | `{"age": {"$type": "number"}}` |

### Array
| Operator | Description | Example |
|----------|-------------|---------|
| `$all` | Contains all | `{"tags": {"$all": ["a", "b"]}}` |
| `$elemMatch` | Element matches | `{"scores": {"$elemMatch": {"$gt": 80}}}` |
| `$size` | Array length | `{"tags": {"$size": 3}}` |

### String
| Operator | Description | Example |
|----------|-------------|---------|
| `$regex` | Regex match | `{"name": {"$regex": "^A"}}` |

### Dot Notation (Nested Fields)
```python
# Query nested fields
users.find({"address.city": "NYC"})
users.find({"stats.score": {"$gte": 90}})

# Update nested fields
users.update_one(
    {"name": "Alice"},
    {"$set": {"address.city": "Boston"}}
)

# Sort by nested fields
users.find({}, sort=[("address.zip", 1)])

# Project nested fields
users.find({}, projection={"address.city": 1})

# Aggregation with nested fields
users.aggregate([
    {"$group": {"_id": "$address.city", "total": {"$sum": "$stats.score"}}}
])
```

## Update Operators

| Operator | Description | Example |
|----------|-------------|---------|
| `$set` | Set field value | `{"$set": {"name": "Bob", "age": 30}}` |
| `$inc` | Increment number | `{"$inc": {"score": 10, "attempts": 1}}` |
| `$unset` | Remove field | `{"$unset": {"temp_field": ""}}` |
| `$push` | Add to array | `{"$push": {"tags": "new_tag"}}` |
| `$pull` | Remove from array | `{"$pull": {"tags": "old_tag"}}` |
| `$addToSet` | Add unique to array | `{"$addToSet": {"tags": "unique_tag"}}` |
| `$pop` | Remove first/last | `{"$pop": {"queue": 1}}` (last) or `{"$pop": {"queue": -1}}` (first) |

## Find Options

```python
# Projection (field selection)
users.find({}, projection={"name": 1, "age": 1, "_id": 0})  # Include mode
users.find({}, projection={"password": 0})  # Exclude mode

# Sorting
users.find({}, sort=[("age", 1)])  # Ascending
users.find({}, sort=[("age", -1)])  # Descending
users.find({}, sort=[("city", 1), ("age", -1)])  # Multi-field

# Pagination
users.find({}, skip=20, limit=10)  # Page 3 (20 skip, 10 per page)

# Combined
users.find(
    {"status": "active"},
    projection={"name": 1, "score": 1},
    sort=[("score", -1)],
    limit=100
)
```

## Aggregation Pipeline

### Stages

| Stage | Description |
|-------|-------------|
| `$match` | Filter documents (like find) |
| `$group` | Group by field, compute aggregates |
| `$project` | Reshape documents (include/exclude/rename) |
| `$sort` | Sort documents |
| `$limit` | Limit result count |
| `$skip` | Skip documents |

### Accumulators (in $group)

| Accumulator | Description |
|-------------|-------------|
| `$sum` | Sum values or count (`{"$sum": 1}`) |
| `$avg` | Average value |
| `$min` | Minimum value |
| `$max` | Maximum value |
| `$first` | First value in group |
| `$last` | Last value in group |

### Example Pipeline

```python
# Sales analytics with nested field support
results = sales.aggregate([
    # Filter completed sales
    {"$match": {"status": "completed"}},

    # Group by store city (nested field)
    {"$group": {
        "_id": "$store.location.city",
        "totalRevenue": {"$sum": "$payment.amount"},
        "orderCount": {"$sum": 1},
        "avgOrder": {"$avg": "$payment.amount"},
        "maxOrder": {"$max": "$payment.amount"}
    }},

    # Reshape output
    {"$project": {
        "city": "$_id",
        "revenue": "$totalRevenue",
        "orders": "$orderCount",
        "avgOrder": 1,
        "_id": 0
    }},

    # Sort by revenue
    {"$sort": {"revenue": -1}},

    # Top 10
    {"$limit": 10}
])
```

See [AGGREGATION.md](AGGREGATION.md) for detailed documentation.

## Indexing

```python
# Create indexes
users.create_index("email", unique=True)
users.create_index("age")
users.create_compound_index(["country", "city"])

# List indexes
print(users.list_indexes())  # ['users_id', 'users_email', 'users_age', ...]

# Query plan analysis
plan = users.explain({"age": {"$gte": 25}})
print(plan["queryPlan"])   # "IndexRangeScan"
print(plan["indexUsed"])   # "users_age"

# Force index usage
results = users.find_with_hint({"age": 25}, "users_age")

# Drop index
users.drop_index("users_age")
```

See [INDEXES.md](INDEXES.md) for detailed documentation.

## Durability Modes

IronBase offers three durability modes for different use cases:

### Safe Mode (Default)
```python
db = IronBase("app.mlite")  # or durability="safe"
```
- **ZERO data loss** - Every operation immediately persisted
- ~200 ops/sec (with fsync)
- Use for: Financial data, user accounts, critical business data

### Batch Mode
```python
db = IronBase("app.mlite", durability="batch", batch_size=100)
```
- **Bounded loss** - Max `batch_size` operations can be lost
- ~500 ops/sec (batched fsync)
- Use for: Logs, analytics, session tracking

### Unsafe Mode
```python
db = IronBase("app.mlite", durability="unsafe")
db.checkpoint()  # Manual commit required!
```
- **Manual control** - High data loss risk without checkpoint()
- ~500 ops/sec
- Use for: Temporary data, bulk imports, benchmarks

## Transactions (ACD)

```python
# Begin transaction
tx_id = db.begin_transaction()

try:
    db.insert_one_tx("accounts", {"id": 1, "balance": 1000}, tx_id)
    db.update_one_tx("accounts", {"id": 2}, {"balance": 500}, tx_id)

    # Atomic commit
    db.commit_transaction(tx_id)
except:
    # Rollback on error
    db.rollback_transaction(tx_id)
    raise
```

Features:
- Atomicity: All-or-nothing execution
- Consistency: Maintains data integrity
- Durability: WAL + crash recovery

## In-Memory Mode

For testing (10-100x faster than file-based):

```python
db = IronBase(":memory:")
users = db.collection("users")
users.insert_one({"name": "test"})
# Data discarded when process ends
```

## Cursor/Streaming

For large result sets:

```python
cursor = collection.find_cursor({"status": "active"}, batch_size=500)

print(f"Total: {cursor.total()}")

# Process in batches
while not cursor.is_finished():
    batch = cursor.next_batch()
    for doc in batch:
        process(doc)

# Or one at a time
cursor.rewind()
while (doc := cursor.next()) is not None:
    process(doc)
```

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    Language Bindings                         │
│         Python (PyO3)  │  C# (.NET)  │  (future: JS/Go)     │
└──────────────┬──────────────┬───────────────────────────────┘
               │              │
┌──────────────▼──────────────▼───────────────────────────────┐
│                    ironbase-core (Rust)                      │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────┐  │
│  │   Query     │  │ Aggregation │  │      Indexing       │  │
│  │   Engine    │  │   Pipeline  │  │   (B+ Tree)         │  │
│  └─────────────┘  └─────────────┘  └─────────────────────┘  │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────┐  │
│  │ Transaction │  │     WAL     │  │   Query Planner     │  │
│  │   Manager   │  │   Manager   │  │   & Cache           │  │
│  └─────────────┘  └─────────────┘  └─────────────────────┘  │
└──────────────────────────┬──────────────────────────────────┘
                           │
┌──────────────────────────▼──────────────────────────────────┐
│                    Storage Engine                            │
│     Append-only storage  │  Compaction  │  Crash recovery   │
└─────────────────────────────────────────────────────────────┘
```

### File Format (.mlite)
```
┌───────────────────────────────┐
│  Header (128 bytes)           │
│  - Magic: "MONGOLTE"          │
│  - Version, page_size         │
├───────────────────────────────┤
│  Collection Metadata (JSON)   │
│  - Document catalog           │
│  - Indexes, schema            │
├───────────────────────────────┤
│  Document Data (append-only)  │
│  [len][JSON bytes]...         │
└───────────────────────────────┘
```

## Project Structure

```
MongoLite/
├── ironbase-core/           # Rust core library
│   └── src/
│       ├── database.rs      # DatabaseCore
│       ├── collection_core/ # CRUD operations
│       ├── query/           # Query operators
│       ├── aggregation.rs   # Pipeline stages
│       ├── index.rs         # B+ tree indexes
│       ├── storage/         # Storage engine
│       ├── transaction.rs   # ACD transactions
│       └── wal.rs           # Write-ahead log
├── bindings/python/         # Python bindings (PyO3)
├── IronBase.NET/            # C# bindings
└── mcp-server/              # MCP server for AI assistants
```

## Testing

```bash
# Rust tests (554+ tests)
cargo test -p ironbase-core

# Python tests
python run_all_tests.py

# C# tests
cd IronBase.NET && dotnet test

# Development checks
just run-dev-checks  # fmt + clippy + tests
```

## Performance

| Operation | Performance |
|-----------|-------------|
| Insert (Safe mode) | ~200 ops/sec |
| Insert (Batch mode) | ~500 ops/sec |
| Insert (bulk, unsafe) | ~1M+ ops/sec |
| Index lookup | O(log n) |
| Range scan | O(log n + k) |
| Full scan | O(n) |

## Limitations

- No ACID isolation (ACD only - no MVCC)
- No cursors with server-side state (client-side only)
- No replication or sharding
- Single-writer model
- No geospatial or full-text indexes (planned)

## Documentation

- [AGGREGATION.md](AGGREGATION.md) - Aggregation pipeline guide
- [INDEXES.md](INDEXES.md) - Indexing guide
- [DESIGN_AUTO_COMMIT.md](DESIGN_AUTO_COMMIT.md) - Durability design
- [INDEX_CONSISTENCY.md](INDEX_CONSISTENCY.md) - Index consistency guarantees
- [IronBase.NET/README.md](IronBase.NET/README.md) - C# documentation

## License

MIT License

## Contributing

1. Fork the repository
2. Create a feature branch
3. Run `just run-dev-checks` before committing
4. Submit a pull request

---

**IronBase** - MongoDB's simplicity with SQLite's elegance
