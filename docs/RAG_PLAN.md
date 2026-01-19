# IronBase RAG Implementation Plan

## Összefoglaló

RAG (Retrieval Augmented Generation) képesség hozzáadása az IronBase-hez, nagy dokumentumokra (ISO kézikönyvek, 17025) optimalizálva.

**Cél:** Gyors szemantikus keresés nagy méretű dokumentumokon.

**Követelmények:**
- ✅ 100+ oldalas dokumentumok kezelése
- ✅ <100ms keresési idő
- ✅ Offline működés (lokális embedding)
- ✅ Táblázatok és struktúrák megőrzése

**Scope:**
- ✅ Markdown (.md) és szöveges (.txt) fájlok
- ✅ Lokális FastText embedding (memmap - nem tölti RAM-ba)
- ✅ HNSW index (gyors ANN keresés)
- ❌ PDF, URL (scope-on kívül)

---

## Mérnöki döntések

| Döntés | Választás | Indoklás |
|--------|-----------|----------|
| Embedding | FastText (memmap) | Offline, gyors, magyar támogatás |
| Vector index | HNSW | O(log n) keresés, skálázható |
| Chunk méret | 1000 token | ISO dokumentumok sűrű szövege |
| Overlap | 100 token | Kontextus megőrzés |
| Táblázat | Egyben + külön index | Struktúra és kereshetőség |

---

## Architektúra

```
┌─────────────────────────────────────────────────────────────────────────┐
│                         RAG PIPELINE (Nagy dokumentumok)                 │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                          │
│  IMPORT (egyszer, háttérben)                                            │
│  ────────────────────────────                                            │
│                                                                          │
│  ISO_17025.md (500KB)                                                   │
│       │                                                                  │
│       ▼                                                                  │
│  ┌─────────────────┐     ┌─────────────────┐     ┌─────────────────┐   │
│  │ Markdown Parser │ ──▶ │ Smart Chunking  │ ──▶ │ FastText Embed  │   │
│  │ (streaming)     │     │ (táblázat védelem)    │ (batch, SIMD)   │   │
│  └─────────────────┘     └─────────────────┘     └────────┬────────┘   │
│                                                            │            │
│                                                            ▼            │
│                                              ┌─────────────────────┐   │
│                                              │ HNSW Index Build    │   │
│                                              │ (háttérszálon)      │   │
│                                              └─────────────────────┘   │
│                                                                          │
│  KERESÉS (<100ms)                                                       │
│  ─────────────────                                                       │
│                                                                          │
│  "mérési bizonytalanság számítása"                                      │
│       │                                                                  │
│       ▼                                                                  │
│  ┌─────────────────┐     ┌─────────────────┐     ┌─────────────────┐   │
│  │ Query Embedding │ ──▶ │ HNSW Search     │ ──▶ │ Top-K Chunks    │   │
│  │ (~1ms)          │     │ (~5-10ms)       │     │ + Relevancia    │   │
│  └─────────────────┘     └─────────────────┘     └─────────────────┘   │
│                                                                          │
└─────────────────────────────────────────────────────────────────────────┘
```

---

## Teljesítmény célok

| Metrika | Cél | Módszer |
|---------|-----|---------|
| Import sebesség | 1000 chunk/sec | Batch embedding, párhuzamos I/O |
| Keresési idő | <100ms | HNSW index, SIMD cosine |
| Memória (runtime) | <500MB | FastText memmap, lazy loading |
| Index méret | ~1KB/chunk | HNSW M=16, ef=200 |

**Példa számítás (ISO 17025 kézikönyv):**
```
Dokumentum: 200 oldal, ~100,000 szó
Chunk-ok: ~200 db (1000 token/chunk)
Embedding: 200 × 300 dim × 4 byte = 240 KB
HNSW index: ~200 KB
Keresési idő: ~5-10ms
```

---

## Adatstruktúra

### Documents Collection

```javascript
{
  "_id": "doc_17025_v3",
  "title": "ISO/IEC 17025:2017 Kézikönyv",
  "source_path": "D:/ISO/17025_kezikonyv_v3.md",
  "content_hash": "sha256:a1b2c3...",
  "file_size": 524288,
  "chunk_count": 187,
  "table_count": 45,
  "import_time_ms": 3200,
  "created_at": "2024-01-15T10:30:00Z",
  "metadata": {
    "standard": "ISO/IEC 17025:2017",
    "version": "3.0",
    "language": "hu"
  }
}
```

### Chunks Collection

