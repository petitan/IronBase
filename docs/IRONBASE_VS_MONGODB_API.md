# IronBase vs MongoDB API Comparison

**Version:** IronBase 1.0.148 vs MongoDB 7.x
**Date:** 2026-01-04

---

## Executive Summary

IronBase egy MongoDB-kompatibilis beágyazott NoSQL dokumentum adatbázis. A legtöbb MongoDB művelet és operátor támogatott, néhány eltéréssel és IronBase-specifikus kiegészítéssel.

| Kategória | MongoDB | IronBase | Kompatibilitás |
|-----------|---------|----------|----------------|
| CRUD műveletek | ✅ | ✅ | 100% |
| Query operátorok | 30+ | 21 | 70% |
| Update operátorok | 15+ | 7 | 47% |
| Aggregation stages | 20+ | 7 | 35% |
| Aggregation accumulators | 15+ | 8 | 53% |
| Index típusok | 10+ | 5 | 50% |
| Tranzakciók | ✅ | ✅ | 90% |

---

## CRUD Műveletek

| Művelet | MongoDB | IronBase | Megjegyzés |
|---------|---------|----------|------------|
| `insertOne()` | ✅ | ✅ `insert_one()` | Azonos |
| `insertMany()` | ✅ | ✅ `insert_many()` | Azonos |
| `find()` | ✅ | ✅ `find()` | Azonos |
| `findOne()` | ✅ | ✅ `find_one()` | Azonos |
| `updateOne()` | ✅ | ✅ `update_one()` | Azonos |
| `updateMany()` | ✅ | ✅ `update_many()` | Azonos |
| `deleteOne()` | ✅ | ✅ `delete_one()` | Azonos |
| `deleteMany()` | ✅ | ✅ `delete_many()` | Azonos |
| `countDocuments()` | ✅ | ✅ `count_documents()` | Azonos |
| `distinct()` | ✅ | ✅ `distinct()` | Azonos |
| `replaceOne()` | ✅ | ❌ | Használj `update_one` + `$set` |
| `bulkWrite()` | ✅ | ❌ | Használj egyedi műveleteket |
| `findAndModify()` | ✅ | ❌ | Nincs atomi find+modify |
| `findOneAndUpdate()` | ✅ | ❌ | Nincs |
| `findOneAndDelete()` | ✅ | ❌ | Nincs |
| `findOneAndReplace()` | ✅ | ❌ | Nincs |

---

## Query Operátorok

### Összehasonlító Operátorok

| Operátor | MongoDB | IronBase | Példa |
|----------|---------|----------|-------|
| `$eq` | ✅ | ✅ | `{"age": {"$eq": 25}}` |
| `$ne` | ✅ | ✅ | `{"status": {"$ne": "inactive"}}` |
| `$gt` | ✅ | ✅ | `{"age": {"$gt": 18}}` |
| `$gte` | ✅ | ✅ | `{"age": {"$gte": 18}}` |
| `$lt` | ✅ | ✅ | `{"age": {"$lt": 65}}` |
| `$lte` | ✅ | ✅ | `{"age": {"$lte": 65}}` |
| `$in` | ✅ | ✅ | `{"status": {"$in": ["a", "b"]}}` |
| `$nin` | ✅ | ✅ | `{"status": {"$nin": ["x", "y"]}}` |

### Logikai Operátorok

| Operátor | MongoDB | IronBase | Példa |
|----------|---------|----------|-------|
| `$and` | ✅ | ✅ | `{"$and": [{...}, {...}]}` |
| `$or` | ✅ | ✅ | `{"$or": [{...}, {...}]}` |
| `$nor` | ✅ | ✅ | `{"$nor": [{...}, {...}]}` |
| `$not` | ✅ | ✅ | `{"age": {"$not": {"$gt": 25}}}` |

### Elem Operátorok

| Operátor | MongoDB | IronBase | Példa |
|----------|---------|----------|-------|
| `$exists` | ✅ | ✅ | `{"email": {"$exists": true}}` |
| `$type` | ✅ | ✅ | `{"age": {"$type": "number"}}` |

### Tömb Operátorok

| Operátor | MongoDB | IronBase | Példa |
|----------|---------|----------|-------|
| `$all` | ✅ | ✅ | `{"tags": {"$all": ["a", "b"]}}` |
| `$elemMatch` | ✅ | ✅ | `{"items": {"$elemMatch": {...}}}` |
| `$size` | ✅ | ✅ | `{"tags": {"$size": 3}}` |

