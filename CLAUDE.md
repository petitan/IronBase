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

**Automatikus orphan compaction:**
- **Checkpoint** (60s): `rebuild_vector_indexes_if_needed()` — csak ha >30% orphan
- **Compact** (`db_compact`): `rebuild_all_vector_indexes()` — minden orphan eltávolítása

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
| **Checkpoint lock contention** | 9e4499b4 | insert_one 14+ perc blokk | `flush_all_indexes_counted()` 1 lock / 22 index | Per-index flush: 22 lock / 1 index |
| **Btree delete not dirty** | 265f0c2e | count_documents ~3x túlszámolás | `remove_document_from_indexes` + `batch_update_indexes` nem jelölte dirty-nek a btree indexet → checkpoint nem mentette a törléseket → stale .idx | `dirty_btree_indexes.insert()` 5 helyen |
| **Fulltext candidate limit** | aa3b8ed5 | Filtrált fulltext 0 eredmény gyakori szóra | `calculate_candidate_limit()` max 300 jelöltet kért, de pl. "ajánlat" 6766 match-ből a year=2026 dokuk a 6735+ pozíción voltak | Filter esetén 100K cap (TF-IDF amúgy is O(N), limit csak output-ot csonkít) |
| **Fulltext empty collection reject** | #49 | `rag_collection_create` üres collection-re nem hoz létre fulltext indexet | `search.rs:498` validáció `num_documents == 0` → error + cleanup, üres collection is triggereli | `live_document_count == 0` check: üres collection → üres index valid |
| **Vector count stale metadata** | #50 | `vector_count: 0` a stats-ban működő HNSW index mellett | `VectorIndexMetadata.vector_count` csak creation-kor íródik, auto-indexing nem frissíti | `list_vector_indexes()` az in-memory HNSW `len()`-ből frissíti a clone-t |
| **HNSW orphan accumulation** | 512990a6, #53 | `vector_count` 2x a valós után delete+reimport; növekvő memória | `batch_update_indexes()` nem kezelte HNSW-t + `remove()` lazy (orphan node marad) + `len()` orphan-okat is számolta | HNSW kezelés `batch_update_indexes`-ben + `len()` = `id_to_index.len()` + orphan rebuild checkpoint/compact-ban |
| **Fulltext flush dirty flag** | ed9016d3, #54 | fulltext_search dokumentumok nagy része nem kereshető restart után | `commit_fulltext_flush()` feltétel nélkül törölte a dirty flag-et → Phase 2 alatti concurrent insert-ek elvesztek | `has_pending_entries()` check: dirty flag csak akkor törlődik ha `inverted_index` üres |
| **HNSW rebuild duplicate ID** | d83885bf, #55 | `db_compact` "ID already exists in vector index" hiba | `rebuild()` `nodes` Vec-ből iterált — remove+reinsert után orphan+aktív node ugyanazzal az ID-vel, mindkettő átment a `contains_key` filter-en | `id_to_index` HashMap-ből iterálás (egyedi kulcsok, mindig a helyes node) |
| **BM25 fails on 4+ words** | cd70eae4, #61 | `hybrid_search` text_score=0 multi-word query-knél | `fulltext_search` OR default vs `hybrid_search` AND default inkonzisztencia + chunk-level AND túl szigorú (1 chunk ritkán tartalmaz 4+ stemelt tokent) | Konzisztens AND default mindkettőnél + `match_scope` default "document" (dok szintű AND kvalifikáció, chunk szintű OR retrieval) |
| **RAG import not idempotent** | #67 | `rag_document_import` retry-nál duplikált chunkok (éles: ~11K duplikátum) | `insert_many` előtt nincs delete-by-doc_id → ugyanazon doc_id újra-importja hozzáfűz | `if_exists` param (default `replace`) + közös `insert_chunks_idempotent` helper (`helpers.rs`) mind a 3 import-úton (rag/embed_document/db_rag_import). Safe ordering: capture old _ids (proj `_id`) → insert new → delete old → sosem veszít adatot |