```javascript
{
  "_id": "chunk_17025_042",
  "doc_id": "doc_17025_v3",
  "chunk_index": 42,
  "section_path": ["5. Strukturális követelmények", "5.3 Felső vezetés"],

  // Tartalom
  "text": "A felső vezetésnek biztosítania kell, hogy...",
  "char_range": [45230, 48750],
  "token_count": 892,

  // Típus és struktúra
  "block_type": "paragraph",  // paragraph | table | heading | list
  "heading_level": null,
  "parent_heading": "5.3 Felső vezetés",

  // Embedding (300 dim, bináris tárolás)
  "embedding": Binary(...)
}
```

### Tables Collection (külön, kereshetőség miatt)

```javascript
{
  "_id": "table_17025_012",
  "doc_id": "doc_17025_v3",
  "chunk_id": "chunk_17025_042",

  // Struktúra
  "headers": ["Követelmény", "Felelős", "Határidő"],
  "rows": [
    ["Kalibráció", "Labor vezető", "Évente"],
    ["Felülvizsgálat", "Minőségügyi vezető", "Félévente"]
  ],

  // Kereshető szöveg verzió
  "text_representation": "Követelmény: Kalibráció, Felelős: Labor vezető...",
  "embedding": Binary(...)
}
```

---

## Komponensek

### 1. FastText Engine (memmap)

**Fájl:** `ironbase-core/src/rag/fasttext.rs`

```rust
use memmap2::Mmap;

/// Memory-mapped FastText - NEM tölti RAM-ba a teljes modellt
pub struct FastTextEngine {
    mmap: Mmap,                           // Memory-mapped fájl
    word_index: HashMap<String, u64>,     // Szó → offset a fájlban
    dim: usize,                           // 300
    vocab_size: usize,                    // ~2M
}

impl FastTextEngine {
    /// Betöltés - csak az indexet olvassa RAM-ba (~50MB)
    pub fn open(path: &Path) -> Result<Self>;

    /// Szó vektor - lazy load a memmap-ból
    pub fn word_vector(&self, word: &str) -> Option<&[f32]>;

    /// Dokumentum embedding - SIMD optimalizált átlagolás
    #[cfg(target_arch = "x86_64")]
    pub fn document_embedding(&self, text: &str) -> Vec<f32>;

    /// Batch embedding - párhuzamos feldolgozás
    pub fn batch_embed(&self, texts: &[&str]) -> Vec<Vec<f32>>;
}
```

**Memória használat:**
- Teljes modell: 650 MB (fájl)
- RAM használat: ~50 MB (csak word index)
- Lazy load: vektorok on-demand

---

### 2. HNSW Index

**Fájl:** `ironbase-core/src/rag/hnsw.rs`

```rust
/// Hierarchical Navigable Small World graph
pub struct HnswIndex {
    // Konfiguráció
    m: usize,                    // Max kapcsolatok (16)
    ef_construction: usize,      // Build minőség (200)
    ef_search: usize,            // Search minőség (50)
    dim: usize,                  // Vektor dimenzió (300)

    // Gráf struktúra
    layers: Vec<Layer>,
    entry_point: NodeId,

    // Persistencia
    storage_path: PathBuf,
}

impl HnswIndex {
    /// Új index létrehozás
    pub fn new(config: HnswConfig) -> Self;

    /// Betöltés fájlból
    pub fn load(path: &Path) -> Result<Self>;

    /// Mentés fájlba
    pub fn save(&self, path: &Path) -> Result<()>;

    /// Vektor hozzáadása
    pub fn insert(&mut self, id: &str, vector: &[f32]) -> Result<()>;

    /// Batch insert (gyorsabb)
    pub fn insert_batch(&mut self, items: &[(&str, &[f32])]) -> Result<()>;

    /// ANN keresés - O(log n)
    pub fn search(&self, query: &[f32], k: usize) -> Vec<(String, f32)>;

    /// Törlés
    pub fn delete(&mut self, id: &str) -> Result<()>;
}
```

**HNSW paraméterek (ISO dokumentumokra optimalizálva):**

| Paraméter | Érték | Hatás |
|-----------|-------|-------|
| M | 16 | Kapcsolatok száma (memória vs pontosság) |
| ef_construction | 200 | Build minőség (lassabb build, jobb index) |
| ef_search | 50 | Keresési pontosság (~99% recall) |

---

### 3. Smart Chunker

**Fájl:** `ironbase-core/src/rag/chunking.rs`