### Szöveg Operátorok

| Operátor | MongoDB | IronBase | Megjegyzés |
|----------|---------|----------|------------|
| `$regex` | ✅ | ✅ | Azonos szintaxis |
| `$text` | ✅ | ❌ | Használj `fulltext_search()` |
| `$fuzzy` | ❌ | ✅ | **IronBase-only!** Fuzzy keresés |
| `$**` | ❌ | ✅ | **IronBase-only!** Rekurzív mező keresés |

### Kifejezés Operátor

| Operátor | MongoDB | IronBase | Példa |
|----------|---------|----------|-------|
| `$expr` | ✅ | ✅ | `{"$expr": {"$gt": ["$a", "$b"]}}` |

### Hiányzó Query Operátorok (MongoDB-ben van, IronBase-ben nincs)

| Operátor | Leírás | Workaround |
|----------|--------|------------|
| `$mod` | Modulo | Aggregation `$mod` |
| `$where` | JavaScript | Nincs |
| `$jsonSchema` | Schema validáció | `set_collection_schema()` |
| `$geoWithin` | Geo query | Nincs |
| `$near` | Geo proximity | Nincs |
| `$text` | Fulltext | `fulltext_search()` |
| `$bitsAllSet` | Bit operátorok | Nincs |

---

## Update Operátorok

| Operátor | MongoDB | IronBase | Példa |
|----------|---------|----------|-------|
| `$set` | ✅ | ✅ | `{"$set": {"name": "Bob"}}` |
| `$inc` | ✅ | ✅ | `{"$inc": {"count": 1}}` |
| `$unset` | ✅ | ✅ | `{"$unset": {"temp": ""}}` |
| `$push` | ✅ | ✅ | `{"$push": {"tags": "new"}}` |
| `$pull` | ✅ | ✅ | `{"$pull": {"tags": "old"}}` |
| `$addToSet` | ✅ | ✅ | `{"$addToSet": {"tags": "unique"}}` |
| `$pop` | ✅ | ✅ | `{"$pop": {"arr": 1}}` |
| `$rename` | ✅ | ❌ | Használj `$unset` + `$set` |
| `$min` | ✅ | ❌ | Nincs |
| `$max` | ✅ | ❌ | Nincs |
| `$mul` | ✅ | ❌ | Nincs |
| `$currentDate` | ✅ | ❌ | Manuálisan adj dátumot |
| `$setOnInsert` | ✅ | ❌ | Nincs upsert |
| `$bit` | ✅ | ❌ | Nincs |

### $push Modifikátorok

| Modifikátor | MongoDB | IronBase |
|-------------|---------|----------|
| `$each` | ✅ | ✅ |
| `$position` | ✅ | ✅ |
| `$slice` | ✅ | ✅ |
| `$sort` | ✅ | ❌ |

---

## Aggregation Pipeline

### Pipeline Stages

| Stage | MongoDB | IronBase | Megjegyzés |
|-------|---------|----------|------------|
| `$match` | ✅ | ✅ | Azonos |
| `$project` | ✅ | ✅ | Azonos |
| `$group` | ✅ | ✅ | Azonos |
| `$sort` | ✅ | ✅ | Azonos |
| `$limit` | ✅ | ✅ | Azonos |
| `$skip` | ✅ | ✅ | Azonos |
| `$unwind` | ✅ | ✅ | Azonos |
| `$lookup` | ✅ | ❌ | Nincs join |
| `$graphLookup` | ✅ | ❌ | Nincs |
| `$facet` | ✅ | ❌ | Nincs |
| `$bucket` | ✅ | ❌ | Nincs |
| `$bucketAuto` | ✅ | ❌ | Nincs |
| `$count` | ✅ | ❌ | Használj `$group` + `$sum` |
| `$out` | ✅ | ❌ | Nincs |
| `$merge` | ✅ | ❌ | Nincs |
| `$replaceRoot` | ✅ | ❌ | Nincs |
| `$addFields` | ✅ | ❌ | Használj `$project` |
| `$set` | ✅ | ❌ | Használj `$project` |
| `$unset` | ✅ | ❌ | Használj `$project` |
| `$sample` | ✅ | ❌ | Nincs |
| `$redact` | ✅ | ❌ | Nincs |
| `$geoNear` | ✅ | ❌ | Nincs geo |

### Aggregation Accumulators

