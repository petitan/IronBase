# CLAUDE.md

**IronBase** - MongoDB-kompatibilis embedded NoSQL (Rust + Python/C# bindings)

| Jellemző | Érték |
|----------|-------|
| Tesztek | 939+ (unit + integration + doctest) |
| API-k | Rust, Python (PyO3), C# (.NET 8) |
| Operátorok | 21 query, 7 update |
| Indexek | B+ tree, compound, fuzzy, fulltext, HNSW |
| Keresés | Fuzzy (Jaro-Winkler/Levenshtein), TF-IDF, RAG |

---

## Build

| Művelet | Parancs |
|---------|---------|
| Python dev build | `maturin develop` |
| Rust tesztek | `cargo test -p ironbase-core` |
| Egyetlen teszt | `cargo test -p ironbase-core -- test_name` |
| Full CI | `just run-dev-checks` |
| .NET tesztek | `cd IronBase.NET && dotnet test` |
| MCP Server | `cd mcp-server && cargo build --release` |

<details>
<summary>Fuzz Testing (nightly)</summary>

```bash
cd ironbase-core/fuzz
cargo +nightly fuzz run fuzz_query_parser -- -max_total_time=60
cargo +nightly fuzz run fuzz_wal_bytes -- -max_total_time=60
```
</details>

## Architecture

```
ironbase-core/src/     bindings/python/    IronBase.NET/    mcp-server/
├── database.rs        └── PyO3            └── .NET 8       └── HTTP/stdio
├── collection_core/
├── query/operators.rs
├── aggregation.rs
├── storage/
├── index.rs
├── transaction.rs
└── wal.rs
```

### Core Modules

| Modul | Felelősség | Kulcs API |
|-------|------------|-----------|
| `database.rs` | Lifecycle, durability | `open()`, `open_memory()` |
| `collection_core/` | CRUD, aggregation | `find`, `insert`, `update`, `delete`, `aggregate` |
| `query/operators.rs` | Query engine | $eq, $gt, $in, $regex, $and, $or... |
| `aggregation.rs` | Pipeline | $match, $group, $sort, $limit + accumulators |
| `storage/` | Append-only engine | `.mlite` files, compaction |
| `index.rs` | B+ tree indexing | `create_index`, `explain`, `hint` |
| `transaction.rs` | ACID | WAL, Read Committed isolation |

<details>
<summary>collection_core/ részletek</summary>

| Fájl | Funkció |
|------|---------|
| `mod.rs` | find/insert/update/delete, scan_with_early_termination |
| `aggregate.rs` | aggregate_auto, aggregate_with_limits |
| `count.rs` | count_documents, adjust_count_for_tombstones |
| `distinct.rs` | try_index_based_distinct |
| `tx.rs` | insert_one_tx, update_one_tx |
| `topk.rs` | Top-K heap selection |
</details>

<details>
<summary>Storage File Format (.mlite v2+)</summary>

```
Header (256 bytes)     → magic: "MONGOLTE", metadata_offset
Document Data          → [u32 len][JSON bytes]... (append-only)
Collection Metadata    → document_catalog, indexes (end of file)
```
Metadata at END → no race conditions, no truncation.
</details>

### HeaderWriter - KRITIKUS

**TILOS `data_end_offset`-ot közvetlenül módosítani!**

| Művelet | Metódus |
|---------|---------|
| Doc write | `HeaderWriter::new(...).advance_after_write()` |
| Metadata flush | `HeaderWriter::new(...).set_after_metadata(offset, size)` |
| Compaction | `write_compaction_header(...)` |

7+ bug volt mert valaki elfelejtette → HeaderWriter AUTOMATIKUSAN számolja.

---

## Operátorok

| Típus | Operátorok |
|-------|-----------|
| **Query (26)** | $eq $ne $gt $gte $lt $lte $in $nin · $and $or $not $nor · $exists $type · $all $elemMatch $size · $regex $fuzzy $text · $startsWith $endsWith $contains · $expr · $** |
| **Update (7)** | $set $inc $unset $push $pull $addToSet $pop (+ dot notation + upsert) |
| **Aggregation** | $match $group $project $count $sort $limit $skip $unwind · Accumulators: $sum $avg $min $max $first $last $push $addToSet |

---

## OOM Protection

**Használat:** `aggregate_auto()` / `FindOptions::with_safe_defaults()`

| RAM | Aggregation max | Find max |
|-----|-----------------|----------|
| < 512 MB | 64 MB, 10K docs | 10 MB |
| 512MB-2GB | 128 MB, 50K docs | 50 MB |
| 2-8 GB | 256 MB, 100K docs | 100 MB |
| 8-32 GB | 512 MB, 250K docs | 200 MB |
| > 32 GB | 1024 MB, 500K docs | 500 MB |

**Védelmek:** ✅ `$match` nem kapcsolja ki limitet · ✅ `$push/$addToSet` per-group limit · ✅ `$unwind` kumulatív · ✅ `try_reserve()`

<details>
<summary>Top-K Optimization</summary>

`$sort` + `$limit` → BinaryHeap O(k) memória O(n) helyett.

| Hely | Funkció |
|------|---------|
| `topk.rs` | Generic Top-K |
| `sort_stage.rs` | Aggregation $sort+$limit |
| `hnsw.rs` | Nearest neighbor |
</details>

<details>
<summary>Kisebb Fixek</summary>

| Fix | Probléma | Megoldás |
|-----|----------|----------|
| read_data_at() | Unflushed doc hiba | `data_end_offset` |
| live_document_count | find 200s+ régi DB | Migration |
| delete_one O(1) | 0.56s → <0.05s | `_id` index lookup |
</details>

## Development Guidelines

### When Adding Features
1. Implement in Rust first (ironbase-core)
2. Add PyO3 bindings (bindings/python/src/lib.rs)
3. Add C# bindings if needed (IronBase.NET)
4. Update tests
5. Use `just run-dev-checks` before committing

### Thread Safety
- `Arc<RwLock<StorageEngine>>` for shared storage (parking_lot::RwLock)
- Write lock: insert, update, delete
- Read lock: find, count, list_collections

### Error Handling (API)
- Rust: `Result<T>` with `IronBaseError` (thiserror)
- Python: Map to PyIOError, PyRuntimeError, PyValueError
- C#: Map to appropriate .NET exceptions

### Kód Konzisztencia Protokoll

> **Előbb OLVASD a meglévő kódot, utána ÍRJ újat.**

**Új kód előtt:**
1. Keress hasonló fájlt/funkciót
2. Azonosítsd: naming, error handling, logging, kommentek
3. Kövesd amit találtál — ne találj ki újat

| Kérdés | Hol nézd |
|--------|----------|
| Változó nevek? | Hasonló fájlok |
| Error kezelés? | Meglévő handlers |
| Függvény signatúra? | Hasonló funkciók |
| Hova kerül? | Könyvtárstruktúra |
| Van helper? | `utils/`, `common/` |

**TILOS:**
- Új pattern ha van meglévő
- Más naming konvenció
- "Szerintem jobb" refaktor konzultáció nélkül

**Kimenet új kódnál:**
```
Mintának használt: [fájl] — [mit vettem át]
Konvenciók: [naming, error, stb.]
```

> **Ha nem találsz mintát → kérdezz, ne találj ki.**

---

## Adatstruktúra Védelem

**SOHA NE MÓDOSÍTS KÉRDÉS NÉLKÜL:**
- Adatbázis séma (új mező, tábla, index)
- Meglévő mezők típusa/neve
- Konfiguráció
- API interface
- Publikus struktúrák

**HA "EGYSZERŰBB LENNE" ÚJ MEZŐVEL:**
1. ÁLLJ MEG
2. Kérdezd meg: "Létrehozhatok egy új mezőt: [név], [típus], [cél]?"
3. VÁRD MEG A VÁLASZT
4. "Egyszerűbb" ≠ "Szabad"

**INDOKLÁS:**
- Az adat struktúra ARCHITEKTÚRA döntés
- Te NEM vagy architekt
- A "gyors fix" === technikai adósság
- Migrációs költség > a te kényelmed

> **Nincs jogod módosítani amit nem te terveztél. Kérdezz. Mindig.**

---

## Hibakezelési Protokoll

### 0. ALAPELV - TE DETEKTÍV VAGY, NEM MEGOLDÓ

> **Az adat szemét. A hiba érték. A megértés cél.**

A feladatod NEM a probléma megoldása. A feladatod a probléma **MEGÉRTÉSE**.

**TILOS:**
- Workaround írása
- "Másik megközelítés" javaslata
- A hiba megkerülése
- "Ez így is működik" megoldás
- Kód futtatása DIAGNÓZIS NÉLKÜL

**KÖTELEZŐ SORREND:**
1. ÁLLJ MEG
2. Mi a PONTOS hibaüzenet?
3. MELYIK SORBAN keletkezik?
4. MIÉRT keletkezik?
5. Mi a ROOT CAUSE?

**Amíg az 5. pontra nincs válaszod → NEM NYÚLSZ A KÓDHOZ.**

**HA KÓDOT AKARSZ ÍRNI, ELŐBB MONDD EL:**
- Mi a hiba oka (1 mondat)
- Honnan tudod (bizonyíték)
- Miért pont ez a javítás

Ha nem tudod → TOVÁBB DEBUGOLSZ, nem kódolsz.

**EMLÉKEZTETŐ:**
- A hiba MEGKERÜLÉSE ≠ MEGOLDÁS
- A hiba ELREJTÉSE ≠ JAVÍTÁS
- A kód MŰKÖDIK ≠ A kód JÓ

### 1. STOP Szabály

**2× szabály:** 2 sikertelen kísérlet után ÁLLJ MEG és elemezz.

**TILOS:**
- Ugyanazt újrapróbálni más paraméterekkel
- Workaround keresése root cause előtt
- "Majd most sikerül" megközelítés
- Problémát megkerülni (pl. Python binding helyett MCP)

**Red Flags** (ha ezeket látod magadon, ÁLLJ MEG):
- "Próbáljuk még egyszer"
- "Növeljük a timeout-ot"
- 3+ próbálkozás ugyanarra
- 10+ perce ugyanazon a problémán

### 2. Debug Folyamat
```
STOP      → Ne próbáld újra azonnal
OLVASD    → Mi a pontos hibaüzenet? Melyik sor dobta?
HIPOTÉZIS → 2-3 lehetséges ok
VALIDÁLD  → Ellenőrizd melyik igaz (NE JAVÍTS MÉG)
JAVÍTS    → Csak ha tudod mi a root cause
```

### 3. Fix Ellenőrzőlista

**KÖTELEZŐ: Fix előtt keress MINDEN code path-ot ahol ugyanaz a logika!**

```bash
git grep "hasonló_pattern"  # Összes előfordulás
```

| Ellenőrzés | Kész? |
|------------|-------|
| Minden fájl ahol ugyanaz a logika | ☐ |
| Duplikált kód → közös utility | ☐ |
| Teszt MINDEN érintett path-ra | ☐ |

**Code Duplication Red Flags:**
- `collection_core/` és `database/` ugyanazzal a logikával
- `create_*` és `rebuild_*` hasonló tartalommal
- Startup/warmup vs runtime ugyanazzal a művelettel

### 4. OOM Minták (Rust)

| Minta | Kockázat | Keresendő |
|-------|----------|-----------|
| Korlátlan collect | Magas | `.collect::<Vec>()` limit nélkül |
| Tömeges betöltés | Magas | `load_all`, `get_all`, `fetch_all` |
| Hiányzó try_reserve | Közepes | `Vec::new()` + loop push |
| Parallel chunk nélkül | Közepes | `par_iter()` nagy kollekcióra |
| Hardcoded limit | Közepes | `100_000`, `50_000` konstansok |

**Javítási minták:**

```rust
// 1. Streaming - egy doc egyszerre
for doc_id in doc_ids {
    let doc = load_one(doc_id)?;
    process(doc);
}

// 2. try_reserve - allokáció előtt
let mut results = Vec::new();
results.try_reserve(count).map_err(|e| IronBaseError::OutOfMemory(...))?;

// 3. Chunked parallel
for chunk in entries.chunks(1000) {
    let batch = load_batch(chunk);
    process_parallel(batch);
}

// 4. Dinamikus limitek (NE hardcoded!)
let limits = AggregationLimits::from_system_memory();
```

### 5. Kimenet Formátum
```markdown
## [fájl/hiba]

### Probléma
[mi a hiba, melyik sor]

### Root Cause
[miért történt]

### Javítás
[csak ha a root cause ismert]

### Memória (ha OOM)
- Előtte: O(?)
- Utána: O(?)
```

> **A cél MEGÉRTENI miért nem működik. Az adat SZEMÉT. A tudás ÉRTÉK.**

---

### Egységes Lazy Loading (2026-01-25)

**Minden index típus támogatja a `LazyLoadable` trait-et a gyors startup és OOM védelem érdekében.**

**Működés (Opció A - Read-only):**
- **Startup**: Csak metadata betöltése ha fájl > threshold
- **Keresés**: `ensure_fully_loaded()` majd memóriában keres
- **Módosítás**: `ensure_fully_loaded()` majd memóriában módosít
- **Persistence**: Változatlan (checkpoint = teljes újraírás)

**RAM-alapú threshold (`calculate_lazy_threshold()`):**

| Elérhető RAM | Lazy Threshold |
|--------------|----------------|
| < 2 GB       | 50 MB          |
| 4 GB         | 100 MB         |
| 8 GB         | 200 MB         |
| 16 GB        | 400 MB         |
| 32 GB+       | 500 MB         |

**Index támogatás:**

| Index | Lazy Loading | Megjegyzés |
|-------|--------------|------------|
| B+ Tree | ✅ Igen | `load_from_path()` threshold check |
| Fulltext | ✅ Igen | V2/V3 format mindig lazy |
| Fuzzy | ✅ Igen | Dinamikus threshold |
| HNSW | ❌ Nem | Gráf struktúra miatt komplex |

**Használat:**

```rust
use ironbase_core::index::traits::{LazyLoadable, calculate_lazy_threshold};

// Threshold lekérdezés
let threshold = calculate_lazy_threshold(); // RAM-alapú

// Index lazy állapot ellenőrzés
if index.is_lazy_mode() {
    index.ensure_fully_loaded()?;
}

// IndexManager monitoring
let mem_mb = index_manager.total_memory_usage() / (1024 * 1024);
let lazy_count = index_manager.lazy_index_count();
index_manager.log_lazy_status(); // tracing::info!
```

**Key files:**
- `ironbase-core/src/index/traits.rs` - LazyLoadable trait, calculate_lazy_threshold()
- `ironbase-core/src/index/btree.rs` - B+ tree lazy loading
- `ironbase-core/src/index/fuzzy.rs` - Fuzzy lazy loading
- `ironbase-core/src/fulltext.rs` - Fulltext lazy loading
- `ironbase-core/src/index/manager.rs` - Memory tracking methods

### Egységes Range Query API (KÖTELEZŐ!)

**2025-01-től a `range_query()` az EGYETLEN ajánlott belépési pont minden B+ tree range művelethez.**

```rust
use crate::index::{RangeQueryMode, RangeQueryResult, ScanOrder};

// ✅ HELYES: Count O(1) memóriával
let result = btree.range_query(
    &start, &end, true, true,
    RangeQueryMode::Count
);
let count = result.unwrap_count();

// ✅ HELYES: Scan limittel O(limit) memóriával
let result = btree.range_query(
    &start, &end, true, true,
    RangeQueryMode::Scan { skip: 0, limit: Some(10), order: ScanOrder::Asc }
);
let docs = result.unwrap_docs();

// ❌ TILOS: Régi metódusok közvetlen használata (wrapper-ek, OOM kockázat limit nélkül!)
// btree.range_scan() - NE HASZNÁLD új kódban!
// btree.range_scan_reversed_with_limit() - NE HASZNÁLD új kódban!
```

**Memória garanciák:**

| Művelet | Mód | Memória |
|---------|-----|---------|
| Count | `RangeQueryMode::Count` | O(1) |
| Scan + limit | `RangeQueryMode::Scan { limit: Some(k) }` | O(k) |
| Scan unlimited | `RangeQueryMode::Scan { limit: None }` | O(n) ⚠️ |

**Top-K dokumentum rendezés (sort + limit):**

```rust
use crate::collection_core::{topk_documents, compare_docs_by_sort};

// ✅ HELYES: Top-K O(k) memóriával
let sort_spec = vec![("age".to_string(), 1)]; // ASC
let top10 = topk_documents(docs.into_iter(), 0, 10, &sort_spec);

// ❌ TILOS: Teljes rendezés majd limit
// docs.sort_by(...); docs.truncate(10); - NE CSINÁLD!
```

**Törölt dead code (e445b44e):**
- `scan_documents_via_catalog()` - törölve
- `batch_read_documents_by_ids()` - törölve
- `count_live_docs_from_ids()` - törölve
- `parallel.rs` modul - törölve

### Dokumentált Bugok (Referencia)

**Korábbi OOM hibák (commit):**
`4904ccc9`, `567e0d11`, `e0001bbe`, `49f27a77`, `88f0a79c`, `e445b44e`

**Kritikus bugok:**

| Bug | Commit | Tünet | Root Cause | Fix |
|-----|--------|-------|------------|-----|
| **WAL Unbounded Growth** | 2026-01-11 | OOM startup, 29GB .wal | `wal.clear()` csak close-kor | Periodikus clear 100 commit után |
| **Sparse Index []** | a54f29a1 | count 300s+ timeout | `[]` = "hiányzó mező" | `get_nested_value().is_some()` |
| **Stale Index Loading** | 9ff48302 | Phantom duplikátumok | `.idx` tombstone-okkal | `was_clean` check + Drop fix |
| **HNSW NaN** | df5cee21 | Rossz heap rendezés | NaN összehasonlítás | NaN → max distance |
| **Fulltext count** | 169e2e6b | Dupla számolás | Lazy mode bug | HashSet union |
| **HNSW PRNG race** | b71c5012 | Thread-safety | Random level race | `compare_exchange_weak` |
| **Index hash collision** | b71c5012 | Fájl ütközés | 32-bit hash | 64-bit hash |
| **read_data() boundary** | d37e442a | 1 doc különbség count-ban | `file_len` vs `data_end_offset` | `data_end_offset` használata |
| **Lazy index get_all_entries** | 2026-01-26 | $group/distinct 0 eredmény | `lazy_mode` nem kezelt | Fájlból olvasás lazy mode-ban |
| **Index building flag** | 9273a19b | explain() 0 availableIndexes | `set_index_ready()` hiányzik rebuild után | `set_index_ready()` hívás rebuild végén |
| **$eq operator ignored** | 6f537dd5 | `{"field":{"$eq":"x"}}` CollectionScan | `collect_equality_candidates()` skip-elte | `$eq` érték kinyerése |

<details>
<summary>Lazy Index get_all_entries() Bug - Részletes elemzés</summary>

**Probléma:** `$group` aggregáció és `distinct()` 0 eredményt adott vissza nagy kollekciókon (130K+ doc).

**Érintett kód:** `ironbase-core/src/index/btree.rs` - `get_all_entries()`

**Root Cause:**
- Ha az index **lazy mode**-ban van (nagy fájl, vagy dirty shutdown után), a `self.root` üres
- A `get_all_entries()` NEM ellenőrizte a `lazy_mode` flag-et
- Így a `collect_entries_recursive(üres_root)` mindig üres Vec-et adott vissza

**Tünetek:**
- `$group` aggregáció: 0 csoport
- `distinct()`: 0 egyedi érték
- DE: `count_documents()`, `find()` működött (nem használják `get_all_entries()`-t)

**Fix:**
```rust
pub fn get_all_entries(&self) -> Vec<(IndexKey, DocumentId)> {
    // LAZY MODE: read directly from file
    if self.lazy_mode {
        if let Some(ref path) = self.source_path {
            if let Ok(mut file) = File::open(path) {
                if let Ok(entries) = self.get_all_entries_with_file(&mut file) {
                    return entries;
                }
            }
        }
    }
    // IN-MEMORY PATH (existing logic)
    ...
}
```

**Érintett függvények:**
- `aggregation/stages/group_stage.rs:664` - `try_index_based_execute_with_context()`
- `collection_core/distinct.rs:188` - `try_index_based_distinct()`

</details>

<details>
<summary>read_data() Boundary Bug (d37e442a) - Részletes elemzés</summary>

**Probléma:** Duplikátum keresésnél 1 dokumentumnyi különbség volt a várt és tényleges count között.

**.mlite fájl szerkezete:**
```
┌─────────────────────────────────────────────────────────────────┐
│ Header (256 bytes)                                              │
│ ├─ magic: "MONGOLTE"                                            │
│ ├─ data_end_offset: u64  ← dokumentumok vége                    │
│ └─ metadata_offset: u64  ← metaadatok kezdete                   │
├─────────────────────────────────────────────────────────────────┤
│ Document Region (append-only)                                   │
│ ├─ [len:4][JSON doc 1]                                          │
│ ├─ [len:4][JSON doc 2]                                          │
│ └─ ...                                                          │
│ ↑ offset: 256 .. data_end_offset                                │
├─────────────────────────────────────────────────────────────────┤
│ Padding (változó hossz)                                         │
│ ↑ data_end_offset .. metadata_offset                            │
├─────────────────────────────────────────────────────────────────┤
│ Collection Metadata (JSON)                                      │
│ ├─ document_catalog: HashMap<DocumentId, offset>                │
│ ├─ live_document_count: u64                                     │
│ └─ indexes, schemas...                                          │
│ ↑ metadata_offset .. FILE_END                                   │
└─────────────────────────────────────────────────────────────────┘
```

**Root Cause:** Két különböző határ!

| Érték | Mit mér | Tartalmazza |
|-------|---------|-------------|
| `file.metadata()?.len()` | Teljes fájl | Header + Docs + Padding + **Metadata** |
| `data_end_offset` | Dokumentum régió | Header + Docs (metadata NÉLKÜL) |

**Hogyan okozott 1 doc különbséget:**
```rust
// Régi kód (HIBÁS):
pub fn read_data(&mut self, offset: u64) -> Result<Vec<u8>> {
    let file_len = self.file.metadata()?.len();  // ← SYSCALL + rossz határ
    if offset >= file_len { ... }  // ← padding/metadata-t is "dokumentumnak" látta
}

// Új kód (HELYES):
pub fn read_data(&mut self, offset: u64) -> Result<Vec<u8>> {
    let data_boundary = self.header.data_end_offset;  // ← Cached + helyes határ
    if offset >= data_boundary { ... }  // ← csak dokumentum régiót nézi
}
```

**Eredmények:**

| Metrika | Előtte | Utána |
|---------|--------|-------|
| Határ ellenőrzés | Teljes fájl | Csak dokumentum régió |
| Syscall / olvasás | 1 fstat() | 0 |
| Index rebuild 133K doc | ~60 perc | ~15 perc |
| Duplikátum különbség | 1 doc | 0 |

**Érintett fájlok:**
- `storage/io.rs:57-119` - `read_data()` javítva
- `storage/io.rs:141,199` - `read_data_at()` már korábban `data_end_offset`-et használt

</details>

<details>
<summary>Workaround-ok (kattints)</summary>

**WAL Unbounded Growth:**
- Tünet: `[STARTUP/DB] StorageEngine opened, recovering WAL...` után crash
- Workaround: Töröld a `.wal` fájlt (backup mlite előtte!)
- `.wal.tmp` auto-törlés startup-kor (2026-01-22)

**Stale Index Loading:**
- `database/collections.rs` - `load_persisted_indexes(..., was_clean: bool)`
- `storage/mod.rs` - `mark_clean_shutdown()` a Drop-ban
- Érintett: `.idx`, `.fzidx`, `.ftidx`, `.hnsw`
</details>

### C# / .NET Native Library Caching Issue
When rebuilding the Rust FFI library (`libironbase_ffi.so`), .NET caches the native library in `Demo/bin/Debug/net8.0/`. Even if you copy the updated library to `runtimes/linux-x64/native/`, .NET continues using the cached version.

**Solution**: Copy directly to the bin folder:
```bash
# After building the FFI library
cargo build --release -p ironbase-ffi

# Copy to .NET's actual load location
cp target/release/libironbase_ffi.so IronBase.NET/Demo/bin/Debug/net8.0/libironbase_ffi.so
```

This is especially important when debugging FFI issues - if debug logging doesn't appear, check that the correct library version is being loaded.

### Python Database Closure (GC Timing Issue)

When using Python bindings, you **must call `db.close()`** before reopening the same database file. Python's garbage collector does not immediately call Rust's `Drop` trait when you use `del db`.

**Symptom**: "IO error: failed to fill whole buffer" when reopening a database

**Root Cause**:
- `del db` only marks the object for garbage collection
- The Rust `Drop` (which calls `flush()`) runs later when GC runs
- If you reopen the database before GC runs, the old instance still holds the file and unflushed data

**Correct usage**:
```python
from ironbase import IronBase

# Create and use database
db = IronBase("/tmp/test.mlite")
db.insert_one("users", {"name": "Alice"})
db.close()  # REQUIRED before reopening!

# Now safe to reopen
db2 = IronBase("/tmp/test.mlite")
print(db2.count_documents("users", {}))  # Works correctly
```

**Alternative** (not recommended):
```python
import gc
del db
gc.collect()  # Forces GC to run Drop immediately
```

**Note**: This issue does not affect Rust code, where scopes trigger `Drop` deterministically.

## MCP Server

**DEFAULT DB:** `/home/petitan/MongoLite/mcp-server/ironbase_data.mlite`

```bash
cd mcp-server && cargo build --release
./target/release/mcp-ironbase-server --db /path/to/db.mlite
./target/release/mcp-ironbase-server --stdio  # Claude Desktop
```

| Env | Leírás |
|-----|--------|
| `IRONBASE_PATH` | DB fájl |
| `IRONBASE_ADMIN_KEY` | Admin kulcs |
| `IRONBASE_PORT` | Port (8080) |

<details>
<summary>Struktúra</summary>

```
mcp-server/src/
├── http_server/  # mod.rs, handler.rs, response.rs, state.rs, config.rs
├── jobs/         # manager.rs, types.rs
├── chunking/     # markdown.rs, text.rs
└── tools/        # auto_embed.rs
```
</details>

<details>
<summary>Windows Service</summary>

Fixes: `PROGRAMDATA` · Tcpip dependency · `wait_hint=30s` · `collection_exists()` check

Telepítés: `mcp-server/docs/windows-service.md`
</details>

## Server Kezelés

| Művelet | Parancs |
|---------|---------|
| Graceful stop | `kill -SIGTERM <pid>` (vár 60s) |
| Force kill | `kill -SIGKILL <pid>` (KERÜLENDŐ!) |
| Restart | `pkill -TERM mcp-ironbase && sleep 60 && ./server --db ...` |

**SIGKILL → Dirty shutdown → indexek újraépítése (LASSÚ!)**

Systemd: `TimeoutStopSec=60`, `KillSignal=SIGTERM`
- `mark_clean_shutdown()` - Drop-ban hívódik

## Testing

| Típus | Parancs/Hely |
|-------|--------------|
| Rust unit | `cargo test -p ironbase-core` |
| Property | `ironbase-core/tests/property_tests.rs` |
| Python | `test_*.py`, `run_all_tests.py` |
| C# | `IronBase.NET/src/IronBase.Tests/` |
| MCP | `cd mcp-server && cargo test` |

**MemoryStorage teszt:** `DatabaseCore::<MemoryStorage>::open_memory()` (10-100x gyorsabb)

---

## Quick Reference

### Alapvető műveletek

```rust
// Dot notation - mindenhol működik
coll.find(&json!({"address.city": "NYC"}))?;
coll.update_one(&json!({"name": "X"}), &json!({"$set": {"a.b": "Y"}}))?;

// Compound index
coll.create_compound_index(vec!["country".into(), "city".into()], false)?;

// Upsert
let opts = UpdateOptions::new().with_upsert(true);
coll.update_one_with_options(&filter, &update, opts)?;
```

<details>
<summary>Upsert részletek</summary>

**Filter → Doc konverzió:** `{"email": "x"}` → doc-ba, `{"$gt": ...}` → ignorálva

**Korlátozások:** `update_many` NEM támogatja · Auto-embed OK upsert-nél
</details>

### Keresés

| Típus | API | Megjegyzés |
|-------|-----|------------|
| **Fuzzy** | `{"$fuzzy": "john"}` | jaro_winkler/levenshtein/damerau, threshold: 0.8 |
| **Text** | `{"$text": "király"}` | Tokenizáció + stemming (HU/EN/DE), AND logika |
| **StartsWith** | `{"$startsWith": "Al"}` | Prefix match, case-insensitive default |
| **EndsWith** | `{"$endsWith": ".hu"}` | Suffix match, case-insensitive default |
| **Contains** | `{"$contains": "Rust"}` | Substring match, case-insensitive default |
| **Fulltext** | `fulltext_search(field, query, limit)` | TF-IDF, stemming, HU/EN/DE |
| **RAG** | `rag_search(collection, query)` | FastText + HNSW |
| **Hybrid** | `hybrid_search(collection, query)` | RRF score fusion (MCP tool) |

`$text`, `$startsWith`, `$endsWith`, `$contains` mindegyik támogatja:
- Egyszerű forma: `{"field": {"$op": "value"}}` (case-insensitive)
- Bővített forma: `{"field": {"$op": {"$value"/"$search": "...", "$caseSensitive": true}}}`
- Array mezők: bármely elem match → true

<details>
<summary>Fulltext példa</summary>

```rust
coll.create_fulltext_index("content".into(), "hungarian", None, None)?;
let results = coll.fulltext_search("content", "király", Some(10), None, None, None)?;
```
</details>

<details>
<summary>RAG MCP Tools</summary>

| Tool | Leírás |
|------|--------|
| `rag_collection_create` | Collection + FastText model |
| `rag_document_import` | Auto-chunked import |
| `rag_search` | Semantic search |
| `rag_collection_stats` | Statisztikák |

Storage: `_rag/` dir · Perf: ~1-5ms search/10K chunks
</details>

<details>
<summary>Score Fusion Architektúra (2026-01-30)</summary>

**Döntés: Score fusion MCP tool szinten marad, NEM query operátor.**

**Indoklás:**
- Query operátorok (`OperatorMatcher`) = stateless boolean predikátumok: `fn(doc_value, filter_value) -> bool`
- Score fusion = ranked retrieval: score-okat ad vissza, nem igaz/hamis
- Index hozzáférés szükséges (fulltext + HNSW), de operátorok stateless-ek

**Meglévő implementáció:**

| Tool | Fájl | Algoritmus |
|------|------|-----------|
| `hybrid_search` | `mcp-server/src/tools/hybrid.rs` | RRF fusion |
| `rag_search` | `mcp-server/src/tools/rag.rs` | RRF fusion + auto-embed |

**RRF formula:** `score = Σ(weight_i / (60 + rank_i))`

**Reranking pipeline (multiplicatív boost):**
- Exact phrase match: 1.3x
- Keyword density: 1.0-1.1x
- Short content penalty: 0.8x

**Eredmény metadata:**
```json
{
  "_rrf_score": 0.032,
  "_final_score": 0.041,
  "_rerank_boost": 1.3,
  "_vector_rank": 2,
  "_text_rank": 5,
  "_vector_score": 0.89,
  "_text_score": 12.4
}
```

**Rétegek:**
```
Query operátorok ($text, $fuzzy, $regex...)  → boolean predikátum, per-doc
Collection metódusok (fulltext_search...)    → scored results, index-alapú
MCP tools (hybrid_search, rag_search)       → score fusion, ranked retrieval
```
</details>

### Auto-Embedding & Cache

```json
{"name": "auto_embed_enable", "arguments": {
  "collection": "articles", "source_field": "content",
  "target_field": "content_embedding", "provider": "fasttext"
}}
{"name": "embed_cache_stats"}

// Clear cache
{"name": "embed_cache_clear"}
```

Jobs: `embed_job_list`, `embed_job_status`, `embed_job_cancel` · Lifecycle: Pending→Running→Done

### $** Wildcard

`{"$**.name": "Alice"}` - mező keresése BÁRMILYEN mélységben (collection scan, MAX_DEPTH=100)

---

## Release & Dependencies

**Verzió frissítés (KÖTELEZŐ):**
- `mcp-server/Cargo.toml` → `1.0.XX`
- `Cargo.toml` (workspace) → `0.3.X`

**CI/CD:** Push → auto-tag.yml → release.yml → Win/Linux/macOS build

**Ellenőrzés:** `gh run list --limit 5` · `gh release list --limit 3`

**Dependencies:** serde, parking_lot, pyo3, maturin, ahash/dashmap, thiserror

**MCP cím:** 192.168.0.136:8080

<details>
<summary>Hot Backup</summary>

```bash
ironbase-backup backup --db /path/to.mlite --output ./backups --full
ironbase-backup restore --dir ./backups --output /path/to/restored.mlite
```
Lock-free: append-only → `data_end_offset`-ig immutable
</details>

---

## Durability & Performance

| Mode | fsync | ops/sec | Crash loss |
|------|-------|---------|------------|
| **Safe** | ✅ | 1-5K | 0 |
| **Batch** | ✅ | 20-50K | Max N |
| **Unsafe** | ❌ | 50-100K | All |

| Művelet | MemoryStorage |
|---------|---------------|
| INSERT batch | ~200K/sec |
| FIND indexed | ~500K/sec |
| AGGREGATION | ~100K docs/sec |

Benchmark: `cargo test -p ironbase-core --release speed_benchmark_full_suite -- --nocapture --ignored`