**Részletes bug elemzések:** Lásd `memory/critical-bugs.md` (Lazy Index, read_data boundary, Fulltext candidate limit, Fulltext flush dirty flag, Btree delete not dirty, Workaround-ok)

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

### Keresés

| Típus | API | Megjegyzés |
|-------|-----|------------|
| **Fuzzy** | `{"$fuzzy": "john"}` | jaro_winkler/levenshtein/damerau, threshold: 0.8 |
| **Text** | `{"$text": "király"}` | Tokenizáció + stemming (HU/EN/DE), AND logika |
| **StartsWith** | `{"$startsWith": "Al"}` | Prefix match, case-insensitive default |
| **EndsWith** | `{"$endsWith": ".hu"}` | Suffix match, case-insensitive default |
| **Contains** | `{"$contains": "Rust"}` | Substring match, case-insensitive default |
| **Fulltext** | `fulltext_search(field, query, limit)` | BM25 (k1=1.2, b=0.75), stemming, HU/EN/DE |
| **Hybrid** | `hybrid_search(collection, query)` | RRF score fusion, auto-embed ha nincs vector |

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
| `hybrid_search` | Unified keresés: explicit vector VAGY auto-embed |

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
    match_scope: "document", // "chunk" (default) | "document" (mode=and + RAG)
    search_mode: "balanced", // "balanced" | "semantic" | "keyword"
    vector_weight: 0.5,      // Explicit vektor súly (felülírja search_mode-ot)
    fulltext_weight: 0.5,    // Explicit fulltext súly
    title_field: "title",    // Cím mező reranking boost-hoz
    text_fields: ["content", "title"], // Multi-field fulltext
    mode: "and",             // Fulltext mode: "and" (default) | "or"
    group_by_document: true, // Dokumentumok szerinti csoportosítás (default: false)
    filter: #{ year: 2026 }, // Dokumentum szűrő
});
```

**Key files:** `mcp-server/src/scripting/db_functions.rs` (registration + impl), `mcp-server/src/tools/fusion.rs` (shared pipeline)
</details>

<details>
<summary>Score Fusion Architektúra (2026-01-30)</summary>

**Döntés: Score fusion MCP tool szinten marad, NEM query operátor.**

**Indoklás:**
- Query operátorok (`OperatorMatcher`) = stateless boolean predikátumok: `fn(doc_value, filter_value) -> bool`
- Score fusion = ranked retrieval: score-okat ad vissza, nem igaz/hamis
- Index hozzáférés szükséges (fulltext + HNSW), de operátorok stateless-ek

**Implementáció (2026-02-18 — unified):**

| Tool | Fájl | Algoritmus |
|------|------|-----------|
| `hybrid_search` | `mcp-server/src/tools/hybrid.rs` | RRF fusion (explicit vector VAGY auto-embed) |

**RRF formula:** `score = Σ(weight_i / (K + rank_i))` ahol K=20 (default, konfigurálható `rrf_k` paraméterrel)

**Reranking pipeline (multiplicatív boost):**
- Exact phrase match: 1.5x
- Keyword density: 1.0-1.3x
- Title match: 1.0-1.5x (ha `title_field` megadva)
- Short content penalty: 0.8x (<50 char)

**Adjacent chunk merge (afd45313, STEP 5.5):**
- RAG chunking overlap (~100 char) → szomszédos chunkok duplikálják a határszöveget → top-K helyet pazarolnak
- `merge_chunks`: `true` (default) — same `doc_id`, consecutive `chunk_index` → összevonás
- Szöveg: `start_char`/`end_char` alapján overlap levágás, UTF-8 safe
- Score: max(final_score) a futamból, embedding: legjobb score-ú chunk
- Metadata: `chunk_merged: true`, `chunks_in_merge: N`, frissített `start_char`/`end_char`/`chunk_index`
- Elhelyezés: reranking UTÁN, MMR ELŐTT
- Response: `"chunks_merged": N`

**MMR diversity reranking (deduplication):**
- Algoritmus: `mmr(c) = λ * relevance(c) - (1-λ) * max_sim(c, selected)`
- `mmr_lambda`: 1.0 = pure relevance, 0.0 = pure diversity, 0.7 = relevance-favoring (default)
- `deduplicate`: default `false` — hívó döntse el kell-e MMR dedup
- Cosine similarity: `ironbase_core::vector::simd::cosine_similarity()` (SIMD)
- Embedding nélküli doc-ok: relevance order (nincs diversity penalty)
- MMR skip-elve `group_by_document=true` esetén (limit dokumentum szinten alkalmazva)

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

**Fulltext mode paraméter (45f74bf7, #47, cd70eae4 #61):**
- `mode`: `"and"` (default) = MINDEN szó kell a dokumentumban, `"or"` = bármely szó elég (deprecated)
- Elérhető: `hybrid_search`, `fulltext_search` — **mindkettő AND default** (v1.0.389+, korábban fulltext_search OR volt)
- AND mód szűkíti a fulltext komponenst; vektor keresés változatlan → RRF fusion vektor-only eredményeket is ad
- `mode` hiánya = `"and"` (v1.0.375+)

| Fájl | Változás |
|------|----------|
| `params.rs` | `pub mode: Option<String>` mindkét struct-ban |
| `definitions/hybrid.rs` | `"mode"` schema entry (default: "and", "or" deprecated) |
| `definitions/index.rs` | `"mode"` schema entry (default: "and", v1.0.389+) |
| `hybrid.rs` | `and_mode: p.mode.as_deref() != Some("or")` |
| `index.rs` | `and_mode: p.mode.as_deref() != Some("or")` (v1.0.389+) |

**Document-level AND mode (0f208d41, #56, cd70eae4 #61):**
- `match_scope`: `"document"` (default, v1.0.389+) = szavak a dokumentum különböző chunkjaiban is lehetnek, `"chunk"` = minden szó egyetlen chunkban
- **Keresési logika:** kvalifikáció (AND) → minden query token megjelenik a dokumentumban; chunk retrieval (OR) → bármely tokent tartalmazó chunk visszajön
- Korábban default "chunk" volt, de 3+ szavas query-knél túl szigorú (egyetlen chunk ritkán tartalmaz minden tokent)
- Aktiválódik: `mode="and"` + `match_scope` != `"chunk"` (v1.0.389+, korábban csak explicit `match_scope="document"` vagy `group_by_document=true`)
- Algoritmus: posting list interszekcióval kvalifikálja a dokumentumokat, majd OR módban keres a kvalifikált doc_id-kre szűrve
- Vektor keresés NEM szűrt (RRF természetesen kezeli)
- Response: `"match_scope": "document"/"chunk"`, `"qualified_doc_ids": N`

| Réteg | Változás |
|-------|----------|
| `fulltext.rs` | `tokenize_query()`, `token_posting_count()`, `token_chunk_ids()` pub metódusok |
| `search.rs` | Delegáló metódusok a CollectionCore-on |
| `adapter.rs` | Adapter metódusok (DocumentId→Value konverzió) |
| `params.rs` | `pub match_scope: Option<String>`, `pub group_by_document: bool` |
| `definitions/hybrid.rs` | `"match_scope"` schema (enum: chunk/document), `"group_by_document"` schema |
| `hybrid.rs` | `qualify_documents()` fn + STEP 1.5 pipeline + STEP 7 grouped response |

```json
{"collection": "docs", "query": "Ifju János fékpad ár",
 "group_by_document": true}