| Accumulator | MongoDB | IronBase | Példa |
|-------------|---------|----------|-------|
| `$sum` | ✅ | ✅ | `{"$sum": "$amount"}` |
| `$avg` | ✅ | ✅ | `{"$avg": "$score"}` |
| `$min` | ✅ | ✅ | `{"$min": "$price"}` |
| `$max` | ✅ | ✅ | `{"$max": "$price"}` |
| `$first` | ✅ | ✅ | `{"$first": "$name"}` |
| `$last` | ✅ | ✅ | `{"$last": "$name"}` |
| `$push` | ✅ | ✅ | `{"$push": "$item"}` |
| `$addToSet` | ✅ | ✅ | `{"$addToSet": "$tag"}` |
| `$count` | ✅ | ❌ | Használj `{"$sum": 1}` |
| `$stdDevPop` | ✅ | ❌ | Nincs |
| `$stdDevSamp` | ✅ | ❌ | Nincs |
| `$mergeObjects` | ✅ | ❌ | Nincs |
| `$accumulator` | ✅ | ❌ | Nincs custom |

---

## Index Típusok

| Index Típus | MongoDB | IronBase | Megjegyzés |
|-------------|---------|----------|------------|
| Single field | ✅ | ✅ `create_index()` | Azonos |
| Compound | ✅ | ✅ `create_compound_index()` | Azonos |
| Unique | ✅ | ✅ `unique: true` | Azonos |
| Sparse | ✅ | ✅ `sparse: true` | Azonos |
| Text | ✅ | ✅ `create_fulltext_index()` | Eltérő API |
| Fuzzy | ❌ | ✅ `create_fuzzy_index()` | **IronBase-only!** |
| Case-insensitive | ✅ (collation) | ✅ `create_ci_index()` | Eltérő API |
| Hashed | ✅ | ❌ | Nincs |
| 2dsphere | ✅ | ❌ | Nincs geo |
| 2d | ✅ | ❌ | Nincs geo |
| Wildcard | ✅ | ❌ | Nincs |
| TTL | ✅ | ❌ | Nincs auto-expire |

---

## Tranzakciók

| Funkció | MongoDB | IronBase | Megjegyzés |
|---------|---------|----------|------------|
| `startSession()` | ✅ | ✅ `begin_transaction()` | Eltérő API |
| `commitTransaction()` | ✅ | ✅ `commit_transaction()` | Azonos |
| `abortTransaction()` | ✅ | ✅ `rollback_transaction()` | Eltérő név |
| Multi-document | ✅ | ✅ | Azonos |
| Read Committed | ✅ | ✅ | Alapértelmezett |
| Snapshot | ✅ | ❌ | Csak Read Committed |
| Distributed | ✅ | ❌ | Single-node only |

---

## IronBase-Specifikus Funkciók

### MongoDB-ben NINCS, IronBase-ben VAN

| Funkció | Leírás | Példa |
|---------|--------|-------|
| **`$fuzzy` operátor** | Fuzzy szövegkeresés Jaro-Winkler, Levenshtein algoritmusokkal | `{"name": {"$fuzzy": {"value": "john", "threshold": 0.8}}}` |
| **`$**` operátor** | Rekurzív mező keresés tetszőleges mélységben | `{"$**.email": "test@example.com"}` |
| **Fuzzy index** | Dedikált fuzzy keresés index | `create_fuzzy_index("name", "jaro_winkler", 0.8)` |
| **`aggregate_auto()`** | Automatikus memória limitelés RAM alapján | Dinamikus skálázás |
| **`find_streaming()`** | Streaming cursor memória-hatékony iterációhoz | Batch-enkénti feldolgozás |
| **Top-K optimalizáció** | `$sort` + `$limit` automatikus heap optimalizáció | O(n log k) vs O(n log n) |
| **Durability módok** | Safe / Batch / Unsafe módok | Teljesítmény vs biztonság trade-off |
| **Single-file storage** | Egyetlen .mlite fájl, nincs külön szerver | Beágyazott használat |
| **Hot backup** | Lock-free backup futó adatbázisról | `ironbase-backup` CLI |

---

## Find Options

