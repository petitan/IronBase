# IronBase Skálázhatóság és Teljesítmény Elemzés

**Dátum:** 2026. március 9.  
**Verzió:** 0.3.200 (core), 1.0.394 (MCP server)
**Státusz:** ✅ Kód-alapú elemzés (valós architektúra)

---

## 📋 Executive Summary

Az IronBase egy **beágyazott (embedded) NoSQL dokumentum-adatbázis** amely **positioned I/O** (`pread`/`seek_read`) technológiát használ. A korábbi dokumentumokkal ellentétben **NINCS memory-mapped I/O** implementáció.

### Valós Architektúra

| Komponens | Implementáció | Valóság |
|-----------|---------------|---------|
| **Storage I/O** | `pread()` (Unix) / `seek_read()` (Windows) | ✅ Kód-alapú |
| **Memory-Mapped** | NINCS implementáció | ❌ Korábbi dokumentum hamis volt |
| **File Méret Korlát** | NINCS hard limit | ❌ "1GB korlát" nem létezik |
| **memmap2 dependency** | CSAK FastText embedding-ekhez (mcp-server) | ✅ Nem a storage engine-hez |

---

## 🏗️ Valós Architektúra

### Storage Engine I/O

**Forráskód:** `ironbase-core/src/storage/io.rs`

```rust
// Forrás: ironbase-core/src/storage/io.rs (pontos kód, nem pseudokód)

/// Positioned read - read data at offset WITHOUT changing file position
///
/// Uses `pread()` on Unix and `seek_read()` on Windows.
/// This allows concurrent reads because it doesn't modify the file descriptor's position.
///
/// # Thread Safety
/// Safe to call from multiple threads simultaneously.
#[cfg(unix)]
pub fn read_data_at(&self, offset: u64) -> Result<Vec<u8>> {
    use crate::error::IronBaseError;
    use std::os::unix::fs::FileExt;

    // PERF FIX: Use cached header values instead of syscall per read!
    // data_end_offset is updated after document writes (not metadata flush)
    let file_len = self.header.data_end_offset;

    if offset >= file_len {
        return Err(IronBaseError::Corruption(format!(
            "Attempted to read at offset {} but data region ends at {} bytes",
            offset, file_len
        )));
    }

    if offset + 4 > file_len {
        return Err(IronBaseError::Corruption(format!(
            "Insufficient space to read length header at offset {} (data ends: {} bytes)",
            offset, file_len
        )));
    }

    // Read length header using pread (no seek, no position change)
    let mut len_bytes = [0u8; 4];
    self.file.read_at(&mut len_bytes, offset)?;           // ← pread() syscall
    let len = u32::from_le_bytes(len_bytes) as usize;

    // Validate document length
    if len == 0 {
        return Err(IronBaseError::Corruption(format!(
            "Document at offset {} has zero length", offset
        )));
    }
    if len > super::MAX_DOCUMENT_SIZE_BYTES {
        return Err(IronBaseError::Corruption(format!(
            "Document at offset {} exceeds max size: {} bytes (limit: {})",
            offset, len, super::MAX_DOCUMENT_SIZE_BYTES
        )));
    }

    if offset + 4 + (len as u64) > file_len {
        return Err(IronBaseError::Corruption(format!(
            "Document at offset {} claims length {} but would exceed file boundary",
            offset, len
        )));
    }

    // Read data using pread
    let mut data = vec![0u8; len];
    self.file.read_at(&mut data, offset + 4)?;            // ← pread() syscall

    Ok(data)
}

// Windows: azonos logika, seek_read() syscall-lal (std::os::windows::fs::FileExt)
```

### Kulcs Megfigyelések

1. **✅ Nincs mmap** - Mindig `pread()` (Unix) vagy `seek_read()` (Windows)
2. **✅ Pozíció-független olvasás** - Nem változtatja a file pointer pozícióját
3. **✅ Thread-safe olvasás** - Több thread is olvashat egyszerre
4. **✅ Nincs 1GB korlát** - Nincs mmap, tehát nincs threshold sem
5. **✅ `data_end_offset` cache** - Nem kell `fstat()` syscall minden olvasásnál

---

## 📊 Teljesítmény Karakterisztikák

### 1. I/O Műveletek

