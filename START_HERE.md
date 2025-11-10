# 🚀 MongoLite - Teljes Projekt

## Mi ez?

**MongoLite** egy MongoDB-szerű, beágyazható NoSQL dokumentum-adatbázis Rust-ban írva, Python API-val.

## 📦 Mit kaptál?

Egy **komplett, működőképes projektet**:

```
ironbase_project/
├── 📘 Dokumentáció (5 fájl)
│   ├── README.md - Főoldal, API dokumentáció
│   ├── ARCHITECTURE.md - Részletes architektúra
│   ├── BUILD.md - Build útmutató
│   ├── PROJECT_OVERVIEW.md - Projekt áttekintés
│   └── SUMMARY.md - Teljes összefoglaló
│
├── 💻 Forráskód
│   ├── src/ - 7 Rust fájl (~32 KB)
│   │   ├── lib.rs - Python binding
│   │   ├── storage.rs - Fájl tárolás
│   │   ├── collection.rs - CRUD műveletek
│   │   ├── document.rs - Dokumentum struktúra
│   │   ├── query.rs - Query engine
│   │   ├── index.rs - Indexelés
│   │   └── error.rs - Hibakezelés
│   └── example.py - Python példák
│
└── ⚙️ Konfiguráció
    ├── Cargo.toml - Rust dependencies
    ├── pyproject.toml - Python package
    ├── .gitignore - Git szabályok
    └── LICENSE - MIT License
```

## ⚡ Gyors Kezdés

### 1. Előfeltételek
```bash
# Rust telepítése
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Maturin telepítése
pip install maturin
```

### 2. Build
```bash
cd ironbase_project
maturin develop
```

### 3. Használat
```python
from mongolite import MongoLite

# Adatbázis
db = MongoLite("myapp.mlite")

# Collection
users = db.collection("users")

# Adat beszúrása
users.insert_one({"name": "János", "age": 30})
users.insert_many([
    {"name": "Anna", "age": 25},
    {"name": "Péter", "age": 35}
])

# Számolás
print(f"Felhasználók: {users.count_documents()}")

db.close()
```

### 4. Példa futtatása
```bash
python example.py
```

## 📊 Jelenlegi Állapot

### ✅ Működik (MVP v0.1.0)
- Database létrehozás/megnyitás
- Collection kezelés
- insert_one(), insert_many()
- count_documents()
- Automatikus ID generálás
- Perzisztens fájl tárolás

### 🚧 Fejlesztés alatt
- find() / find_one() keresés
- update_one() / update_many()
- delete_one() / delete_many()
- Query operátorok ($gt, $lt, $in, stb.)
- Indexelés

## 📚 Dokumentáció

1. **README.md** - Kezdd itt! Teljes API dokumentáció
2. **ARCHITECTURE.md** - Részletes architektúra, MVP követelmények
3. **BUILD.md** - Build problémák megoldása
4. **PROJECT_OVERVIEW.md** - Teljes projekt áttekintés
5. **SUMMARY.md** - Gyors összefoglaló

## 🎯 Miért hasznos?

**MongoLite = SQLite (egyszerűség) + MongoDB (API)**

### Use case-ek:
- 📱 Desktop alkalmazások
- 📲 Mobil app backend
- 🤖 IoT eszközök
- 🧪 Prototípusok, MVP-k
- 💾 Embedded adatbázis

### Előnyök:
- ✅ Zero-config (nincs setup)
- ✅ Egyetlen fájl
- ✅ Nincs szükség szerverre
- ✅ MongoDB-kompatibilis API
- ✅ Rust teljesítmény
- ✅ Python egyszerűség

## 🛠️ Troubleshooting

### "maturin: command not found"
```bash
pip install --user maturin
# vagy
pip3 install maturin
```

### "Python.h not found"
```bash
# Ubuntu/Debian
sudo apt install python3-dev

# Fedora
sudo dnf install python3-devel
```

### "linker 'cc' not found"
```bash
# Ubuntu/Debian
sudo apt install build-essential

# macOS
xcode-select --install
```

## 📈 Roadmap

- **v0.1.0** (Most) - Alapvető insert műveletek ✅
- **v0.2.0** (1-2 hónap) - Teljes CRUD + query engine
- **v0.3.0** (2-3 hónap) - Indexelés + optimalizálás
- **v1.0.0** (6 hónap) - Production ready

## 🤝 Hozzájárulás

Projekt nyílt forráskódú (MIT License).

```bash
git clone <your-repo>
cd ironbase_project
git checkout -b feature/my-feature
# ... fejlesztés ...
git push origin feature/my-feature
# Pull Request
```

## 📧 Kapcsolat

- GitHub: github.com/yourusername/mongolite
- Issues: Hibabejelentés és feature request
- Email: your.email@example.com

## 🎓 Tanulási Érték

Ez a projekt remek példa:
- Rust + Python integráció (PyO3)
- Database internals
- Memory-mapped I/O
- API design
- Open source projekt struktúra

## 🌟 Következő Lépések

1. **Olvasd el**: README.md
2. **Build**: `maturin develop`
3. **Tesztelj**: `python example.py`
4. **Fejlessz**: Lásd ARCHITECTURE.md
5. **Dokumentálj**: Frissítsd a docs-ot

---

## 📦 Fájlok Mérete

```
Összesen: ~100 KB

Dokumentáció: ~50 KB
  - README.md: 7.3 KB
  - ARCHITECTURE.md: 7.5 KB
  - BUILD.md: 4.5 KB
  - PROJECT_OVERVIEW.md: 7.9 KB
  - SUMMARY.md: 9.0 KB

Forráskód: ~32 KB
  - collection.rs: 7.6 KB
  - storage.rs: 8.4 KB
  - query.rs: 6.7 KB
  - index.rs: 3.5 KB
  - lib.rs: 2.5 KB
  - document.rs: 2.3 KB
  - error.rs: 0.9 KB

Példák: ~5 KB
  - example.py: 4.5 KB

Konfiguráció: ~3 KB
```

## 🎯 Tech Stack

- **Backend**: Rust 1.70+ (teljesítmény)
- **Binding**: PyO3 0.20 (Rust→Python)
- **API**: Python 3.8+ (egyszerűség)
- **Build**: Maturin (wheel építés)
- **I/O**: memmap2 (memory-mapped files)
- **Serialization**: serde_json, BSON

---

**MongoLite** - When you need MongoDB simplicity with SQLite's elegance ⚡

*MVP v0.1.0 - Alapvető CRUD insert műveletek működnek!*
*Készítve: 2025-11-09*
*License: MIT*

🚀 **Jó kódolást!** 🚀
