# MongoLite - MVP Követelmények és Architektúra

## 🎯 Projekt Áttekintés

**MongoLite** = SQLite a NoSQL világában

Ahogy az SQLite egyszerűsítette a relációs adatbázisokat beágyazható formára,
úgy a MongoLite egyszerűsíti a MongoDB-t egy könnyűsúlyú, beágyazható
dokumentum-adatbázissá.

### Analógia
```
MySQL/PostgreSQL  →  SQLite
      ↓                 ↓
    MongoDB      →  MongoLite
```

## 🏗️ Technikai Stack

### Backend (Rust)
- **Nyelv**: Rust 1.70+
- **Binding**: PyO3 0.20 (Python interfész)
- **Szerializáció**: serde_json, BSON
- **I/O**: memmap2 (memory-mapped fájlok)
- **Párhuzamosság**: parking_lot, crossbeam

### Frontend (Python API)
- **Nyelv**: Python 3.8+
- **Build**: Maturin
- **API**: MongoDB-kompatibilis

## 📁 Fájl Struktúra

```
ironbase_project/
├── Cargo.toml              # Rust dependencies
├── pyproject.toml          # Python package config
├── README.md               # Dokumentáció
├── example.py              # Példa használat
├── src/
│   ├── lib.rs             # Fő könyvtár, Python binding
│   ├── storage.rs         # Storage engine (fájl I/O)
│   ├── collection.rs      # Collection műveletek
│   ├── document.rs        # Dokumentum struktúra
│   ├── query.rs           # Query engine
│   ├── index.rs           # Index kezelés
│   └── error.rs           # Hibakezelés
└── tests/
    └── (később)
```

## 💾 Fájl Formátum

### Adatbázis fájl (.mlite)
```
┌─────────────────────────────────────┐
│         Header (128 bytes)          │
│  - Magic: "MONGOLTE" (8 bytes)      │
│  - Version: u32                     │
│  - Page size: u32                   │
│  - Collection count: u32            │
│  - Free list head: u64              │
├─────────────────────────────────────┤
│    Collection Metadata (változó)    │
│  - Collection name                  │
│  - Document count                   │
│  - Data offset                      │
│  - Index offset                     │
│  - Last ID                          │
├─────────────────────────────────────┤
│         Document Data               │
│  [Length: u32][JSON bytes]          │
│  [Length: u32][JSON bytes]          │
│  ...                                │
├─────────────────────────────────────┤
│         Index Data                  │
│  (B-tree struktúrák)                │
└─────────────────────────────────────┘
```

### Fájlméret
- **Minimum**: ~1 KB (üres adatbázis)
- **Maximum**: OS limit (Linux: 16 EB, Windows: 16 EB)
- **Ajánlott**: < 10 GB (optimális teljesítmény)

## 🚀 MVP Követelmények

### Phase 1: Core Storage (✅ KÉSZ)
- [x] Fájl-alapú tárolás
- [x] Header management
- [x] Collection metadata
- [x] Append-only write
- [x] Basic read

### Phase 2: CRUD Operations (🚧 FOLYAMATBAN)
- [x] insert_one()
- [x] insert_many()
- [x] count_documents()
- [ ] find_one() - egyszerű query
- [ ] find() - összes dokumentum
- [ ] update_one()
- [ ] delete_one()

### Phase 3: Query Engine (📋 TERVEZETT)
- [ ] $eq, $ne operátorok
- [ ] $gt, $gte, $lt, $lte operátorok
- [ ] $in, $nin operátorok
- [ ] $exists operátor
- [ ] $and, $or logikai operátorok

### Phase 4: Indexing (📋 TERVEZETT)
- [ ] Automatikus _id index
- [ ] create_index() - egyszerű mezőre
- [ ] Unique index támogatás
- [ ] Index-alapú keresés

### Phase 5: Optimization (📋 KÉSŐBBI)
- [ ] Memory-mapped I/O optimalizálás
- [ ] Query optimizer
- [ ] Compression
- [ ] Compaction (garbage collection)

## 🎯 Teljesítmény Célok