#### Olvasás (Positioned I/O)

```
Művelet                    Syscall         Pozíció változik?   Konkurens?
──────────────────────────────────────────────────────────────────────────
read_data_at()            pread()          NEM                 ✅ IGEN
seek() + read()           seek() + read()  IGEN                ❌ NEM
mmap + access             page fault       NEM                 ✅ IGEN
```

**Előnyök:**
- ✅ Több olvasó párhuzamosan (nem versenyeznek a file pointeren)
- ✅ Nincs lock contention olvasásnál
- ✅ OS page cache-t használ (hatékony)

**Hátrányok:**
- ⚠️ Minden olvasás 2 syscall (pread: length + data)
- ⚠️ Nincs zero-copy (mmap lenne zero-copy)

---

#### Írás (Append-Only)

```rust
// Forrás: ironbase-core/src/storage/io.rs (pontos kód)

pub fn write_data(&mut self, data: &[u8]) -> Result<u64> {
    // Determine write position from data_end_offset
    let write_offset = if self.header.data_end_offset >= super::HEADER_SIZE {
        self.header.data_end_offset
    } else {
        super::HEADER_SIZE  // Migration fallback
    };

    // Seek to write position
    self.file.seek(SeekFrom::Start(write_offset))?;

    // Write length + data
    let len = (data.len() as u32).to_le_bytes();
    self.file.write_all(&len)?;
    self.file.write_all(data)?;

    // Update data_end_offset using HeaderWriter (prevents forgetting this step)
    HeaderWriter::new(&mut self.header, &mut self.file).advance_after_write()?;

    self.metadata_dirty = true;
    Ok(write_offset)
}
```

**Karakterisztikák:**
- ✅ Append-only (minden fájl végére íródik)
- ✅ Single writer (RwLock write lock)
- ⚠️ seek() + write() = 2 syscall per írás

---

### 2. Syscall Optimalizálás

#### `data_end_offset` Cache

**Performance Fix (2026-01-26):**

```rust
// RÉGI (lassú): fstat() syscall minden olvasásnál
pub fn read_data(&mut self, offset: u64) -> Result<Vec<u8>> {
    let file_len = self.file.metadata()?.len();  // ← fstat() syscall!
    // ...
}

// ÚJ (gyors): cached header value
pub fn read_data(&mut self, offset: u64) -> Result<Vec<u8>> {
    let file_len = self.header.data_end_offset;  // ← NINCS syscall!
    // ...
}
```

**Hatás:**
- **Index rebuild 133K dokumentumon:** 60 perc → 15 perc
- **Syscall csökkentés:** 133,000 fstat() elkerülve
- **Speedup:** 4x gyorsabb

---

## 📈 Skálázhatóság

### 1. Konkurens Hozzáférés

#### Thread Safety Model

```
┌─────────────────────────────────────────────────────────────┐
│              DatabaseCore<Arc<RwLock<StorageEngine>>>       │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  Read Operations (RwLock Read Lock - parallel):            │
│  ┌──────────────────────────────────────────────────────┐  │
│  │  find() → read_data_at() → pread()  ✅ Parallel      │  │
│  │  count() → scan() → pread()         ✅ Parallel      │  │
│  │  aggregate() → scan() → pread()     ✅ Parallel      │  │
│  └──────────────────────────────────────────────────────┘  │
│                                                             │
│  Write Operations (RwLock Write Lock - serialized):        │
│  ┌──────────────────────────────────────────────────────┐  │
│  │  insert() → write_data() → seek()+write()  ⚠️ Serial │  │
│  │  update() → write_data() → seek()+write()  ⚠️ Serial │  │
│  │  delete() → write_tombstone()              ⚠️ Serial │  │
│  └──────────────────────────────────────────────────────┘  │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

#### Gyakorlati Teljesítmény

| Művelet | Lock Type | Konkurencia | Throughput (Safe mode) | Adat forrás |
|---------|-----------|-------------|------------------------|-------------|
| **find()** | Read | ✅ Parallel | ~2000-3000 req/s | ⁽ᵇ⁾ becsült |
| **count()** | Read | ✅ Parallel | ~2000-3000 req/s | ⁽ᵇ⁾ becsült |
| **insert()** | Write | ❌ Serial | ~100 ops/s | ⁽ᵐ⁾ mért (speed_benchmarks.rs) |
| **update()** | Write | ❌ Serial | ~50-100 ops/s | ⁽ᵇ⁾ becsült |
| **delete()** | Write | ❌ Serial | ~50-100 ops/s | ⁽ᵇ⁾ becsült |

---

### 2. File Méret Skálázás

#### NINCS Hard Limit

```rust
// NINCS ilyen korlát a kódban!
// let use_mmap = file_size < 1_000_000_000;  // ← NEM LÉTEZIK