```

**Multi-field fulltext (b938c487, #48):**
- `text_fields`: string tömb — több mező párhuzamos fulltext keresése, best-field strategy (max score merge)
- Elérhető: `hybrid_search` (a `fulltext_search` már korábban támogatta `fields` néven)
- `text_fields` felülírja a `text_field` (string) paramétert ha mindkettő megadva
- Előfeltétel: minden megadott mezőn fulltext index kell (`index_create_fulltext`)
- Backward compatible: `text_fields` hiánya = single-field (régi viselkedés)

```json
{"collection": "docs", "query": "Juhai ajánlat",
 "text_fields": ["content_text", "title", "customer"]}
```

**Search mode presets (220679f3):**
- `search_mode`: `"balanced"` (default), `"semantic"`, `"keyword"` — LLM-barát nevesített preset a numerikus weight-ek helyett
- Elérhető: `hybrid_search`
- Explicit `vector_weight`/`fulltext_weight` felülírja a preset-et ha megadva

| Mode | vector_weight | fulltext_weight | Mikor |
|------|--------------|-----------------|-------|
| `balanced` | 0.5 | 0.5 | Default, általános keresés |
| `semantic` | 0.8 | 0.2 | Fogalmi/konceptuális kérdések |
| `keyword` | 0.2 | 0.8 | Specifikus szó/kifejezés keresés |

Prioritás: explicit weights > search_mode preset > balanced default

**Shared fusion modul (febba776, afd45313):**
- `mcp-server/src/tools/fusion.rs` — közös reranking/fusion kód (FusedResult, rerank_results, merge_adjacent_chunks, mmr_reorder, apply_projection, id_to_string, strip_punctuation, extract_embedding)
- hybrid.rs importálja (rag.rs már csak thin wrapper, nem használ fusion kódot)

**hybrid_search pipeline (hybrid.rs):**
```
STEP 1: Resolve vector + field names (explicit vs auto-embed)
STEP 1.5: Document-level AND qualification             [match_scope=document OR group_by_document + mode=and]
STEP 2: Vector search → ranks (single-pass HashMap)
STEP 3: Fulltext search → ranks (single-pass HashMap, OR mode + doc_id filter if STEP 1.5)
STEP 4: RRF Fusion (drain-based, no cloning)
STEP 5: Reranking (phrase, density, title, length)    [rerank=true]
STEP 5.5: Adjacent chunk merge (overlap dedup)        [merge_chunks=true]
STEP 6: MMR diversity reranking                       [deduplicate=true, skip if group_by_document]
STEP 7: Projection + response
        ├─ Flat mode (default): chunks ordered by score, limit = chunk count
        └─ Grouped mode [group_by_document=true]:
           Phase 1: Top N doc_ids from fused results
           Phase 2: Single fulltext OR search filtered to N doc_ids → ALL chunks
           Group by doc_id, limit = document count