| Opció | MongoDB | IronBase | Megjegyzés |
|-------|---------|----------|------------|
| `projection` | ✅ | ✅ | `{field: 1}` vagy `{field: 0}` |
| `sort` | ✅ | ✅ | `[("field", 1)]` (1=ASC, -1=DESC) |
| `limit` | ✅ | ✅ | Azonos |
| `skip` | ✅ | ✅ | Azonos |
| `hint` | ✅ | ✅ | `find_with_hint()` |
| `explain` | ✅ | ✅ | `explain()` metódus |
| `maxTimeMS` | ✅ | ❌ | Nincs query timeout |
| `collation` | ✅ | ❌ | Használj CI indexet |
| `allowDiskUse` | ✅ | ❌ | Mindig memóriában |
| `batchSize` | ✅ | ✅ | `find_streaming().with_batch_size()` |
| `noCursorTimeout` | ✅ | ❌ | Nincs cursor timeout |
| `min` / `max` | ✅ | ❌ | Nincs |
| `returnKey` | ✅ | ❌ | Nincs |
| `showRecordId` | ✅ | ❌ | Nincs |

---

## API Szintaxis Különbségek

### MongoDB (JavaScript)
```javascript
// Insert
db.users.insertOne({name: "Alice", age: 30});

// Find
db.users.find({age: {$gte: 18}}).sort({name: 1}).limit(10);

// Update
db.users.updateOne({name: "Alice"}, {$set: {age: 31}});

// Aggregate
db.users.aggregate([
  {$match: {status: "active"}},
  {$group: {_id: "$department", count: {$sum: 1}}}
]);
```

### IronBase (Rust)
```rust
// Insert
db.insert_one("users", HashMap::from([
    ("name".into(), json!("Alice")),
    ("age".into(), json!(30))
]))?;

// Find
let options = FindOptions::new()
    .with_sort(vec![("name".to_string(), 1)])
    .with_limit(10);
collection.find_with_options(&json!({"age": {"$gte": 18}}), options)?;

// Update
db.update_one("users", &json!({"name": "Alice"}), &json!({"$set": {"age": 31}}))?;

// Aggregate
collection.aggregate(&json!([
  {"$match": {"status": "active"}},
  {"$group": {"_id": "$department", "count": {"$sum": 1}}}
]))?;
```

### IronBase (Python)
```python
# Insert
db.insert_one("users", {"name": "Alice", "age": 30})

# Find
db.find("users", {"age": {"$gte": 18}},
        sort=[("name", 1)], limit=10)

# Update
db.update_one("users", {"name": "Alice"}, {"$set": {"age": 31}})

# Aggregate
db.aggregate("users", [
  {"$match": {"status": "active"}},
  {"$group": {"_id": "$department", "count": {"$sum": 1}}}
])
```

---

## Összefoglaló Táblázat

| Kategória | MongoDB | IronBase | Státusz |
|-----------|---------|----------|---------|
| **CRUD** | ✅ Teljes | ✅ Teljes | ✅ Kompatibilis |
| **Query operátorok** | 30+ | 21 | ⚠️ Részleges |
| **Update operátorok** | 15+ | 7 | ⚠️ Részleges |
| **Aggregation** | 20+ stage | 7 stage | ⚠️ Részleges |
| **Indexek** | 10+ típus | 5 típus | ⚠️ Részleges |
| **Tranzakciók** | ✅ Distributed | ✅ Single-node | ⚠️ Korlátozott |
| **Geo funkciók** | ✅ Teljes | ❌ Nincs | ❌ Nem támogatott |
| **Fulltext** | ✅ `$text` | ✅ `fulltext_search()` | ✅ Eltérő API |
| **Fuzzy search** | ❌ Nincs | ✅ `$fuzzy` | ✅ IronBase extra |
| **Sharding** | ✅ Teljes | ❌ Nincs | ❌ Single-node |
| **Replication** | ✅ Teljes | ❌ Nincs | ❌ Single-node |

---

## Migráció MongoDB-ről IronBase-re

### Támogatott (1:1 migráció)
- Alap CRUD műveletek
- Egyszerű query-k (`$eq`, `$gt`, `$in`, stb.)
- Logikai operátorok (`$and`, `$or`)
- Alap aggregation (`$match`, `$group`, `$sort`, `$limit`)
- Single-field és compound indexek

### Módosítás szükséges
- `$text` → `fulltext_search()` API
- `replaceOne()` → `update_one()` + `$set`
- `findOneAndUpdate()` → külön `find_one()` + `update_one()`
- Collation → Case-insensitive index

### Nem támogatott (újratervezés szükséges)
- Geo query-k
- `$lookup` (join)
- Distributed tranzakciók
- Sharding / Replication
- Change streams
- GridFS

---

*IronBase v1.0.148 | 2026-01-04*