// Mindig pread/seek_read használatos
// Bármekkora file méret működik (de teljesítmény romolhat)
```

#### Valós Teljesítmény Adatok

**Mért esetek (⁽ᵐ⁾ = valós mérés, nem becsült):**

| File Méret | Dokumentumok | find() indexed | find() no index | Forrás |
|------------|--------------|----------------|-----------------|--------|
| **2.79 GB** | 118,000 | 26ms ⁽ᵐ⁾ | ~500ms ⁽ᵇ⁾ | Valós benchmark (2026-03) |
| **39.68 GB** | 78,295 | 40ms ⁽ᵐ⁾ | 3200ms ⁽ᵐ⁾ | QUERY_PERFORMANCE_REPORT.md |

**Megfigyelések:**
- ✅ 2.79 GB file: 26ms indexed find (elfogadható)
- ✅ 39.68 GB file: működőképes, de lassabb
- ⚠️ Teljesítményromlás lineáris file mérettel (nincs mmap)

---

### 3. Dokumentum Szám Skálázás

#### Tesztelt Mérföldkövek

| Dokumentum | Teszt Típus | Eredmény | Forrás |
|------------|-------------|----------|--------|
| **10,000** | Python performance | ✅ 1,000-5,000 insert/s | performance_test.py |
| **100,000** | Rust speed benchmark | ✅ 20,000+ insert/s (Batch) | speed_benchmarks.rs |
| **118,000** | Valós eset | ✅ 26ms indexed find | Valós benchmark |
| **500,000** | E2E stress test | ✅ Működőképes | test_e2e_extreme_650k.py |
| **1,000,000** | Scalability test | ✅ Tesztelve | CLAUDE.md |

---

## 🔍 Részletes Teljesítmény Elemzés

### 1. Olvasás vs Írás Arány

#### Read-Heavy Workload (Ajánlott)

```python
# ✅ JÓ: Sok olvasás, kevés írás
# 90% olvasás, 10% írás esetén kiváló

# Párhuzamos olvasók (nem versenyeznek)
thread1: collection.find({"age": {"$gte": 25}})  # pread()
thread2: collection.find({"city": "NYC"})         # pread()
thread3: collection.count({})                     # pread()

# Egy író (serializált)
thread4: collection.insert_one({"name": "Alice"})  # RwLock write
```

**Várható teljesítmény ⁽ᵇ⁾:**
- Olvasás: ~2000-3000 req/s (több thread összesen, becsült)
- Írás: ~100 ops/s (single writer, Safe mode mért)

---

#### Write-Heavy Workload (Nem ajánlott)

```python
# ⚠️ ROSSZ: Sok írás, kevés olvasás
# 90% írás, 10% olvasás esetén bottleneck

