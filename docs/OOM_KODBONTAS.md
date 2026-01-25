# OOM-Biztos Kód Minták

## TILOS Minták ❌

### 1. Bulk Document Loading

```rust
// ❌ TILOS - Összes doc memóriában
let all_docs: Vec<Document> = doc_ids
    .iter()
    .map(|id| load_document(id).unwrap())
    .collect();

// ❌ TILOS - Catalog collect
let docs: Vec<_> = catalog.iter().collect();

// ❌ TILOS - Map + Collect nagy collection-re
let results: Vec<Value> = storage
    .scan_all()
    .map(|bytes| serde_json::from_slice(&bytes).unwrap())
    .collect();
```

### 2. Korlátlan Vec Allokáció

```rust
// ❌ TILOS - Nincs méret ellenőrzés
let mut results = Vec::new();
for item in large_iterator {
    results.push(item);  // Korlátlan növekedés!
}

// ❌ TILOS - with_capacity nagy számmal
let mut vec = Vec::with_capacity(1_000_000);  // 1M elem előre
```

### 3. Teljes Rendezés + Truncate

```rust
// ❌ TILOS - O(n) memória a rendezéshez
let mut all_docs = load_all_documents();
all_docs.sort_by(|a, b| compare(a, b));
all_docs.truncate(10);  // Csak 10 kell, de MIND memóriában volt!
```

---

## KÖTELEZŐ Minták ✅

### 1. Streaming Document Loading

```rust
// ✅ HELYES - Egy doc egyszerre
for doc_id in doc_ids {
    let doc = load_one(doc_id)?;  // EGY doc memóriában
    process(&doc);
    // doc FELSZABADUL itt
}

// ✅ HELYES - Iterator chain (lazy)
doc_ids.iter()
    .filter_map(|id| load_one(id).ok())
    .take(limit)  // LIMIT ELŐBB!
    .for_each(|doc| process(&doc));
```

### 2. try_reserve() Használata

```rust
// ✅ HELYES - Allokáció ellenőrzés
let mut results = Vec::new();
results.try_reserve(estimated_count).map_err(|e| {
    IronBaseError::OutOfMemory(format!(
        "Cannot allocate {} elements: {}",
        estimated_count, e
    ))
})?;

// Ezután biztonságos a push
for item in items.take(estimated_count) {
    results.push(item);
}
```

### 3. Chunked Processing

```rust
// ✅ HELYES - Chunk-olt feldolgozás
const CHUNK_SIZE: usize = 1000;  // ~500MB max per chunk

for chunk in catalog_entries.chunks(CHUNK_SIZE) {
    // Chunk betöltése
    let batch: Vec<Document> = chunk
        .iter()
        .filter_map(|entry| load_document(&entry.id).ok())
        .collect();

    // Feldolgozás (akár párhuzamosan)
    process_batch(&batch);

    // batch FELSZABADUL a scope végén
}
```

### 4. Top-K Heap (Sort + Limit)

```rust
use std::collections::BinaryHeap;
use std::cmp::Reverse;

// ✅ HELYES - O(k) memória
fn top_k<T: Ord>(items: impl Iterator<Item = T>, k: usize) -> Vec<T> {
    let mut heap = BinaryHeap::with_capacity(k + 1);

    for item in items {
        heap.push(Reverse(item));
        if heap.len() > k {
            heap.pop();  // Legkisebb kidobása
        }
    }

    heap.into_iter().map(|Reverse(x)| x).collect()
}

// Használat
let top_10 = top_k(all_scores.into_iter(), 10);
```

### 5. Range Query Unified API

```rust
use crate::index::{RangeQueryMode, RangeQueryResult, ScanOrder};

// ✅ HELYES - Count O(1) memóriával
let result = btree.range_query(
    &start_key,
    &end_key,
    true,  // inclusive_start
    true,  // inclusive_end
    RangeQueryMode::Count
);
let count = match result {
    RangeQueryResult::Count(c) => c,
    _ => unreachable!(),
};

// ✅ HELYES - Scan limittel O(limit) memóriával
let result = btree.range_query(
    &start_key,
    &end_key,
    true,
    true,
    RangeQueryMode::Scan {
        skip: 0,
        limit: Some(100),  // MAX 100 elem!
        order: ScanOrder::Asc,
    }
);
let docs = match result {
    RangeQueryResult::Docs(d) => d,
    _ => unreachable!(),
};
```

