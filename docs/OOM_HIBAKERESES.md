# OOM Hibakeresési Útmutató

## Tünetek és Diagnosztika

### 1. Startup OOM - WAL Recovery

**Tünet:**
```
[STARTUP/DB] StorageEngine opened, recovering WAL...
[CRASH / OOM killed]
```

**Ok:** WAL fájl túl nagy (pl. 29GB) - Safe módban nem ürült.

**Diagnosztika:**
```bash
ls -lh /path/to/database.mlite.wal
```

**Megoldás:**
1. Backup az mlite fájlról
2. Töröld a .wal fájlt
3. Újraindítás

**Megelőzés:** Fix implementálva - `wal.clear()` minden 100 commit után.

---

### 2. Aggregation OOM

**Tünet:**
```
memory allocation of XXXXX bytes failed
```

**Ok:** Pipeline túl sok dokumentumot / csoportot tölt memóriába.

**Diagnosztika - Query elemzés:**

| Kérdés | Probléma ha... |
|--------|----------------|
| Van `$match` a pipeline elején? | Nincs → összes doc betöltődik |
| Van `$limit`? | Nincs → korlátlan output |
| `$group` hány egyedi kulcsot generál? | >50K → memória probléma |
| `$unwind` van? | Több is → kumulatív limit |
| `$push`/`$addToSet` van `$group`-ban? | Tömb mérete korlátlan? |

**Megoldás:**
```rust
// Használj dinamikus limiteket
let results = collection.aggregate_auto(&pipeline)?;

// Vagy explicit memory budget
let limits = AggregationLimits::with_memory_budget(256); // 256 MB
let results = collection.aggregate_with_limits(&pipeline, limits)?;
```

---

### 3. Find OOM - Nagy Response

**Tünet:**
```
Response size limit exceeded: loaded X documents (Y bytes)...
```

**Ok:** Túl sok/nagy dokumentum a response-ban.

**Diagnosztika:**
```rust
// Ellenőrizd a dokumentum méreteket
let sample = collection.find_one(&query)?;
println!("Sample doc size: {} bytes", serde_json::to_string(&sample)?.len());
```

**Megoldás:**
```rust
// Használj limitet és projekciót
let options = FindOptions::with_safe_defaults()
    .with_limit(100)
    .with_projection(hashmap!{"_id" => 1, "name" => 1});
```

---

### 4. Index Rebuild OOM

**Tünet:**
- `create_fulltext_index()` crash nagy collection-nél
- `rebuild_indexes_from_catalog()` crash backup restore-nál

**Ok:** Összes dokumentum egyszerre betöltődik indexeléshez.

**Diagnosztika:**
```bash
# Collection méret
echo 'db.collection.count()' | mcp-client
```

**Megoldás:** Batching implementálva - ellenőrizd hogy a legújabb verzió fut.

---

### 5. Count OOM - Collection Scan

**Tünet:**
- `count_documents()` timeout (300s+)
- Memória folyamatosan nő count közben

**Ok:** Nincs megfelelő index, collection scan fut.

**Diagnosztika:**
```rust
// Explain a query-t
let plan = collection.explain(&query)?;
println!("{:?}", plan);  // IndexScan vs CollectionScan
```

**Megoldás:**
1. Hozz létre indexet a filter mezőre
2. Sparse index `$exists` query-khez:
```rust
collection.create_index("field", false, true)?; // sparse=true
```

---

## Memória Monitorozás

### Runtime Stats

```rust
// MCP health endpoint
GET /health

// Válasz tartalmazza:
{
  "memory": {
    "used_mb": 1234,
    "available_mb": 5678,
    "usage_percent": 17.8
  }
}
```

### Linux Parancsok

```bash
# Process memória
ps -o pid,rss,vsz,comm -p $(pgrep mcp-ironbase)

# Rendszer memória
free -h

# Top memória fogyasztók
top -o %MEM

# OOM killer log
dmesg | grep -i "killed process"
```

### Jemalloc Stats (Unix)

```rust
// Ha tikv-jemalloc engedélyezve
use tikv_jemalloc_ctl::{stats, epoch};

epoch::advance().unwrap();
let allocated = stats::allocated::read().unwrap();
let resident = stats::resident::read().unwrap();
println!("Allocated: {} MB", allocated / 1024 / 1024);
println!("Resident: {} MB", resident / 1024 / 1024);
```

---

## Korábbi OOM Bugok Referencia

| Commit | Bug | Fix |
|--------|-----|-----|
| `4904ccc9` | `scan_documents_via_catalog()` összes doc | Streaming |
| `567e0d11` | aggregation összes doc memóriában | Limitek |
| `e0001bbe` | `count_with_scan` párhuzamos | Chunked parallel |
| `49f27a77` | `update_one` bulk load | Streaming |
| `88f0a79c` | `update_many` bulk load | Streaming |
| `e445b44e` | range_query + Top-K | Egységesítés |
| `2026-01-11` | WAL unbounded growth (29GB) | Periodikus clear |
| `a54f29a1` | Sparse index üres tömb | `[]` kezelés fix |

---

## Emergency Workarounds

### 1. WAL Törlés (adatvesztés kockázat!)
```bash
# CSAK ha a szerver nem indul WAL OOM miatt
cp database.mlite database.mlite.backup
rm database.mlite.wal
# Újraindítás
```

### 2. Memory Limit Növelés (Docker)
```yaml
services:
  ironbase:
    mem_limit: 4g
    memswap_limit: 8g
```

### 3. Swap Engedélyezés (Linux)
```bash
sudo fallocate -l 4G /swapfile
sudo chmod 600 /swapfile
sudo mkswap /swapfile
sudo swapon /swapfile
```

### 4. OOM Killer Prioritás
```bash
# Csökkentsd az OOM score-t (kevésbé valószínű kill)
echo -500 > /proc/$(pgrep mcp-ironbase)/oom_score_adj
```