# Minden író sorban áll (RwLock write lock)
thread1: collection.insert_one({...})  # Várakozik
thread2: collection.insert_one({...})  # Várakozik
thread3: collection.insert_one({...})  # Várakozik
thread4: collection.update_one({...})  # Várakozik
```

**Várható teljesítmény ⁽ᵇ⁾:**
- Írás: ~100 ops/s (összes thread együtt, becsült)
- Olvasás: ~100-500 req/s (írási lock miatt, becsült)

---

### 2. Index Hatása

#### Valós Példa (39.68 GB, 78,295 dokumentum)

**Forrás:** `mcp-server/QUERY_PERFORMANCE_REPORT.md`

| Query | Index | Végrehajtási idő | Docs scanned |
|-------|-------|------------------|--------------|
| `{$exists: true, $ne: ""}` | Nincs | 2m 10s (130,000ms) | 78,295 |
| `{$gt: ""}` | B+ tree | 664ms | 69,707 |

**Javulás:** 196x gyorsabb

#### Index Types és Teljesítmény

| Index Típus | Létrehozás | Keresés | Használat |
|-------------|------------|---------|-----------|
| **B+ Tree** | O(n log n) | O(log n) | equality, range |
| **Compound** | O(n log n) | O(log n) | prefix matching |
| **Fuzzy** | O(n²) | O(n) | similarity search |
| **Fulltext** | O(n × terms) | O(terms) | BM25 scoring |
| **HNSW Vector** | O(n log n) | O(log n) | ANN search |

---

### 3. Durability Mode Hatás

#### Durability Mode Összefoglaló

A durability mode az `fsync()` hívás gyakoriságát szabályozza.

| Mode | fsync() | Throughput | Latency | Crash safety | Forrás |
|------|---------|------------|---------|--------------|--------|
| **Safe** (default) | Minden írásnál | ~100 ops/s ⁽ᵐ⁾ | ~10ms ⁽ᵇ⁾ | ✅ Zero loss | CLAUDE.md |
| **Batch** | Minden N. írásnál | ~20K ops/s ⁽ᵐ⁾ | ~0.5ms ⁽ᵇ⁾ | ⚠️ Max N elveszhet | speed_benchmarks.rs |
| **Unsafe** | Manuális/nincs | ~50-100K ops/s ⁽ᵐ⁾ | ~0.01ms ⁽ᵇ⁾ | ❌ Last checkpoint óta | speed_benchmarks.rs |

⁽ᵐ⁾ = mért (Rust speed benchmarks, 10K-100K doc)
⁽ᵇ⁾ = becsült (throughput-ból derivált)

---

## 🚧 Skálázhatóság Korlátok

### 1. Single-Writer Bottleneck

**Gyökér ok:**
```rust
// DatabaseCore::insert_one()
let mut storage = self.storage.write()?;  // ← RwLock write lock
storage.write_document(collection, doc_id, data)?;
// Lock released
```

**Következmény:**
- Minden írás (insert, update, delete) serializálva van
- Hiába 100 concurrent client, írások sorban állnak
- Max write throughput = 1 writer teljesítménye

**Jelenlegi megoldások:**
1. **Batch mode:** 200x throughput növekedés (~100 → ~20,000 ops/s)
2. **Bulk operations:** `insert_many()` vs `insert_one()`
3. **Transaction batching:** Több művelet egy tranzakcióban

**Nem implementált (jövő):**
- ❌ MVCC (Multi-Version Concurrency Control)
- ❌ Partitioned writes
- ❌ WAL parallel commit

---

### 2. Syscall Overhead

#### Minden Olvasás: 2 syscall

```
find() → read_data_at()
  ├─ pread(offset)      → 4 bytes (length)
  └─ pread(offset + 4)  → N bytes (data)
```

**Költség:**
- Syscall overhead: ~1-2 µs per syscall
- 2 syscall per olvasás = ~2-4 µs overhead
- 1000 olvasás = ~2-4 ms csak syscall overhead

**Optimalizálás:**
- ✅ `data_end_offset` cache (eliminálja fstat() syscall-t)
- ✅ OS page cache (pread olvasások a kernel page cache-ből szolgálhatók ki, nem mindig fizikai I/O)
- ✅ OS read-ahead (a kernel felismeri a szekvenciális olvasási mintát és előre betölti a következő page-eket)
- ⚠️ Nincs alkalmazás-szintű read buffering (minden olvasás syscall, de a kernel cache-ből gyors)

---

### 3. Result Set Memória Kezelés

#### Alap `find()` — Vec visszatérés

Az alap `find()` API `Vec<Value>`-t ad vissza (összes eredmény RAM-ban). Ez kényelmes, de nagy result set-eknél memóriaigényes.

#### OOM Védelem (implementált)

**A dokumentum NEM védtelen a nagy result set-ek ellen.** Több szintű védelem létezik:

1. **`FindOptions::with_safe_defaults()`** — RAM-arányos `max_response_bytes` limit:

| Elérhető RAM | max_response_bytes |
|--------------|-------------------|
| < 512 MB | 10 MB |
| 512MB - 2GB | 50 MB |
| 2 - 8 GB | 100 MB |
| 8 - 32 GB | 200 MB |
| > 32 GB | 500 MB |

2. **`scan_with_early_termination()`** — skip/limit pushdown a scan szintre (nem tölti be a felesleges dokumentumokat)

3. **`AggregationLimits::from_system_memory()`** — aggregation pipeline OOM védelem (max docs, max RAM)

4. **`try_reserve()`** — allokáció előtt ellenőrzés, graceful error OOM panic helyett

#### Streaming Cursor (implementált)

```rust
// Forrás: ironbase-core/src/collection_core/cursor.rs

