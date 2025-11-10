# 🚀 MongoLite Projekt - Összefoglaló

## Mi az a MongoLite?

**MongoLite** egy beágyazható, fájl-alapú NoSQL dokumentum-adatbázis, amely a MongoDB egyszerűségét kombinálja az SQLite könnyűsúlyával.

### Analógia
```
SQL Szerverek (MySQL, PostgreSQL)  →  SQLite (egyszerű, beágyazott)
                ↓                            ↓
    MongoDB (NoSQL szerver)        →  MongoLite (egyszerű, beágyazott)
```

## 🎯 Miért MongoLite?

| Tulajdonság | MongoDB | MongoLite |
|-------------|---------|-----------|
| Telepítés | Komplex szerver setup | Zero-config |
| Méret | ~500 MB+ | ~2-3 MB |
| Fájl | Több fájl + log | Egyetlen .mlite fájl |
| Hálózat | Port, security | Helyi fájl |
| Use case | Nagy, skálázható projektek | Desktop, mobil, IoT, MVP |

## 📁 Projekt Struktúra

```
ironbase_project/
├── README.md              # Főoldali dokumentáció
├── ARCHITECTURE.md        # Részletes architektúra
├── BUILD.md              # Build útmutató
├── LICENSE               # MIT License
├── .gitignore            # Git ignore szabályok
│
├── Cargo.toml            # Rust dependencies
├── pyproject.toml        # Python package config
├── example.py            # Példa használat
│
└── src/                  # Rust forráskód
    ├── lib.rs           # Fő könyvtár + Python binding
    ├── storage.rs       # Fájl I/O és storage engine
    ├── collection.rs    # Collection műveletek (CRUD)
    ├── document.rs      # Dokumentum struktúra
    ├── query.rs         # Query engine (MongoDB operátorok)
    ├── index.rs         # Indexelés (B-tree)
    └── error.rs         # Hibakezelés
```

## 🏗️ Technológiai Stack

### Backend (Rust)
- **Teljesítmény**: Natív, memory-safe kód
- **Binding**: PyO3 (Python-Rust híd)
- **I/O**: Memory-mapped fájlok (memmap2)
- **Szerializáció**: JSON (serde_json) + BSON

### Frontend (Python API)
- **API**: MongoDB-kompatibilis szintaxis
- **Build**: Maturin (Rust → Python wheel)
- **Kompatibilitás**: Python 3.8+

## ⚡ Gyors Kezdés

### 1. Build és Telepítés
```bash
# Előfeltételek
pip install maturin

# Build
cd ironbase_project
maturin develop

# Teszt
python example.py
```

### 2. Használat Python-ból
```python
from mongolite import MongoLite

# Adatbázis
db = MongoLite("myapp.mlite")

# Collection
users = db.collection("users")

# CRUD
users.insert_one({"name": "János", "age": 30})
users.insert_many([
    {"name": "Anna", "age": 25},
    {"name": "Péter", "age": 35}
])

print(f"Összes felhasználó: {users.count_documents()}")

db.close()
```

## 📊 MVP Státusz (v0.1.0)

### ✅ Implementált Funkciók
- [x] Adatbázis létrehozás/megnyitás
- [x] Collection kezelés
- [x] `insert_one()` - Egy dokumentum beszúrása
- [x] `insert_many()` - Több dokumentum beszúrása
- [x] `count_documents()` - Számolás
- [x] Automatikus ID generálás
- [x] Fájl-alapú perzisztens tárolás
- [x] Python API (PyO3)

### 🚧 Fejlesztés Alatt
- [ ] `find()` / `find_one()` - Keresés
- [ ] Query operátorok ($gt, $lt, $in, $eq, stb.)
- [ ] `update_one()` / `update_many()` - Frissítés
- [ ] `delete_one()` / `delete_many()` - Törlés
- [ ] Indexelés (B-tree alapú)

### 📋 Tervezett (v0.2+)
- [ ] Aggregation pipeline
- [ ] Tranzakciók
- [ ] Full-text search
- [ ] Compression
- [ ] Backup/Restore

## 🎯 Use Case-ek

### 1. Desktop Alkalmazás
```python
# Config fájl helyettesítése
db = MongoLite("~/.myapp/settings.mlite")
config = db.collection("settings")
config.insert_one({"theme": "dark", "language": "hu"})
```

### 2. Mobil App Backend
```python
# Offline-first architektúra
db = MongoLite("/data/app.mlite")
todos = db.collection("todos")
todos.insert_one({
    "title": "Teendő",
    "completed": False,
    "sync_status": "pending"
})
```

### 3. IoT Device
```python
# Senzor adatok lokális tárolása
db = MongoLite("/var/sensors.mlite")
readings = db.collection("temperature")
readings.insert_one({
    "sensor_id": "temp_01",
    "value": 23.5,
    "timestamp": datetime.now()
})
```

