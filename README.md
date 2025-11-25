# ironbase

**Embedded NoSQL document database** with MongoDB-compatible API, written in Rust with Python bindings.

## Features

- 🎯 **MongoDB-compatible API** - Familiar syntax and operations
- 📦 **Embedded** - No separate server needed
- 🚀 **Fast** - Rust-powered native performance with B+ tree indexes
- 💾 **Single file** - Simple backup and version control
- 🔧 **Zero-config** - No installation or setup required
- 🐍 **Python API** - Easy to use from Python
- 🧪 **In-memory mode** - 10-100x faster for testing, no file I/O
- 🔍 **Full indexing support** - B+ tree indexes with automatic query optimization
- 🔗 **Compound indexes** - Multi-field indexes for complex queries
- 📊 **Query explanation** - See which indexes are used with `explain()`
- 🔄 **Aggregation Pipeline** - MongoDB-compatible data processing with $match, $group, $project, $sort, $limit, $skip
- 🔎 **Advanced find()** - Projection, sort, limit, skip for powerful queries
- 📜 **Cursor/Streaming** - Memory-efficient iteration over large result sets
- ⚡ **Performance** - 1.26M inserts/sec, 1.39µs index lookups, 1.4-1.6x query speedup
- ✅ **400+ tests passing** - Comprehensive test coverage (85%+) including ACD transactions, crash recovery, property-based tests
- 🌐 **Multi-language support** - Rust core with language-specific bindings (Python, C# planned)
- 🔒 **ACD Transactions** - Atomicity, Consistency, Durability with Write-Ahead Log and crash recovery (Python API ✅)
- 🛡️ **Auto-commit Durability Modes** - Safe (ZERO data loss), Batch (bounded loss), Unsafe (manual checkpoint) - configurable per database

## 🎯 Célközönség

ironbase tökéletes választás:
- Desktop alkalmazásokhoz
- Mobil app backend-ekhez
- Prototípusokhoz és MVP-khez
- IoT eszközökhöz
- Kis és közepes adatbázisokhoz
- Amikor nem akarsz MongoDB szervert futtatni

## 🔧 Telepítés

### Előfeltételek
- **Python 3.8+**
- **Rust 1.70+** (build-hez)
- **Windows**: Microsoft C++ Build Tools (lásd [BUILD.md](BUILD.md))

### Pip-el (Ajánlott - PyPI-ról, minden platform)

```bash
pip install ironbase
```

Támogatott platformok:
- **Linux** (x86_64, aarch64) - manylinux
- **Windows** (x64, x86) - win_amd64, win32
- **macOS** (Intel, Apple Silicon) - universal2

### Maturin-nal (Fejlesztőknek - build from source)

#### Linux / macOS
```bash
# Rust és Python környezet előkészítése
pip install maturin

# Development build
maturin develop

# Release build
maturin build --release
```

#### Windows
```powershell
# Előfeltételek: Rust + Microsoft C++ Build Tools (lásd BUILD.md)
pip install maturin

# Development build
maturin develop

# Release build
maturin build --release
```

**Részletes build instrukciók:** [BUILD.md](BUILD.md)

## 🚀 Gyors Kezdés

```python
from ironbase import ironbase

# Adatbázis megnyitása (létrehozza, ha nem létezik)
# Default: Safe mode (ZERO data loss, auto-commit every operation)
db = ironbase("myapp.mlite")

# Vagy: Batch mode (high throughput, bounded data loss risk)
# db = ironbase("myapp.mlite", durability="batch", batch_size=100)

# Vagy: Unsafe mode (maximum performance, manual checkpoint required)
# db = ironbase("myapp.mlite", durability="unsafe")

# Collection lekérése
users = db.collection("users")

# Dokumentum beszúrása
result = users.insert_one({
    "name": "Kovács János",
    "email": "janos@example.com",
    "age": 30,
    "city": "Budapest"
})
print(f"Beszúrva: {result['inserted_id']}")

# Több dokumentum beszúrása
users.insert_many([
    {"name": "Nagy Anna", "age": 25, "city": "Szeged"},
    {"name": "Szabó Péter", "age": 35, "city": "Debrecen"}
])

# Dokumentumok számlálása
count = users.count_documents()
print(f"Összes felhasználó: {count}")

# Index létrehozása (gyorsabb lekérdezésekhez)
users.create_index("age")

# Lekérdezés (automatikusan használja az indexet)
adults = users.find({"age": {"$gte": 18}})

# Query terv megtekintése
plan = users.explain({"age": {"$gte": 18}})
print(f"Query plan: {plan['queryPlan']}")  # IndexRangeScan

# Bezárás
db.close()
```

## 🧰 Fejlesztői workflow (lokális)

Az ismétlődő build/test lépésekre felkerült egy **justfile** és egy egyszerű futtató script:

| Parancs | Mit csinál |
| --- | --- |
| `just test-core` | `cargo test -p ironbase-core` |
| `just test-mcp` | MCP szerver Rust tesztek (`cd mcp-server && cargo test`) |
| `just seed-test-doc` | Aktiválja a `venv`-et és lefuttatja a `mcp-server/seed_test_doc.py`-t |
| `just test-python-auto` | Python auto-commit smoke teszt (`test_python_auto_commit.py`) |
| `just run-dev-checks` | A `scripts/run_dev_checks.sh` fut: fmt + clippy + Rust tesztek + Python smoke teszt |

A `scripts/run_dev_checks.sh` Bash script egymás után lefuttatja:

1. `cargo fmt`, `cargo clippy`, `cargo test -p ironbase-core`
2. `cd mcp-server && cargo fmt && cargo clippy && cargo test`
3. ha van `venv`, akkor `python3 mcp-server/test_python_auto_commit.py`

Használat:

```bash
# egyszerűen
just run-dev-checks

# vagy közvetlenül
./scripts/run_dev_checks.sh
```

Ezekkel a parancsokkal helyben is gyorsan végigfuthat a fő Rust + Python ellenőrzés, mielőtt manuális E2E teszteket futtatnánk.

## 📚 API Dokumentáció

### Database (ironbase)

```python
# Adatbázis megnyitása
db = ironbase("path/to/database.mlite")

# Adatbázis megnyitása durability móddal
db = ironbase("path/to/database.mlite", durability="safe")  # default
db = ironbase("path/to/database.mlite", durability="batch", batch_size=100)
db = ironbase("path/to/database.mlite", durability="unsafe")

# Collection lekérése (létrehozza, ha nincs)
collection = db.collection("collection_name")

# Collection-ök listázása
collections = db.list_collections()

# Collection törlése
db.drop_collection("collection_name")

# Statisztikák
stats = db.stats()

# Manual checkpoint (csak Unsafe módban szükséges)
db.checkpoint()

# Bezárás
db.close()
```

### 🧪 In-Memory Database (Testing)

Az in-memory mód **10-100x gyorsabb** mint a fájl-alapú storage, tökéletes unit tesztekhez:

```python
from ironbase import ironbase

# In-memory database (nincs fájl, nincs perzisztencia)
db = ironbase(":memory:")

# Használat pont ugyanaz mint a fájl-alapú
users = db.collection("users")
users.insert_one({"name": "Alice", "age": 30})

# Tesztek után automatikusan törlődik
```

**Rust API:**
```rust
use ironbase_core::{DatabaseCore, storage::MemoryStorage};

// In-memory database
let db = DatabaseCore::<MemoryStorage>::open_memory()?;
let users = db.collection("users")?;

users.insert_one(HashMap::from([
    ("name".to_string(), json!("Alice")),
]))?;
```

**Mikor használd az in-memory módot:**
- ✅ Unit tesztek (gyors, izolált)
- ✅ Integration tesztek
- ✅ Prototípusok
- ✅ Benchmarkok

**⚠️ Figyelem:** Az in-memory mód NEM perzisztál - a process végén minden adat elveszik! Production-ben használd a fájl-alapú módot (`ironbase("myapp.mlite")`), ami teljes WAL + crash recovery támogatással rendelkezik.

### Durability Modes (Auto-Commit)

ironbase három durability módot kínál, amelyek különböző kompromisszumokat kínálnak a teljesítmény és adatbiztonság között:

#### 🛡️ Safe Mode (Default)

**ZERO data loss guarantee** - Minden művelet azonnal commit-olva van WAL-lal + fsync.

```python
db = ironbase("myapp.mlite")  # Safe mode alapértelmezett
# VAGY explicit:
db = ironbase("myapp.mlite", durability="safe")

users = db.collection("users")
users.insert_one({"name": "Alice"})  # Azonnal perzisztálva
# ⚡ Power failure → 0 adat veszteség
```

**Jellemzők:**
- ✅ **ZERO data loss**: Minden művelet garantáltan megőrzött
- ✅ **Auto-commit**: Minden insert/update/delete azonnal WAL-ba írva
- ✅ **Crash recovery**: WAL replay automatikusan visszaállít minden műveletet
- ⚠️ **Teljesítmény**: ~190 ops/sec (40% of unsafe, de BIZTONSÁGOS)

**Használati esetek:**
- 💰 Pénzügyi tranzakciók
- 👤 Felhasználói fiókok/profilok
- 🛒 E-commerce rendelések
- 📝 Kritikus üzleti adatok

#### ⚡ Batch Mode

**Bounded data loss** - Műveletek kötegekben commit-olva, maximum `batch_size` művelet veszhet el.

```python
db = ironbase("myapp.mlite", durability="batch", batch_size=100)

logs = db.collection("logs")
for i in range(1000):
    logs.insert_one({"event": f"Event {i}"})
    # Minden 100. műveletnél automatikus flush

# Manual flush (optional):
db.checkpoint()  # Azonnal commit-ol minden függőben levő műveletet
```

**Jellemzők:**
- ✅ **Bounded loss**: Maximum `batch_size` művelet veszhet el power failure esetén
- ✅ **High throughput**: ~490 ops/sec (104% of unsafe! Batch gyorsabb!)
- ✅ **Auto-flush**: Automatikus commit minden N. műveletnél
- ⚠️ **Data loss risk**: Max `batch_size` műveletnél (pl. max 100 ops)

**Használati esetek:**
- 📊 Alkalmazás logok (batch_size=100-1000)
- 📈 Analytics események (batch_size=1000-5000)
- 🔍 Session tracking (batch_size=100-500)
- 📡 Telemetria adatok

#### 🚀 Unsafe Mode

**Manual checkpoint required** - Nincs auto-commit, maximális teljesítmény, nagy adatvesztési kockázat.

```python
db = ironbase("myapp.mlite", durability="unsafe")

temp = db.collection("staging")
for i in range(10000):
    temp.insert_one({"data": i})  # Gyors, de nem perzisztálva

# KÖTELEZŐ: Manual checkpoint
db.checkpoint()  # Most történik a WAL write + fsync

# ⚡ Power failure checkpoint() előtt → MINDEN adat elveszhet
```

**Jellemzők:**
- ❌ **HIGH data loss risk**: Minden adat elveszhet checkpoint() nélkül
- ✅ **Maximum speed**: ~472 ops/sec baseline (de batch modes gyorsabbak!)
- ⚠️ **Manual control**: Fejlesztő felelőssége a checkpoint() hívás
- ✅ **Use case**: Temporary/staging data, ahol újrafuttatható az import

**Használati esetek:**
- 🔄 Temporary staging data (újrafuttatható import)
- 🧪 Teszt/fejlesztési környezet
- 📦 Bulk import (retry safe, újra lehet futtatni hiba esetén)
- 🎯 Performance benchmarks

#### 📊 Performance Comparison

Benchmark eredmények (1000 dokumentum insert):

| Mode        | Throughput (ops/sec) | Relative | Safety                   | Use Case                |
|-------------|----------------------|----------|--------------------------|-------------------------|
| **Safe**    | 190                  | 40%      | ✅ ZERO loss             | Production (critical)   |
| **Batch-10**| 402                  | 85%      | ⚠️ Max 10 ops            | High-frequency logs     |
| **Batch-100**| 489                 | 104%     | ⚠️ Max 100 ops           | **RECOMMENDED** (balance)|
| **Batch-500**| 498                 | 105%     | ⚠️ Max 500 ops           | Analytics events        |
| **Unsafe**  | 472                  | 100%     | ❌ HIGH risk             | Temp/staging only       |

**Meglepő eredmény:** Batch modes (100, 500) GYORSABBAK mint az Unsafe mode! Ez a batch flushing optimalizációjának köszönhető.

#### 🎯 Recommendations

**Financial/Critical Data:**
```python
db = ironbase("production.mlite", durability="safe")  # ZERO data loss
```

**High-Throughput Logs:**
```python
db = ironbase("logs.mlite", durability="batch", batch_size=100)  # Best balance
```

**Temporary Staging:**
```python
db = ironbase("staging.mlite", durability="unsafe")
# ... bulk operations ...
db.checkpoint()  # Manual commit at the end
```

**Default Recommendation:** Use **Safe mode** for production data (like SQL databases). Only use Batch/Unsafe if you understand the trade-offs.

**Részletes dokumentáció:** Lásd [DESIGN_AUTO_COMMIT.md](DESIGN_AUTO_COMMIT.md) a teljes tervezési döntésekért, algoritmusokért és benchmark eredményekért.

### Transactions (ACD)

ironbase támogat **ACD tranzakciókat** (Atomicity, Consistency, Durability) Write-Ahead Log (WAL) alapú crash recovery-vel.

```python
# Transaction indítása
tx_id = db.begin_transaction()

# Műveletek hozzáadása (jelenleg még csak core szinten)
# TODO: Collection-level transaction methods (jövőbeli feature)

# Commit (atomi alkalmazás + WAL)
db.commit_transaction(tx_id)

# VAGY: Rollback (minden művelet eldobása)
db.rollback_transaction(tx_id)
```

**Error Handling:**

```python
tx_id = db.begin_transaction()
try:
    # ... operations ...
    db.commit_transaction(tx_id)
except Exception as e:
    db.rollback_transaction(tx_id)
    raise
```

**Jellemzők:**
- ✅ **Atomicity**: Minden művelet együtt végrehajtva vagy egyáltalán nem
- ✅ **Consistency**: Adatintegritás fenntartása
- ✅ **Durability**: WAL + dual fsync biztosítja az adatok megőrzését crash után
- ✅ **9-lépéses commit protokoll** CRC32 checksumokkal
- 📖 Részletek: `IMPLEMENTATION_ACD.md`, `INDEX_CONSISTENCY.md`

### Collection

#### INSERT műveletek

```python
# Egy dokumentum
result = collection.insert_one({
    "field1": "value1",
    "field2": 123
})
# Eredmény: {"acknowledged": True, "inserted_id": 1}

# Több dokumentum
result = collection.insert_many([
    {"name": "Item 1"},
    {"name": "Item 2"}
])
# Eredmény: {"acknowledged": True, "inserted_ids": [1, 2]}
```

#### READ operations

```python
# Find one document
doc = collection.find_one({"name": "János"})

# Find all documents
all_docs = collection.find({})

# Find with filters
filtered = collection.find({"age": {"$gt": 25}})

# Find with projection (field filtering)
docs = collection.find(
    {},
    projection={"name": 1, "age": 1, "_id": 0}  # Include name, age; exclude _id
)

# Find with sort
docs = collection.find({}, sort=[("age", 1)])  # Sort by age ascending
docs = collection.find({}, sort=[("age", -1)])  # Sort by age descending
docs = collection.find({}, sort=[("city", 1), ("age", -1)])  # Multi-field sort

# Find with limit and skip (pagination)
docs = collection.find({}, limit=10)  # First 10 documents
docs = collection.find({}, skip=5, limit=10)  # Documents 6-15

# Combined: query + projection + sort + limit
results = collection.find(
    {"age": {"$gte": 18}},              # Query
    projection={"name": 1, "age": 1},   # Projection
    sort=[("age", -1)],                 # Sort
    limit=10                            # Limit
)

# Count documents
count = collection.count_documents()
count_filtered = collection.count_documents({"city": "Budapest"})

# Get distinct values
ages = collection.distinct("age")
cities = collection.distinct("city", {"active": True})
```

#### UPDATE operations

```python
# Update one document
result = collection.update_one(
    {"name": "János"},
    {"$set": {"age": 31, "updated": True}}
)

# Update many documents
result = collection.update_many(
    {"city": "Budapest"},
    {"$set": {"country": "Hungary"}}
)

# Increment/decrement
collection.update_one(
    {"name": "János"},
    {"$inc": {"score": 10, "attempts": 1}}
)

# Remove fields
collection.update_one(
    {"name": "János"},
    {"$unset": {"temp_field": ""}}
)
```

#### DELETE operations

```python
# Delete one document
result = collection.delete_one({"name": "János"})

# Delete many documents
result = collection.delete_many({"age": {"$lt": 18}})
```

#### INDEX operations

```python
# Create non-unique index
collection.create_index("age")

# Create unique index
collection.create_index("email", unique=True)

# Create compound index (multi-field)
collection.create_compound_index(["country", "city"])
collection.create_compound_index(["category", "price"], unique=True)

# List all indexes
indexes = collection.list_indexes()
# Returns: ['users_id', 'users_age', 'users_country_city']

# Explain query execution plan
plan = collection.explain({"age": {"$gte": 18}})
print(plan["queryPlan"])      # "IndexRangeScan"
print(plan["indexUsed"])      # "users_age"
print(plan["estimatedCost"])  # "O(log n + k)"

# Manual index selection (hint)
results = collection.find_with_hint(
    {"age": 25},
    "users_age"  # Force use of this index
)

# Drop an index
collection.drop_index("users_age")
```

**Compound Index példa:**
```python
# E-commerce: termékek country + city szerinti gyors keresése
products = db.collection("products")
products.create_compound_index(["country", "city"])

# Ez a query használja a compound indexet
results = products.find({"country": "HU", "city": "Budapest"})
```

**For detailed index documentation, see [INDEXES.md](INDEXES.md)**

#### AGGREGATION operations

```python
# Aggregation pipeline
results = collection.aggregate([
    {"$match": {"age": {"$gte": 18}}},
    {"$group": {"_id": "$city", "count": {"$sum": 1}, "avgAge": {"$avg": "$age"}}},
    {"$sort": {"count": -1}},
    {"$limit": 10}
])

# Available stages: $match, $group, $project, $sort, $limit, $skip
# Available accumulators: $sum, $avg, $min, $max, $first, $last
```

**For detailed aggregation documentation, see [AGGREGATION.md](AGGREGATION.md)**

#### CURSOR / STREAMING operations

Nagy eredményhalmazok memória-hatékony feldolgozásához:

```python
# Cursor létrehozása (nem tölti be az összes dokumentumot egyszerre)
cursor = collection.find_streaming({"status": "active"})

print(f"Total: {cursor.total()}")        # Összes találat
print(f"Remaining: {cursor.remaining()}") # Hátralévő

# Iterálás egyenként
doc = cursor.next()

# Batch-ekben feldolgozás (hatékonyabb)
batch = cursor.next_batch(100)  # Következő 100 dokumentum

# Skip (átugrás)
cursor.skip(50)

# Visszaugrás az elejére
cursor.rewind()

# Első N dokumentum
first_10 = cursor.take(10)

# Összes begyűjtése (ha elfér memóriában)
all_docs = cursor.collect_all()

# For-each feldolgozás
cursor.for_each(lambda doc: print(doc["name"]))
```

**Rust API:**
```rust
let mut cursor = collection.find_streaming(&json!({}))?;

// Batch feldolgozás
while cursor.remaining() > 0 {
    let batch = cursor.next_chunk(100)?;
    process_batch(batch);
}
```

**Mikor használd:**
- 📊 Nagy adathalmazok (>10,000 dokumentum)
- 💾 Memória-korlátozott környezet
- 🔄 Streaming feldolgozás
- 📄 Lapozás (pagination)

#### Complex Queries

```python
# Logical AND
results = collection.find({
    "$and": [
        {"age": {"$gte": 25}},
        {"city": "NYC"}
    ]
})

# Logical OR
results = collection.find({
    "$or": [
        {"age": {"$lt": 25}},
        {"city": "LA"}
    ]
})

# NOT operator
results = collection.find({
    "age": {"$not": {"$gt": 30}}
})

# Complex nested query
results = collection.find({
    "$and": [
        {
            "$or": [
                {"city": "NYC"},
                {"city": "LA"}
            ]
        },
        {"age": {"$gte": 25}},
        {"active": True}
    ]
})
```

## Supported Query Operators

### Comparison Operators ✅
- `$eq` - Equal to
- `$ne` - Not equal to
- `$gt` - Greater than
- `$gte` - Greater than or equal
- `$lt` - Less than
- `$lte` - Less than or equal
- `$in` - Value in array
- `$nin` - Value not in array

### Logical Operators ✅
- `$and` - Logical AND
- `$or` - Logical OR
- `$not` - Logical NOT
- `$nor` - Logical NOR

### Update Operators ✅
- `$set` - Set field value
- `$inc` - Increment/decrement numeric field
- `$unset` - Remove field
- `$push` - Add to array
- `$pull` - Remove from array
- `$addToSet` - Add unique to array
- `$pop` - Remove first/last from array

### Element Operators ✅
- `$exists` - Field exists check
- `$type` - Type check (string, number, boolean, object, array)

### Array Operators ✅
- `$all` - Array contains all values
- `$elemMatch` - Array element matches condition
- `$size` - Array size check

### String Operators ✅
- `$regex` - Regular expression match

### Planned Operators
- `$expr` - Aggregation expressions in queries
- `$text` - Full-text search

## 🏗️ Architektúra

### Cargo Workspace Structure

```
ironbase/
├── Cargo.toml                    # Workspace root
├── ironbase-core/               # 🦀 Pure Rust Core Library
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs                # Clean API exports
│       ├── database.rs           # DatabaseCore (language-independent)
│       ├── collection_core.rs    # CollectionCore (pure logic)
│       ├── storage.rs            # Storage engine
│       ├── query.rs              # Query engine
│       ├── document.rs           # Document model
│       ├── error.rs              # Error handling
│       └── index.rs              # Indexing (future)
└── bindings/
    ├── python/                   # 🐍 Python Bindings (PyO3)
    │   ├── Cargo.toml
    │   └── src/
    │       └── lib.rs            # ironbase, Collection wrappers
    └── csharp/                   # (Planned) C# Bindings
        └── ...
```

### Architecture Layers

```
┌─────────────────────────────────────────────────────┐
│     Language Bindings (Python, C#, etc.)            │
│  - ironbase, Collection wrappers                   │
│  - Language-specific type conversions               │
└──────────────┬──────────────────────────────────────┘
               │ (Foreign Function Interface)
┌──────────────▼──────────────────────────────────────┐
│       ironbase-core (Pure Rust)                    │
│  - DatabaseCore, CollectionCore                     │
│  - CRUD operations                                  │
│  - Query engine with MongoDB operators             │
│  - Document model & serialization                  │
└──────────────┬──────────────────────────────────────┘
               │
┌──────────────▼──────────────────────────────────────┐
│     Storage Engine                                  │
│  - Append-only file storage                        │
│  - Tombstone pattern for deletes                   │
│  - HashMap-based version tracking                  │
│  - Metadata persistence                            │
└─────────────────────────────────────────────────────┘
```

## Implementation Status

### ✅ Completed Features (137 tests passing)

**CRUD Operations:**
- [x] `insert_one()` - Insert single document
- [x] `insert_many()` - Insert multiple documents
- [x] `find()` - Query documents with options
- [x] `find_one()` - Find single document
- [x] `update_one()` - Update single document
- [x] `update_many()` - Update multiple documents
- [x] `delete_one()` - Delete single document
- [x] `delete_many()` - Delete multiple documents

**Query Operations:**
- [x] `count_documents()` - Count with filters
- [x] `distinct()` - Get unique values from field

**Find Options:**
- [x] `projection` - Field filtering (include/exclude mode)
- [x] `sort` - Single and multi-field sorting
- [x] `limit` - Maximum results count
- [x] `skip` - Skip documents (pagination support)

**Aggregation Pipeline:**
- [x] `aggregate()` - Execute aggregation pipelines
- [x] Pipeline stages: `$match`, `$group`, `$project`, `$sort`, `$limit`, `$skip`
- [x] Accumulators: `$sum`, `$avg`, `$min`, `$max`, `$first`, `$last`
- [x] Group by field or null (all documents)
- [x] Multi-stage pipelines with automatic data flow

**Indexing:**
- [x] `create_index()` - Create B+ tree indexes (unique/non-unique)
- [x] `list_indexes()` - List all indexes
- [x] `drop_index()` - Remove index
- [x] `explain()` - Query execution plan analysis
- [x] `find_with_hint()` - Manual index selection
- [x] Automatic query optimization with index selection
- [x] Range scans with B+ tree traversal
- [x] Equality lookups with O(log n) performance

**Query Operators:**
- [x] Comparison: `$eq`, `$ne`, `$gt`, `$gte`, `$lt`, `$lte`, `$in`, `$nin`
- [x] Logical: `$and`, `$or`, `$not`, `$nor`
- [x] Update: `$set`, `$inc`, `$unset`

**Architecture:**
- [x] Cargo workspace with clean separation
- [x] Pure Rust core library (ironbase-core)
- [x] Python bindings via PyO3 (bindings/python)
- [x] Append-only storage with compaction
- [x] Tombstone pattern for deletes
- [x] HashMap-based version tracking
- [x] Auto-generated IDs
- [x] Metadata persistence with iterative convergence
- [x] B+ tree implementation for indexing

**Testing:**
- [x] 111 passing tests (0 failures)
- [x] Storage tests (creation, persistence, compaction)
- [x] Query tests (comparison, logical operators)
- [x] Document tests (serialization, field operations)
- [x] Aggregation tests (pipeline stages, accumulators)
- [x] Find options tests (projection, sort, limit, skip)
- [x] Index tests (B+ tree, explain, hint, performance)
- [x] **ACD Transaction tests** (commit, rollback, crash recovery, WAL)
- [x] Property-based tests (proptest)
- [x] Integration tests (multi-collection scenarios)

### 🚧 Planned Features

**Near-term:**
- [ ] C# bindings (bindings/csharp)
- [ ] JavaScript/Node.js bindings (napi-rs)
- [ ] More aggregation operators (expression operators, array operators)
- [x] More update operators (`$push`, `$pull`, `$addToSet`, `$pop`) ✅
- [x] Compound indexes (multi-field) ✅
- [x] Cursor/streaming API for large result sets ✅
- [x] In-memory storage for fast testing ✅
- [ ] Nested field access in projection/sort (`"user.name"`)

**Medium-term:**
- [x] **ACD Transactions** - Atomicity, Consistency, Durability with WAL ✅ **IMPLEMENTED**
  - Multi-operation atomic commits via begin/commit/rollback API
  - Write-Ahead Log (WAL) for crash recovery with automatic replay
  - JSON-based WAL serialization for compatibility
  - 9-step atomic commit protocol with fsync guarantees
  - Crash recovery tests with automatic WAL cleanup
  - Transaction state machine (Active/Committed/Aborted)
  - ~1,500 LOC implementation (transaction.rs, wal.rs, storage integration, database API, tests)
  - See [IMPLEMENTATION_ACD.md](IMPLEMENTATION_ACD.md) and [INDEX_CONSISTENCY.md](INDEX_CONSISTENCY.md)
- [ ] Text search indexes (full-text search)
- [ ] Geospatial indexes and queries
- [ ] Advanced query optimizer (cost-based)
- [ ] Bulk operations API
- [ ] Benchmark suite (criterion)

**Long-term:**
- [ ] Full ACID (add Isolation to ACD) - MVCC, snapshot isolation
- [ ] MVCC
- [ ] Network protocol (optional)

## 🔍 Példák

Lásd az `example.py` fájlt részletes példákért.

## 🧪 Tesztelés

```bash
# Core library tests (56 unit + 11 integration tests)
cargo test --manifest-path ironbase-core/Cargo.toml

# Python bindings smoke test
cd bindings/python && maturin develop && python -c "import ironbase; print('OK')"

# Run all workspace tests
cargo test --workspace

# Benchmark (when criterion is re-enabled)
cargo bench
```

## 🚀 Teljesítmény

Célok az MVP-hez:
- **1 MB adatbázis**: <10ms olvasás
- **10,000 dokumentum**: <100ms keresés
- **Index nélkül**: Lineáris keresés O(n)
- **Index-szel**: 2-5x gyorsítás

## 🤝 Hozzájárulás

A projekt nyílt forráskódú és várja a hozzájárulásokat!

1. Fork-old a projektet
2. Hozz létre egy feature branch-et (`git checkout -b feature/amazing`)
3. Commit-old a változásokat (`git commit -m 'Add amazing feature'`)
4. Push-old a branch-et (`git push origin feature/amazing`)
5. Nyiss egy Pull Request-et

## 📝 Licensz

MIT License - lásd a LICENSE fájlt

## 🙏 Köszönet

- SQLite inspiráció az egyszerűségért
- MongoDB inspiráció az API-ért
- Rust közösség a fantasztikus eszközökért

## 📧 Kapcsolat

- GitHub Issues: [github.com/yourusername/ironbase/issues](https://github.com/yourusername/ironbase/issues)
- Email: your.email@example.com

---

**ironbase** - When you need MongoDB simplicity with SQLite's elegance ⚡