// FindCursor - Memory-efficient iterator for large result sets
let mut cursor = collection.find_streaming(&query)?;

// Batch feldolgozás (nem tölti be az összes dokumentumot egyszerre)
while !cursor.is_finished() {
    let batch = cursor.next_chunk(100)?;
    for doc in batch {
        process(doc);
    }
}

// Vagy egyenként
while let Some(doc) = cursor.next()? {
    process(doc);
}
```

#### Fennmaradó Korlátok

- ⚠️ Az alap `find()` API továbbra is `Vec`-et ad vissza (streaming-hez `find_streaming()` kell)
- ⚠️ Aggregation pipeline köztes eredményei RAM-ban (de `AggregationLimits` véd)
- 🔵 **Jövő:** Cursor-alapú pagination REST/MCP API szinten

---

## 📊 Összehasonlítás Más Adatbázisokkal

### I/O Stratégia

| Adatbázis | I/O Módszer | Read Scale | Write Scale |
|-----------|-------------|------------|-------------|
| **IronBase** | `pread()`/`seek_read()` | Multi-reader | Single writer |
| **SQLite** | `pread()` + page cache | Multi-reader | Single writer |
| **MongoDB** | WiredTiger B-tree (v3.2+) | Sharded | Sharded |
| **Redis** | RAM (no I/O) | Single thread | Single thread |
| **PostgreSQL** | Buffer pool + I/O | MVCC | MVCC |

### Teljesítmény (Write Throughput)

| Adatbázis | Safe Mode | Batch Mode | Unsafe |
|-----------|-----------|------------|--------|
| **IronBase** | ~100 ops/s ⁽ᵐ⁾ | ~20K ops/s ⁽ᵐ⁾ | ~50-100K ops/s ⁽ᵐ⁾ |
| **SQLite** | ~50 ops/s ⁽ᵖ⁾ | ~10K ops/s ⁽ᵖ⁾ | ~30K ops/s ⁽ᵖ⁾ |
| **MongoDB** | ~1K ops/s ⁽ᵖ⁾ | ~10K ops/s ⁽ᵖ⁾ | ~50K ops/s ⁽ᵖ⁾ |
| **PostgreSQL** | ~100 ops/s ⁽ᵖ⁾ | ~5K ops/s ⁽ᵖ⁾ | ~20K ops/s ⁽ᵖ⁾ |

⁽ᵐ⁾ = IronBase mért (speed_benchmarks.rs), ⁽ᵖ⁾ = publikált benchmark-okból származó hozzávetőleges értékek

**Megjegyzés:** IronBase Batch mode versenyképes!

---

## 🎯 Ajánlások Skálázáshoz

### 1. Read-Heavy Workload (Optimális)

```python
# ✅ IDEÁLIS: Sok olvasás, kevés írás

# Indexek létrehozása (kritikus!)
collection.create_index("email", unique=True)
collection.create_index("created_at")

# Párhuzamos olvasók (thread-safe)
results = collection.find({"email": "user@example.com"})  # Indexed: ~0.1ms

# Projection (kevesebb adat)
results = collection.find(
    {"email": "user@example.com"},
    projection={"name": 1, "email": 1}  # Ne töltsön be minden mezőt
)
```

**Várható teljesítmény:**
- Indexed find: ~0.1-1ms
- Concurrent reads: ~2000-3000 req/s

---

### 2. Write-Heavy Workload (Kerülendő)

```python
# ⚠️ NEM AJÁNLOTT: Sok írás

# ❌ ROSSZ: Sok kicsi írás
for doc in docs:
    collection.insert_one(doc)  # ~10ms per insert (Safe mode)

