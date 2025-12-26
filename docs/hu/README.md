# IronBase

**Nagysebességű beágyazott NoSQL dokumentum-adatbázis** MongoDB-kompatibilis API-val.

Rust-ban írva, Python és C# kötésekkel. Egyfájlos, zero-konfig, szerver nélküli.

[English version](../../README.md)

## Tartalomjegyzék

- [Funkciók](#funkciók)
- [Gyors kezdés](#gyors-kezdés)
- [Telepítés](#telepítés)
- [Lekérdezési operátorok](#lekérdezési-operátorok)
- [Frissítési operátorok](#frissítési-operátorok)
- [Aggregáció](#aggregáció)
- [Indexelés](#indexelés)
- [Tartósság](#tartósság)
- [Tranzakciók](#tranzakciók)

## Funkciók

| Kategória | Funkciók |
|-----------|----------|
| **Core** | MongoDB-kompatibilis API, egyfájlos tárolás, zero-konfig, beágyazott |
| **Lekérdezés** | 21 operátor: összehasonlítás, logikai, elem, tömb, regex, fuzzy |
| **Frissítés** | 7 operátor: `$set`, `$inc`, `$unset`, `$push`, `$pull`, `$addToSet`, `$pop` |
| **Aggregáció** | 6 stage + 6 akkumulátor, dot notation támogatás |
| **Indexelés** | B+ fa indexek, összetett indexek, fuzzy indexek, explain(), hint() |
| **Tartósság** | ACD tranzakciók, WAL, crash recovery, 3 tartóssági mód |
| **Teljesítmény** | ~1M+ insert/sec, O(log n) index keresés |
| **Nyelvek** | Rust, Python (PyO3), C# (.NET 8) |
| **Tesztelés** | 744+ teszt, property-based tesztelés, fuzz tesztelés |

## Gyors kezdés

### Python
```bash
pip install ironbase
```

```python
from ironbase import IronBase

# Adatbázis megnyitása (létrehozza ha nem létezik)
db = IronBase("myapp.mlite")
users = db.collection("users")

# Beszúrás
users.insert_one({"name": "Alice", "age": 30, "city": "Budapest"})
users.insert_many([
    {"name": "Bob", "age": 25, "city": "Szeged"},
    {"name": "Carol", "age": 35, "city": "Budapest"}
])

# Lekérdezés operátorokkal
adults = users.find({"age": {"$gte": 18}})
budapest_users = users.find({"city": "Budapest", "age": {"$lt": 40}})

# Lekérdezés opciókkal
results = users.find(
    {"city": "Budapest"},
    projection={"name": 1, "age": 1, "_id": 0},
    sort=[("age", -1)],
    limit=10
)

# Fuzzy keresés (v1.0.5 újdonság)
similar = users.find({"name": {"$fuzzy": "alic"}})  # Megtalálja "Alice"-t

# Aggregáció
stats = users.aggregate([
    {"$match": {"age": {"$gte": 18}}},
    {"$group": {"_id": "$city", "count": {"$sum": 1}, "avgAge": {"$avg": "$age"}}},
    {"$sort": {"count": -1}}
])

# Indexelés
users.create_index("age")
users.create_compound_index(["city", "age"])
users.create_fuzzy_index("name")  # Fuzzy index (v1.0.5)

db.close()
```

### C# (.NET)
```csharp
using IronBase;

var client = new IronBaseClient("myapp.mlite");
var users = client.GetCollection<User>("users");

// Beszúrás
users.InsertOne(new User { Name = "Alice", Age = 30 });

// Lekérdezés
var adults = users.Find(Builders<User>.Filter.Gte("Age", 18));

// Frissítés
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

## Telepítés

### Python (PyPI)
```bash
pip install ironbase
```

Támogatott: Linux (x86_64, aarch64), Windows (x64), macOS (Intel, Apple Silicon)

### C# (NuGet)
```bash
dotnet add package IronBase
```

### Rust (Forrásból)
```bash
git clone https://github.com/petitan/IronBase.git
cd IronBase
cargo build --release -p ironbase-core
```

## Lekérdezési operátorok

### Összehasonlítás
| Operátor | Leírás | Példa |
|----------|--------|-------|
| `$eq` | Egyenlő | `{"age": {"$eq": 25}}` vagy `{"age": 25}` |
| `$ne` | Nem egyenlő | `{"status": {"$ne": "inactive"}}` |
| `$gt` | Nagyobb mint | `{"age": {"$gt": 18}}` |
| `$gte` | Nagyobb vagy egyenlő | `{"score": {"$gte": 90}}` |
| `$lt` | Kisebb mint | `{"price": {"$lt": 100}}` |
| `$lte` | Kisebb vagy egyenlő | `{"count": {"$lte": 10}}` |
| `$in` | Benne van | `{"city": {"$in": ["Budapest", "Szeged"]}}` |
| `$nin` | Nincs benne | `{"status": {"$nin": ["deleted", "banned"]}}` |

### Logikai
| Operátor | Leírás | Példa |
|----------|--------|-------|
| `$and` | Logikai ÉS | `{"$and": [{"age": {"$gte": 18}}, {"city": "Budapest"}]}` |
| `$or` | Logikai VAGY | `{"$or": [{"city": "Budapest"}, {"city": "Szeged"}]}` |
| `$not` | Logikai NEM | `{"age": {"$not": {"$gt": 30}}}` |
| `$nor` | Logikai NOR | `{"$nor": [{"deleted": true}, {"banned": true}]}` |

### Elem
| Operátor | Leírás | Példa |
|----------|--------|-------|
| `$exists` | Mező létezik | `{"email": {"$exists": true}}` |
| `$type` | Típus ellenőrzés | `{"age": {"$type": "number"}}` |

### Tömb
| Operátor | Leírás | Példa |
|----------|--------|-------|
| `$all` | Tartalmazza mindet | `{"tags": {"$all": ["a", "b"]}}` |
| `$elemMatch` | Elem egyezik | `{"scores": {"$elemMatch": {"$gt": 80}}}` |
| `$size` | Tömb hossz | `{"tags": {"$size": 3}}` |

### Szöveg
| Operátor | Leírás | Példa |
|----------|--------|-------|
| `$regex` | Regex egyezés | `{"name": {"$regex": "^A"}}` |

### Fuzzy keresés (v1.0.5)
| Operátor | Leírás | Példa |
|----------|--------|-------|
| `$fuzzy` | Hasonlóság keresés | `{"name": {"$fuzzy": "john"}}` |
| `$fuzzy` | Opciókkal | `{"name": {"$fuzzy": {"value": "john", "threshold": 0.7}}}` |

```python
# Egyszerű fuzzy keresés (alapértelmezett: Jaro-Winkler, küszöb: 0.8)
users.find({"name": {"$fuzzy": "john"}})

# Algoritmus választással
users.find({"name": {"$fuzzy": {
    "value": "john",
    "algorithm": "levenshtein",  # jaro_winkler | levenshtein | damerau_levenshtein
    "threshold": 0.7
}}})

# Algoritmusok:
# - jaro_winkler (alapértelmezett): Leggyorsabb, nevek keresésére
# - levenshtein: Legpontosabb, karakter-szintű távolság
# - damerau_levenshtein: Elgépelésekhez, OCR hibákhoz
```

## Frissítési operátorok

| Operátor | Leírás | Példa |
|----------|--------|-------|
| `$set` | Mező beállítása | `{"$set": {"name": "Bob", "age": 30}}` |
| `$inc` | Szám növelése | `{"$inc": {"score": 10, "attempts": 1}}` |
| `$unset` | Mező eltávolítása | `{"$unset": {"temp_field": ""}}` |
| `$push` | Hozzáadás tömbhöz | `{"$push": {"tags": "new_tag"}}` |
| `$pull` | Eltávolítás tömbből | `{"$pull": {"tags": "old_tag"}}` |
| `$addToSet` | Egyedi elem hozzáadása | `{"$addToSet": {"tags": "unique_tag"}}` |
| `$pop` | Első/utolsó eltávolítása | `{"$pop": {"queue": 1}}` (utolsó) |

## Aggregáció

### Stage-ek

| Stage | Leírás |
|-------|--------|
| `$match` | Dokumentumok szűrése (mint find) |
| `$group` | Csoportosítás mező szerint, aggregátumok számítása |
| `$project` | Dokumentumok átformázása (mezők be/ki/átnevezés) |
| `$sort` | Rendezés |
| `$limit` | Eredmények korlátozása |
| `$skip` | Dokumentumok kihagyása |

### Akkumulátorok ($group-ban)

| Akkumulátor | Leírás |
|-------------|--------|
| `$sum` | Összegzés vagy darabszám (`{"$sum": 1}`) |
| `$avg` | Átlag |
| `$min` | Minimum |
| `$max` | Maximum |
| `$first` | Első érték a csoportban |
| `$last` | Utolsó érték a csoportban |

### Példa

```python
# Eladási analitika beágyazott mező támogatással
results = sales.aggregate([
    {"$match": {"status": "completed"}},
    {"$group": {
        "_id": "$store.location.city",
        "totalRevenue": {"$sum": "$payment.amount"},
        "orderCount": {"$sum": 1},
        "avgOrder": {"$avg": "$payment.amount"}
    }},
    {"$sort": {"totalRevenue": -1}},
    {"$limit": 10}
])
```

## Indexelés

```python
# Indexek létrehozása
users.create_index("email", unique=True)
users.create_index("age")
users.create_compound_index(["country", "city"])

# Fuzzy index (v1.0.5)
users.create_fuzzy_index("name")
users.create_fuzzy_index("email", algorithm="levenshtein", threshold=0.7)

# Indexek listázása
print(users.list_indexes())

# Lekérdezési terv elemzése
plan = users.explain({"age": {"$gte": 25}})
print(plan["queryPlan"])   # "IndexRangeScan"
print(plan["indexUsed"])   # "users_age"

# Index kényszerítése
results = users.find_with_hint({"age": 25}, "users_age")

# Index törlése
users.drop_index("users_age")
```

### Index típusok

| Típus | Leírás | Használat |
|-------|--------|-----------|
| **Egyszerű** | B+ fa egy mezőn | Egyenlőség, tartomány lekérdezések |
| **Összetett** | B+ fa több mezőn | Többmezős lekérdezések |
| **Egyedi** | Egyediség kényszer | Email, felhasználónév |
| **Fuzzy** | Szöveg hasonlóság index | Névkeresés, elgépelés tolerancia |

## Tartósság

### Safe mód (Alapértelmezett)
```python
db = IronBase("app.mlite")
```
- **NULLA adatvesztés** - Minden művelet azonnal mentve
- ~200 ops/sec (fsync-kel)
- Használat: Pénzügyi adatok, felhasználói fiókok

### Batch mód
```python
db = IronBase("app.mlite", durability="batch", batch_size=100)
```
- **Korlátozott vesztés** - Max `batch_size` művelet veszhet el
- ~500 ops/sec
- Használat: Logok, analitika, session tracking

### Unsafe mód
```python
db = IronBase("app.mlite", durability="unsafe")
db.checkpoint()  # Manuális mentés szükséges!
```
- **Manuális kontroll** - Nagy adatvesztés kockázat checkpoint nélkül
- ~500 ops/sec
- Használat: Ideiglenes adatok, tömeges import

## Tranzakciók (ACD)

```python
# Tranzakció indítása
tx_id = db.begin_transaction()

try:
    db.insert_one_tx("accounts", {"id": 1, "balance": 1000}, tx_id)
    db.update_one_tx("accounts", {"id": 2}, {"balance": 500}, tx_id)

    # Atomi commit
    db.commit_transaction(tx_id)
except:
    # Rollback hiba esetén
    db.rollback_transaction(tx_id)
    raise
```

## Lapozás include_total-lal (v1.0.5)

```python
# Lapozás összes találat számával
result = users.find({}, limit=10, skip=0, include_total=True)
print(f"{len(result['documents'])} / {result['total']} találat")
# Eredmény: {"documents": [...], "total": 100}
```

## Dokumentáció

- [Főoldal (English)](../../README.md)
- [Build útmutató](../../BUILD.md)
- [Aggregáció](../../AGGREGATION.md)
- [Indexelés](../../INDEXES.md)
- [C# dokumentáció](../../IronBase.NET/README.md)

## Licenc

MIT Licenc