```rust
pub struct ChunkConfig {
    pub max_tokens: usize,          // 1000 (ISO dokumentumok)
    pub overlap_tokens: usize,      // 100
    pub preserve_tables: bool,      // true
    pub preserve_lists: bool,       // true
    pub split_on_heading: bool,     // true
}

pub struct Chunk {
    pub text: String,
    pub char_range: (usize, usize),
    pub token_count: usize,
    pub block_type: BlockType,
    pub section_path: Vec<String>,   // Heading hierarchia
    pub parent_heading: Option<String>,
}

impl Chunker {
    /// Streaming chunking - nagy fájlokhoz
    pub fn chunk_streaming<R: Read>(&self, reader: R) -> impl Iterator<Item = Chunk>;

    /// Teljes markdown chunking
    pub fn chunk_markdown(&self, content: &str) -> Vec<Chunk>;
}
```

**Chunking szabályok (ISO dokumentumokra):**

1. **Heading = vágási pont** - minden új fejezet új chunk
2. **Táblázat egyben** - akár 2000 token is lehet
3. **Lista egyben** - ha <1500 token
4. **Bekezdés osztható** - overlap-pel
5. **Section path megőrzés** - "5. Követelmények > 5.3 Felső vezetés"

---

### 4. RAG Manager

**Fájl:** `ironbase-core/src/rag/manager.rs`

```rust
/// Központi RAG kezelő
pub struct RagManager {
    fasttext: Arc<FastTextEngine>,
    collections: HashMap<String, RagCollection>,
}

pub struct RagCollection {
    name: String,
    hnsw: HnswIndex,
    chunk_config: ChunkConfig,
    doc_count: usize,
    chunk_count: usize,
}

impl RagManager {
    /// Collection létrehozás
    pub fn create_collection(&mut self, name: &str, config: ChunkConfig) -> Result<()>;

    /// Fájl import (streaming, háttérszálon)
    pub async fn import_file(&self, collection: &str, path: &Path, metadata: Value) -> Result<ImportResult>;

    /// Mappa import
    pub async fn import_folder(&self, collection: &str, path: &Path, pattern: &str) -> Result<Vec<ImportResult>>;

    /// Szemantikus keresés
    pub fn search(&self, collection: &str, query: &str, top_k: usize) -> Result<Vec<SearchResult>>;

    /// Hibrid keresés (vector + fulltext)
    pub fn hybrid_search(&self, collection: &str, query: &str, top_k: usize) -> Result<Vec<SearchResult>>;
}
```

---

## MCP Tools

### rag_create_collection

```javascript
{
  "name": "rag_create_collection",
  "arguments": {
    "name": "iso_17025",
    "config": {
      "max_tokens": 1000,
      "overlap": 100,
      "preserve_tables": true,
      "language": "hu"
    },
    "admin_key": "..."
  }
}
```

### rag_import_file

```javascript
{
  "name": "rag_import_file",
  "arguments": {
    "collection": "iso_17025",
    "path": "D:/ISO/17025_kezikonyv.md",
    "title": "ISO/IEC 17025:2017 Kézikönyv",
    "metadata": {
      "standard": "ISO/IEC 17025:2017",
      "version": "3.0"
    },
    "admin_key": "..."
  }
}

// Válasz
{
  "doc_id": "doc_17025_v3",
  "chunks_created": 187,
  "tables_extracted": 45,
  "import_time_ms": 3200
}
```

### rag_search

```javascript
{
  "name": "rag_search",
  "arguments": {
    "collection": "iso_17025",
    "query": "mérési bizonytalanság számítása",
    "top_k": 5,
    "include_tables": true
  }
}

// Válasz
{
  "results": [
    {
      "doc_id": "doc_17025_v3",
      "doc_title": "ISO/IEC 17025:2017 Kézikönyv",
      "chunk_id": "chunk_17025_089",
      "section": "7.6 A mérési bizonytalanság értékelése",
      "text": "A laboratóriumnak azonosítania kell a bizonytalansági forrásokat...",
      "score": 0.91,
      "block_type": "paragraph"
    },
    {
      "doc_id": "doc_17025_v3",
      "chunk_id": "table_17025_023",
      "section": "7.6 A mérési bizonytalanság értékelése",
      "text": "| Bizonytalansági forrás | Típus | Értékelés módja |...",
      "score": 0.87,
      "block_type": "table"
    }
  ],
  "search_time_ms": 8
}
```

### rag_hybrid_search

```javascript
{
  "name": "rag_hybrid_search",
  "arguments": {
    "collection": "iso_17025",
    "query": "kalibráló laboratórium követelményei",
    "top_k": 10,
    "vector_weight": 0.7,     // Szemantikus
    "fulltext_weight": 0.3    // Kulcsszó
  }
}
```

---