# ✅ JÓ: Bulk insert + Batch mode
db = IronBase("app.mlite", durability="batch", batch_size=100)
collection.insert_many(docs, batch_size=1000)  # ~0.5ms per insert
```

**Várható javulás:**
- Safe mode: ~10ms per insert
- Batch mode: ~0.5ms per insert (20x gyorsabb)

---

### 3. Vegyes Workload (Gyakori)

```python
# 80% olvasás, 20% írás

# ✅ JÓ: Read cache + Batch writes

# Olvasás (gyors, concurrent)
results = collection.find({"status": "active"})

# Írás batchelve (lassú, serializált)
with db.transaction():
    collection.insert_one({"data": 1})
    collection.insert_one({"data": 2})
    collection.update_one({"_id": 5}, {"$set": {"updated": True}})
# Egy lock acquisition, több művelet
```

---

## 📋 Következtetések

### Erősségek (Valós)

✅ **Positioned I/O:** `pread()` lehetővé teszi concurrent olvasást
✅ **Append-only:** Egyszerű crash recovery, lock-free backup
✅ **data_end_offset cache:** Eliminálja fstat() syscall-t (4x speedup ⁽ᵐ⁾)
✅ **Multi-reader:** Több párhuzamos olvasó (RwLock read)
✅ **Batch mode:** 200x write throughput növekedés ⁽ᵐ⁾
✅ **OOM védelem:** FindOptions, AggregationLimits, try_reserve(), FindCursor
✅ **OS page cache:** pread kihasználja a kernel cache-t és read-ahead-et

### Gyengeségek (Valós)

⚠️ **Single-writer bottleneck:** Minden írás serializálva
⚠️ **Syscall overhead:** 2 syscall per olvasás (pread: length + data)
⚠️ **Alap find() Vec visszatérés:** Nagy result set memóriaigényes (de FindCursor és OOM védelem létezik)
⚠️ **Nincs alkalmazás-szintű read buffer:** Minden olvasás syscall (de OS page cache gyorsítja)

### NEM Létező Korlátok (Korábbi Hamis Infók)

❌ **"1GB mmap korlát"** - NINCS mmap, tehát nincs korlát  
❌ **"3-4x lassulás 1GB felett"** - Nincs mmap/standard I/O váltás  
❌ **"StorageEngine.mmap mező"** - NEM LÉTEZIK a kódban  
❌ **"Memory-mapped I/O"** - HAMIS, mindig pread/seek_read  

---

## 🔮 Jövőbeli Fejlesztések

### Priority Backlog

| Feature | Priority | Státusz | Várható Haszon |
|---------|----------|---------|----------------|
| **Cursor API** | High | ✅ Implementált (`find_streaming()`) | Streaming nagy result set-ekhez |
| **OOM védelem** | High | ✅ Implementált (`FindOptions`, `AggregationLimits`) | RAM-arányos limitek |
| **Read buffer/cache** | Medium | ❌ Nincs (OS page cache-re támaszkodik) | ~2-5x olvasás speedup |
| **MVCC** | Low | ❌ Nincs | 10x write concurrency |
| **Connection pool** | Low | ❌ Nincs | 2x HTTP throughput |

---

## 📞 Források

### Valós Kód

- **Storage I/O:** `ironbase-core/src/storage/io.rs` (read_data_at, write_data)
- **Storage Engine:** `ironbase-core/src/storage/mod.rs` (append-only, catalog)
- **Traits:** `ironbase-core/src/storage/traits.rs` (Storage trait)

### Benchmark Dokumentumok

- **Speed benchmarks:** `ironbase-core/tests/speed_benchmarks.rs`
- **Performance test:** `performance_test.py`
- **Query performance:** `mcp-server/QUERY_PERFORMANCE_REPORT.md`

### Koncurrency Tesztek

- **Concurrency tests:** `tests/concurrency/`
- **Concurrent insert/read:** `mcp-server/tests/concurrent_insert_read.sh`

---

**Dokumentum verzió:** 1.1 (Kód-alapú, javított — pseudokód→valós kód, OOM/streaming/read-ahead pontosítás)
**Utolsó frissítés:** 2026. március 9.
**Státusz:** ✅ Review complete — pontos, kód-alapú, mért/becsült adatok jelölve