### 4. Prototípus/MVP
```python
# Gyors prototípus MongoDB migráció nélkül
db = MongoLite("prototype.mlite")
# ... ugyanaz az API, mint MongoDB
# Később: átmigráció MongoDB-re
```

## 📈 Teljesítmény Célok

| Művelet | MVP Cél | Optimalizált (v1.0) |
|---------|---------|---------------------|
| insert_one | < 1ms | < 100µs |
| find (scan) | 1000 doc/ms | - |
| find (index) | < 5ms | < 1ms |
| Fájlméret | Korlátlan | OS limit (16 EB) |

## 🔄 Összehasonlítás

### MongoLite vs MongoDB
```
MongoLite:
+ Egyszerű telepítés (zero-config)
+ Kis méret (~2 MB)
+ Egyetlen fájl
+ Nincs szükség szerverre
- Nincs replikáció
- Nincs sharding
- Egy gépen fut

MongoDB:
+ Skálázható (clusters)
+ Replikáció
+ Sharding
+ Production-ready
- Komplex setup
- Nagy méret
- Szerver szükséges
```

### MongoLite vs SQLite + JSON
```
MongoLite:
+ MongoDB-kompatibilis API
+ Dokumentum-orientált
+ Beépített query operátorok
+ Indexelés dokumentumokhoz

SQLite + JSON:
+ SQL nyelv
+ ACID tranzakciók
- Nehézkesebb JSON kezelés
- Nem natív dokumentum-orientált
```

## 🛠️ Fejlesztői Információk

### Build Követelmények
- Rust 1.70+
- Python 3.8+
- Maturin build system

### Architektúra Rétegek
```
Python API (PyO3)
      ↓
Rust Core (CRUD + Query Engine)
      ↓
Storage Engine (Memory-mapped I/O)
      ↓
.mlite fájl (Append-only log + metadata)
```

### Fájl Formátum (.mlite)
```
[Header 128B] → [Collection Meta változó] → [Documents] → [Indexes]
```

## 📚 Dokumentáció

- **README.md** - Főoldal, gyors kezdés
- **ARCHITECTURE.md** - Részletes architektúra, MVP követelmények
- **BUILD.md** - Build és telepítési útmutató
- **example.py** - Kód példák

## 🤝 Hozzájárulás

A projekt nyílt forráskódú (MIT License).

```bash
# Fork + Clone
git clone https://github.com/yourusername/mongolite.git
cd mongolite

# Feature branch
git checkout -b feature/my-feature

# Commit + Push
git commit -m "Add amazing feature"
git push origin feature/my-feature

# Pull Request
```

## 🎓 Tanulási Források

### MongoDB
- Query syntax: https://docs.mongodb.com/manual/tutorial/query-documents/
- CRUD operations: https://docs.mongodb.com/manual/crud/

### Rust + Python
- PyO3: https://pyo3.rs/
- Maturin: https://www.maturin.rs/

### Database Design
- SQLite Architecture: https://www.sqlite.org/arch.html
- B-tree indexes: https://en.wikipedia.org/wiki/B-tree

## 📞 Kapcsolat

- **GitHub**: github.com/yourusername/mongolite
- **Issues**: Hibabejelentés és feature request-ek
- **Email**: your.email@example.com

## 🗺️ Roadmap

### v0.1.0 (Current - MVP) ✅
- Alapvető CRUD műveletek
- Python binding
- Fájl-alapú tárolás

### v0.2.0 (1-2 hónap)
- Teljes query engine
- Update/Delete műveletek
- Egyszerű indexelés

### v0.3.0 (2-3 hónap)
- Optimalizált tárolás
- Aggregation kezdetek
- Teljesítmény tuning

### v1.0.0 (6 hónap)
- Production-ready
- ACID tranzakciók
- Comprehensive docs
- Benchmark suite

## ⭐ Miért érdekes ez a projekt?

1. **Tanulási lehetőség**: Rust + Python + Database internals
2. **Hasznos eszköz**: Valódi problémát old meg
3. **Nyílt forráskód**: Közösségi fejlesztés
4. **Modern tech stack**: Rust teljesítmény + Python egyszerűség
5. **Piaci rés**: Nincs széles körben használt MongoDB-lite alternatíva

---

## 🚀 Következő Lépések

```bash
# 1. Projekt klónozása
git clone <repo-url>

# 2. Build
cd ironbase_project
maturin develop

# 3. Példa futtatása
python example.py

# 4. Dokumentáció olvasása
cat README.md
cat ARCHITECTURE.md

# 5. Fejlesztés indítása!
```

---

**MongoLite** - When you need MongoDB simplicity with SQLite's elegance ⚡

*Projekt státusz: 🚧 MVP fejlesztés (v0.1.0)*
*Verzió: 0.1.0-alpha*
*Utolsó frissítés: 2025-11-09*