### 6. Aggregation Dinamikus Limitek

```rust
// ✅ HELYES - Rendszer RAM alapú skálázás
let results = collection.aggregate_auto(&pipeline)?;

// ✅ HELYES - Explicit memory budget
let limits = AggregationLimits::with_memory_budget(256); // 256 MB max
let results = collection.aggregate_with_limits(&pipeline, limits)?;

// ✅ HELYES - Rendszer memória alapján
let limits = AggregationLimits::from_system_memory();
// Automatikusan beállítja max_docs, max_groups, stb.
```

### 7. Find Safe Defaults

```rust
// ✅ HELYES - RAM-alapú response limit
let options = FindOptions::with_safe_defaults()
    .with_limit(100)
    .with_projection(projection);

let results = collection.find_with_options(&query, options)?;

// ✅ HELYES - Explicit response limit
let options = FindOptions::new()
    .with_max_response_bytes(50 * 1024 * 1024); // 50 MB max
```

---

## Memória Garanciák Táblázat

| Művelet | Helyes Minta | Memória Komplexitás |
|---------|--------------|---------------------|
| Count | `RangeQueryMode::Count` | O(1) |
| Scan + limit | `RangeQueryMode::Scan { limit: Some(k) }` | O(k) |
| Top-K rendezés | `BinaryHeap` + `pop()` | O(k) |
| Doc loading | Streaming `for` loop | O(1) per doc |
| Batch processing | `chunks(N)` | O(N) per chunk |
| Vec allokáció | `try_reserve()` | Fail-fast OOM előtt |

---

## Skálázási Táblázatok

### Aggregation Limits (`from_system_memory()`)

| Elérhető RAM | max_memory_mb | max_docs | max_groups |
|--------------|---------------|----------|------------|
| < 512 MB     | 64            | 10K      | 5K         |
| 512MB - 2GB  | 128           | 50K      | 25K        |
| 2GB - 8GB    | 256           | 100K     | 50K        |
| 8GB - 32GB   | 512           | 250K     | 100K       |
| > 32GB       | 1024          | 500K     | 250K       |

### Find Response Limits (`with_safe_defaults()`)

| Elérhető RAM | max_response_bytes |
|--------------|--------------------|
| < 512 MB     | 10 MB              |
| 512MB - 2GB  | 50 MB              |
| 2GB - 8GB    | 100 MB             |
| 8GB - 32GB   | 200 MB             |
| > 32GB       | 500 MB             |

---

## Key Files

| Fájl | Tartalom |
|------|----------|
| `ironbase-core/src/aggregation/memory_info.rs` | RAM detektálás |
| `ironbase-core/src/aggregation/types.rs` | AggregationLimits struct |
| `ironbase-core/src/find_options.rs` | FindOptions, estimate_json_size() |
| `ironbase-core/src/collection_core/mod.rs` | find_with_options response tracking |
| `ironbase-core/src/index.rs` | RangeQueryMode, range_query() |
| `ironbase-core/src/collection_core/topk.rs` | topk_documents() |

---

## Checklist Új Kód Review-hoz

- [ ] Van `.collect()` nagy iterátoron? → Streaming-re cserélni
- [ ] Van `Vec::new()` + loop push? → `try_reserve()` hozzáadni
- [ ] Van sort + truncate? → Top-K heap-re cserélni
- [ ] Van `with_capacity(N)` ahol N > 10K? → Dinamikus limit
- [ ] Használ régi `range_scan()` API-t? → `range_query()` unified API
- [ ] Aggregation hardcoded limitekkel? → `from_system_memory()`
- [ ] Find limit nélkül? → `with_safe_defaults()`