### MVP Szint
| Művelet | Cél | Megjegyzés |
|---------|-----|------------|
| insert_one | < 1ms | SSD-n |
| find (scan) | 1000 doc/ms | Index nélkül |
| find (index) | < 5ms | Index-szel |
| update_one | < 2ms | |
| delete_one | < 2ms | |

### Optimalizált Szint (későbbi)
| Művelet | Cél | Megjegyzés |
|---------|-----|------------|
| insert_one | < 100µs | Batch insert |
| find (index) | < 1ms | B-tree index |
| Throughput | 10K ops/sec | Egyszerű műveletek |

## 🧪 Tesztelési Stratégia

### Unit Tesztek (Rust)
```rust
#[test]
fn test_insert_and_read() {
    let db = StorageEngine::open("test.mlite").unwrap();
    // ...
}
```

### Integration Tesztek (Python)
```python
def test_full_crud_cycle():
    db = MongoLite("test.mlite")
    users = db.collection("users")
    # INSERT
    result = users.insert_one({"name": "Test"})
    # READ
    doc = users.find_one({"_id": result["inserted_id"]})
    # UPDATE
    users.update_one({"_id": doc["_id"]}, {"$set": {"name": "Updated"}})
    # DELETE
    users.delete_one({"_id": doc["_id"]})
```

### Benchmark
```bash
cargo bench
```

## 🔄 Build és Deploy

### Development Build
```bash
# Rust build + Python install
maturin develop

# Példa futtatása
python example.py
```

### Release Build
```bash
# Optimalizált build
maturin build --release

# Wheel létrehozása
ls target/wheels/
```

### PyPI Publikálás (később)
```bash
maturin publish
```

## 📊 Use Case-ek

### 1. Desktop Alkalmazás
```python
# Config tárolás
db = MongoLite("~/.myapp/config.mlite")
settings = db.collection("settings")
settings.insert_one({"theme": "dark", "language": "hu"})
```

### 2. IoT Device
```python
# Senzor adatok
db = MongoLite("/data/sensors.mlite")
readings = db.collection("temperature")
readings.insert_one({
    "sensor_id": "temp_01",
    "value": 23.5,
    "timestamp": datetime.now()
})
```

### 3. Mobile Backend (SQLite alternatíva)
```python
# Offline-first app
db = MongoLite("app_data.mlite")
todos = db.collection("todos")
todos.insert_one({
    "title": "Buy milk",
    "completed": False,
    "due_date": "2025-11-10"
})
```

## 🔐 Biztonság

### MVP Szint
- Nincs authentication
- Nincs encryption
- Fájl-szintű jogosultságok (OS)

### Későbbi
- Optional encryption at rest
- Password protected databases
- User permissions

## 🎓 Learning Resources

### MongoDB Query Syntax
- https://docs.mongodb.com/manual/tutorial/query-documents/

### Rust + Python (PyO3)
- https://pyo3.rs/

### Database Design
- SQLite internals: https://www.sqlite.org/arch.html
- B-tree: https://en.wikipedia.org/wiki/B-tree

## 🚧 Ismert Limitációk (MVP)

1. **Nincs transaction** - Csak atomi írások
2. **Nincs cursor** - Minden eredmény memóriában
3. **Nincs aggregation** - Csak egyszerű query-k
4. **Nincs replication** - Single file only
5. **Nincs sharding** - Egy fájl, egy gép

## 📈 Jövőbeli Fejlesztések

### v0.2.0 - Query Optimization
- [ ] Query planner
- [ ] Statistics-based optimization
- [ ] Covering indexes

### v0.3.0 - Advanced Features
- [ ] Aggregation pipeline
- [ ] Text search
- [ ] Geospatial queries

### v1.0.0 - Production Ready
- [ ] Transactions (ACID)
- [ ] Backup/Restore
- [ ] Migration tools
- [ ] Performance tuning guide

## 📝 Changelog

### v0.1.0 (Current - MVP)
- Initial release
- Basic CRUD operations
- Python binding
- File-based storage

---

**Status**: 🚧 MVP fejlesztés alatt
**Next Milestone**: Query engine implementálás
**Estimated completion**: 2-3 hónap (hobby projekt)
