# CLAUDE.md

**IronBase** - MongoDB-kompatibilis embedded NoSQL (Rust + Python/C# bindings)

| Jellemző | Érték |
|----------|-------|
| Tesztek | 2,000+ (unit + integration + doctest) |
| API-k | Rust, Python (PyO3), C# (.NET 8) |
| Operátorok | 25 query, 7 update |
| Indexek | B+ tree, compound, fuzzy, fulltext, HNSW |
| Keresés | Fuzzy (Jaro-Winkler/Levenshtein), BM25 fulltext, RAG |

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
| **Query (25)** | $eq $ne $gt $gte $lt $lte $in $nin · $and $or $not $nor · $exists $type · $all $elemMatch $size · $regex $fuzzy $text · $startsWith $endsWith $contains · $expr · $** |
| **Update (7)** | $set $inc $unset $push $pull $addToSet $pop (+ dot notation + upsert) |
| **Aggregation** | $match $group $project $count $sort $limit $skip $unwind · Accumulators: $sum $avg $min $max $first $last $push $addToSet · $group nested object _id |

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
| HNSW | ❌ Nem (lazy rebuild ✅) | Gráf struktúra miatt komplex, de orphan compaction checkpoint/compact-ban |

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

### HNSW Orphan Management (512990a6, #53)

**Probléma:** HNSW `remove()` lazy — csak `id_to_index`-ből töröl, az orphan node a `self.nodes` Vec-ben marad. Delete+reimport ciklusok halmozzák az orphan-okat → memória nő, `vector_count` felfújódik.

**Három rétegű fix:**

| Réteg | Változás | Hatás |
|-------|----------|-------|
| `batch_update_indexes()` | HNSW kezelés hozzáadva (mod.rs:1931+) | `update_many` frissíti a vektorokat |
| `len()` / `is_empty()` | `id_to_index.len()` (nem `nodes.len()`) | Pontos aktív vektor szám |
| Orphan rebuild | Checkpoint + compact integráció | Memória felszabadítás |

**API:**
```rust
index.len()            // Aktív vektorok (id_to_index)
index.total_nodes()    // Összes node (beleértve orphan-okat)
index.orphan_count()   // total_nodes - len
index.needs_rebuild()  // >30% orphan ÉS >100 orphan
index.rebuild_if_needed()  // Rebuild ha needs_rebuild()
```

**Orphan compaction — gated (automatikus) vs force (explicit):**
- **Checkpoint** (60s): `rebuild_vector_indexes_if_needed()` — csak ha >30% ÉS >100 orphan
- **Automatikus compact** (bloat-trigger, induláskor: `last_compact_size=0` → `bloat_ratio=inf`):
  `force_vector_rebuild=false` → `rebuild_vector_indexes_if_needed()` (orphan-gated, **olcsó**).
  ⚠️ Ezért az automatikus/induló compact **NEM** javít degradált-de-orphan-mentes
  (vagy <100 orphan) gráfot — ez szándékos (különben minden restart teljes,
  egyszálú HNSW rebuild-et fizetne 0 orphannál is, ~15-20 perc 1 mag pinned).
- **Explicit compact** (MCP `db_compact` tool, blokkoló `DatabaseCore::compact()`,
  Python/C# `db.compact()`): `force_vector_rebuild=true` → `rebuild_all_vector_indexes()`
  — minden orphan eltávolítása + teljes gráf-rekonstrukció. Degradált gráf
  javításához EZT kell hívni (pl. régi/buggos verzió után). Szemantika: *explicit hívás = force*.
- Flag: `CompactionConfig::force_vector_rebuild` (default `false`). Belépési pontok:
  `mcp-server/src/compaction.rs` auto path → `false`, `tools/admin.rs` db_compact → `true`.

**Key files:**
- `ironbase-core/src/vector/hnsw.rs` — `len()`, `orphan_count()`, `needs_rebuild()`, `rebuild_if_needed()`
- `ironbase-core/src/index/manager.rs` — `rebuild_vector_indexes_if_needed()`, `rebuild_all_vector_indexes()`
- `ironbase-core/src/database/maintenance.rs` — checkpoint + compact integráció
- `ironbase-core/src/collection_core/mod.rs` — `batch_update_indexes()` HNSW szekció

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

Korábbi OOM hibák: `4904ccc9`, `567e0d11`, `e0001bbe`, `49f27a77`, `88f0a79c`, `e445b44e`.

A kritikus bugok teljes katalógusa (tünet → root cause → fix, #49–#69, WAL/HNSW/fulltext/btree)
→ **[`docs/DOCUMENTED-BUGS.md`](docs/DOCUMENTED-BUGS.md)**. Részletes elemzések: `memory/critical-bugs.md`.

### Checkpoint Per-Index Flush (2026-02-01)

**Probléma:** A 60s-os periodikus checkpoint az `IndexManager` write lock-ot tartva flush-ölte az ÖSSZES dirty indexet. Az `emails` collection-nél (22 index, ~26K `file.flush()` syscall) ez percekig tartott alacsony RAM mellett, blokkolva minden `insert_one`-t.

**Megoldás:** `flush_all_indexes_counted()` átírva per-index lock/unlock logikára.

```
Régi: 1 write lock → flush 22 index → unlock (percek)
Új:   read lock → dirty nevek → unlock
      for each dirty index:
          write lock → flush 1 index → unlock (ms)
```

**Race condition:** Ha insert befut két flush között és dirty-re állít egy már flush-ölt indexet, az a KÖVETKEZŐ checkpoint-ban lesz kiírva. Adatvesztés nincs (WAL tartalmazza).

**Key files:**
- `ironbase-core/src/index/manager.rs` — `dirty_*_index_names()`, `flush_one_*_index()`
- `ironbase-core/src/database/maintenance.rs` — `flush_all_indexes_counted()`

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
| `MCP_PORT` | Port (8080) |

**FONTOS: Anthropic API schema korlátozás**
- `inputSchema` top-level szintjén TILOS `oneOf`, `allOf`, `anyOf` használata
- Nested (property-n belüli) használat megengedett
- Ha egy tool-nak alternatív mezőkre van szüksége (pl. `field` VAGY `fields`), a description-ben jelezd és server-side validáld
- Teszt: `test_no_top_level_oneof_allof_anyof` (`definitions/mod.rs`)

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

### RAG Pipeline — mérnöki áttekintés

**Egységes referencia:** [`docs/RAG_PIPELINE.md`](docs/RAG_PIPELINE.md) — chunkolás,
contextual embedding, idempotens import, multi-field FTS, hibrid keresés,
adjacent-chunk merge, konzisztencia/visszamenőleges kompatibilitás, operatív
ajánlások, verzió-szerinti hozzájárulások (v1.0.494–v1.0.543). Az #67/#64/#65/#63/#66
issue-k lezárva; a **`context_fields` dokumentum-identitás kontextuális embedding (#117,
v1.0.543)** dokumentálva — részletek a doksiban + a CHANGELOG `[Unreleased]` szekcióban.

### Keresés

| Típus | API | Megjegyzés |
|-------|-----|------------|
| **Fuzzy** | `{"$fuzzy": "john"}` | jaro_winkler/levenshtein/damerau, threshold: 0.8 |
| **Text** | `{"$text": "király"}` | Tokenizáció + stemming (HU/EN/DE), AND logika |
| **StartsWith** | `{"$startsWith": "Al"}` | Prefix match, case-insensitive default |
| **EndsWith** | `{"$endsWith": ".hu"}` | Suffix match, case-insensitive default |
| **Contains** | `{"$contains": "Rust"}` | Substring match, case-insensitive default |
| **Fulltext** | `fulltext_search(field, query, limit)` | BM25 (k1=1.2, b=0.75), stemming, HU/EN/DE |
| **Hybrid** | `search(collection, query)` | Intent-only document-anchored RRF retrieval, server-owned fusion, auto-embed |

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
<summary>BM25 Scoring (29e2ef8c, #58)</summary>

**Formula:** `score = IDF * ((k1+1) * tf) / (k1 * (1 - b + b * dl/avgdl) + tf)`

| Paraméter | Érték | Jelentés |
|-----------|-------|----------|
| k1 | 1.2 | TF szaturáció (magasabb = lineárisabb TF hatás) |
| b | 0.75 | Doc length normalizáció (0=nincs, 1=teljes) |
| dl | per-doc | Dokumentum token száma |
| avgdl | globális | Átlagos dokumentumhossz (total_doc_length / doc_count) |

**Memória:** `doc_lengths: HashMap<DocumentId, u32>` + `total_doc_length: u64` per index.

**Backward compat:** `#[serde(default)]` → régi .ftidx fájlok betöltésekor üres doc_lengths → fallback TF-saturation-only. `rebuild_doc_lengths_if_needed()` startup migration.

**Perzisztálás:** `FulltextIndexMetadataForSave`-ben `Vec<(DocumentId, u32)>` (JSON DocumentId kulcs compat).

**Key file:** `ironbase-core/src/fulltext.rs`
</details>

<details>
<summary>RAG MCP Tools</summary>

| Tool | Leírás |
|------|--------|
| `rag_collection_create` | Collection + vector + fulltext indexes |
| `rag_document_import` | Auto-chunked import |
| `rag_collection_stats` | Statisztikák |
| `search` | Unified intent-only hybrid retrieval (document-anchored, auto-embed) |

Storage: `_rag/` dir · Perf: ~1-5ms search/10K chunks
</details>

<details>
<summary>Rhai Scripting DB Functions</summary>

A Rhai script engine-ben elérhető adatbázis függvények (script_exec / script_run):

| Függvény | Leírás |
|----------|--------|
| `db_find(collection, query)` | Dokumentumok keresése (`#{documents: [...], count: n}`) |
| `db_find_one(collection, query)` | Első találat |
| `db_insert_one(collection, doc)` | Beszúrás |
| `db_update_one(collection, filter, update)` | Frissítés (első találat) |
| `db_update_many(collection, filter, update)` | Frissítés (összes találat) |
| `db_delete_one(collection, filter)` | Törlés (első találat) |
| `db_delete_many(collection, filter)` | Törlés (összes találat) |
| `db_count(collection, query)` | Dokumentumok számolása |
| `db_aggregate(collection, pipeline)` | Aggregációs pipeline |
| `db_hybrid_search(collection, query)` | RRF hybrid keresés (fusion.rs delegálás) |
| `db_hybrid_search(collection, query, options)` | Hybrid keresés opciókkal |
| `db_rag_import(collection, text, metadata)` | RAG dokumentum import |
| `db_rag_create(collection)` / `db_rag_create(collection, options)` | RAG collection létrehozás |
| `db_rag_stats(collection)` | RAG statisztikák |

**`db_hybrid_search` opciók** (Rhai Map):
```rhai
let results = db_hybrid_search("kb", "keresett szöveg", #{
    limit: 10,              // Max eredmények (default: 10)
    rrf_k: 20.0,            // RRF K konstans (default: 20)
    rerank: true,            // Reranking (phrase 1.5x, density 1.3x, title boost)
    deduplicate: false,      // MMR diversity reranking (default: false)
    mmr_lambda: 0.7,         // MMR lambda (1.0=relevance, 0.0=diversity, default: 0.7)
    merge_chunks: true,      // Szomszédos chunk összevonás
    match_scope: "document", // "document" (default) | "chunk" — csak mode="and" mellett él
    search_mode: "balanced", // "balanced" | "semantic" | "keyword"
    vector_weight: 0.5,      // Explicit vektor súly (felülírja search_mode-ot)
    fulltext_weight: 0.5,    // Explicit fulltext súly
    title_field: "title",    // Cím mező reranking boost-hoz
    text_fields: ["content", "title"], // Multi-field fulltext
    mode: "or",              // Fulltext mode: "or" (default, diszjunktív) | "and" (opt-in precízió)
    group_by_document: true, // Dokumentumok szerinti csoportosítás (default: false)
    filter: #{ year: 2026 }, // Dokumentum szűrő
});
```

**Key files:** `mcp-server/src/scripting/db_functions.rs` (registration + impl), `mcp-server/src/tools/fusion.rs` (shared pipeline)
</details>

### Score Fusion / hibrid keresés architektúra

RRF fusion, reranking, adjacent-chunk merge, MMR, fulltext mode (OR default v1.0.537+),
document-level AND, multi-field FTS, search-mode presetek, közös `fusion.rs`/`hybrid.rs` motor.
→ **[`docs/SCORE_FUSION.md`](docs/SCORE_FUSION.md)** (teljes spec). Operatív: [`docs/RAG_PIPELINE.md`](docs/RAG_PIPELINE.md).

### Auto-Embedding & Cache

```json
{"name": "auto_embed_enable", "arguments": {
  "collection": "articles", "source_field": "content",
  "target_field": "content_embedding", "provider": "ollama"
}}
{"name": "embed_cache_stats"}

// Clear cache
{"name": "embed_cache_clear"}
```

Jobs: `embed_job_list`, `embed_job_status`, `embed_job_cancel` · Lifecycle: Pending→Running→Done

### Model & Preprocessing Change Detection

**Mechanizmus:** `AutoEmbeddingConfig`-ban tárolt `model` és `preprocessing_version` mezők. Startup detekció (`check_model_changes_and_reembed()`) összehasonlítja a tárolt értékeket az élő provider által visszaadottakkal — eltérés → automatikus re-embed.

**Startup logika:**

| Helyzet | Mi történik |
|---------|-------------|
| Legacy config (mindkettő üres) | Mentés, nincs re-embed |
| Modell változott | Force re-embed |
| Preprocessing változott | Force re-embed |
| Dimenzió változott | HNSW rebuild + re-embed |
| `--force-reembed` flag | Force re-embed mindig |

**CLI:**
```bash
./mcp-ironbase-server --force-reembed              # HTTP mód
./mcp-ironbase-server --stdio --force-reembed      # stdio mód
```

**Key files:**
- `ironbase-core/src/storage/mod.rs` — `AutoEmbeddingConfig::preprocessing_version`
- `mcp-server/src/embedding/mod.rs` — `EmbeddingProvider::preprocessing_version()` trait default = `"default"`
- `mcp-server/src/tools/auto_embed.rs` — `check_model_changes_and_reembed()`, `handle_auto_embed_enable()`

### $** Wildcard

`{"$**.name": "Alice"}` - mező keresése BÁRMILYEN mélységben (collection scan, MAX_DEPTH=100)

### $group Nested Object _id (305e6c91)

**MongoDB-kompatibilis multi-dimenzionális csoportosítás.**

```rust
// Nested object _id: több mező szerinti csoportosítás
coll.aggregate(&json!([
    {"$group": {"_id": {"year": "$year", "type": "$type"}, "count": {"$sum": 1}}}
]))?;
// Output: [{"_id": {"year": 2024, "type": "A"}, "count": 5}, ...]

// Nested object $push/$addToSet értékek
coll.aggregate(&json!([
    {"$group": {"_id": "$category", "items": {"$push": {"title": "$title", "year": "$year"}}}}
]))?;
```

| Syntax | GroupId | Index opt. |
|--------|---------|-----------|
| `{_id: null}` | Null | N/A |
| `{_id: "$field"}` | Field | ✅ |
| `{_id: {$substr: [...]}}` | Substring | ❌ |
| `{_id: {year: "$year", type: "$type"}}` | **Object** | ❌ |

**Megkülönböztetés:** `$`-ral kezdődő kulcs = operátor, egyébként = nested object field referencia.

**ValueExpression::Object** rekurzív: `$push`/`$addToSet` értékei lehetnek Field, Substr, vagy nested Object.

**Hiányzó mező → `null`** az output _id-ben (MongoDB-kompatibilis viselkedés).

**Key files:**
- `ironbase-core/src/aggregation/types.rs` — `GroupId::Object`, `ValueExpression::Object`
- `ironbase-core/src/aggregation/stages/group_stage.rs` — parsing, hash extraction
- `ironbase-core/src/aggregation/helpers.rs` — `parse_value_expression()` nested object
- `ironbase-core/src/aggregation/stages/accumulator.rs` — `evaluate_value_expr()` Object

---

## Release & Dependencies

**Verzió frissítés (KÖTELEZŐ):**
- `mcp-server/Cargo.toml` → `1.0.XX`
- `Cargo.toml` (workspace) → `0.3.X`

**CI/CD:** Push → auto-tag.yml → release.yml → Win/Linux/macOS build

**Ellenőrzés:** `gh run list --limit 5` · `gh release list --limit 3`

**Dependencies:** serde, parking_lot, pyo3, maturin, ahash/dashmap, thiserror

**MCP cím:** 172.19.144.1:8080

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