## Fájlstruktúra

```
ironbase-core/src/
├── rag/
│   ├── mod.rs              # Pub exports
│   ├── fasttext.rs         # FastText engine (memmap)
│   ├── hnsw.rs             # HNSW index implementáció
│   ├── chunking.rs         # Smart chunker
│   ├── markdown.rs         # Markdown parser
│   ├── manager.rs          # RAG manager (központi API)
│   ├── search.rs           # Keresési logika
│   └── types.rs            # Struktúrák

mcp-server/src/
├── tools/
│   ├── rag.rs              # RAG tool handlers
│   └── definitions/
│       └── rag.rs          # Tool schemas

models/                      # Gitignore-olt
└── cc.hu.300.bin           # FastText modell (650 MB)

data/
└── {collection}/
    ├── hnsw.idx            # HNSW index fájl
    └── chunks.dat          # Chunk embeddings
```

---

## Fejlesztési ütemterv

| Fázis | Feladat | Idő | Prioritás |
|-------|---------|-----|-----------|
| **1** | FastText memmap loader | 3 nap | 🔴 Kritikus |
| **2** | HNSW index implementáció | 5 nap | 🔴 Kritikus |
| **3** | Markdown parser + chunker | 3 nap | 🔴 Kritikus |
| **4** | RAG manager + persistence | 2 nap | 🔴 Kritikus |
| **5** | MCP tools | 2 nap | 🔴 Kritikus |
| **6** | SIMD optimalizáció | 2 nap | 🟡 Fontos |
| **7** | Hibrid keresés | 2 nap | 🟡 Fontos |
| **8** | Tesztek (unit + integration) | 3 nap | 🔴 Kritikus |
| | **Összesen** | **~3 hét** | |

---

## Függőségek

```toml
# ironbase-core/Cargo.toml
[dependencies]
# Meglévők...

# RAG
memmap2 = "0.9"                    # Memory-mapped fájlok
byteorder = "1.5"                  # Binary formátum olvasás
parking_lot = "0.12"               # Gyors RwLock (már van)

# SIMD (opcionális, de ajánlott)
simdeez = "1.0"                    # Platform-független SIMD

[target.'cfg(target_arch = "x86_64")'.dependencies]
safe_arch = "0.7"                  # x86 SIMD intrinsics
```

---

## Benchmark célok

### Import (ISO 17025 kézikönyv, 200 oldal)

| Lépés | Cél idő |
|-------|---------|
| Fájl olvasás | <50ms |
| Markdown parsing | <100ms |
| Chunking | <100ms |
| Embedding (200 chunk) | <2000ms |
| HNSW index build | <500ms |
| **Összesen** | **<3 sec** |

### Keresés

| Méret | Brute-force | HNSW |
|-------|-------------|------|
| 1K chunk | 50ms | 5ms |
| 10K chunk | 500ms | 8ms |
| 100K chunk | 5000ms | 15ms |

---

## Tesztelési terv

### Unit tesztek

```rust
#[test]
fn test_fasttext_memmap_load();

#[test]
fn test_fasttext_word_lookup();

#[test]
fn test_hnsw_insert_and_search();

#[test]
fn test_hnsw_recall_accuracy();  // >95% @ top-10

#[test]
fn test_chunking_preserves_tables();

#[test]
fn test_chunking_section_hierarchy();
```

### Integrációs tesztek

```rust
#[test]
fn test_import_large_document() {
    // 100+ oldalas markdown import
    // Ellenőrzés: chunk count, index méret
}

#[test]
fn test_search_relevance() {
    // ISO 17025 kérdések
    // Ellenőrzés: releváns chunk a top-3-ban
}

#[test]
fn test_search_performance() {
    // 10K chunk
    // Ellenőrzés: <100ms
}
```

### Benchmark teszt

```rust
#[test]
#[ignore]  // cargo test --release -- --ignored
fn benchmark_full_pipeline() {
    // Import + 100 keresés
    // Statisztikák kiírása
}
```

---

## Külső modell letöltés

```bash
# Magyar FastText modell
wget https://dl.fbaipublicfiles.com/fasttext/vectors-crawl/cc.hu.300.bin.gz
gunzip cc.hu.300.bin.gz
mv cc.hu.300.bin models/

# Ellenőrzés
ls -lh models/cc.hu.300.bin
# 626M models/cc.hu.300.bin
```

---

## Kapcsolódó dokumentumok

- [CLAUDE.md](../CLAUDE.md) - Fejlesztési irányelvek
- [mcp-server/README.md](../mcp-server/README.md) - MCP szerver dokumentáció