```

**`group_by_document` paraméter:**
- Default: `false` — flat chunk lista score sorrendben (hagyományos viselkedés)
- `true` — eredmények doc_id szerint csoportosítva, dokumentumonként MINDEN releváns chunk visszajön
- Limit szemantika: flat → max chunk szám, grouped → max dokumentum szám
- Automatikusan bekapcsolja document-level AND qualification-t (qualify_documents + OR mode)
- Dokumentum kiválasztás: AND (minden query szó benne van a dokumentumban)
- Chunk retrieval: OR (bármely query szó → releváns chunk)
- Phase 2: egyetlen fulltext OR search `doc_id $in` filterrel (nem N külön keresés)
- MMR deduplicate skip-elve grouped módban
- Response formátum:
```json
{
  "results": [
    {"doc_id": "abc", "best_score": 0.039, "chunk_count": 9, "chunks": [...]},
    {"doc_id": "def", "best_score": 0.028, "chunk_count": 4, "chunks": [...]}
  ],
  "count": 2,
  "total_chunks": 13,
  "group_by_document": true,
  "qualified_doc_ids": 359
}
```

**Rétegek:**
```
Query operátorok ($text, $fuzzy, $regex...)  → boolean predikátum, per-doc
Collection metódusok (fulltext_search...)    → scored results, index-alapú
MCP tools (hybrid_search)                    → score fusion, ranked retrieval
```
</details>

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