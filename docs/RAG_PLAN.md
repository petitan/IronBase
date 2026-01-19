# IronBase RAG Implementation Plan

## Összefoglaló

RAG (Retrieval Augmented Generation) képesség hozzáadása az IronBase-hez, lokális FastText embedding modellel.

**Cél:** Dokumentumok szemantikus keresése természetes nyelvi kérdésekkel.

**Scope:**
- ✅ Markdown (.md) és szöveges (.txt) fájlok
- ✅ Lokális FastText magyar embedding
- ✅ Smart chunking (táblázat védelem)
- ❌ PDF (túl komplex)
- ❌ URL import (felesleges)

---

## Architektúra

```
┌─────────────────────────────────────────────────────────────────┐
│                         RAG PIPELINE                             │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  1. IMPORT                                                       │
│  ────────                                                        │
│  Fájl (MD/TXT) → Smart Chunking → Chunks + Embedding → DB       │
│                                                                  │
│  2. KERESÉS                                                      │
│  ─────────                                                       │
│  Query → Embedding → Vector Search → Top-K Chunks               │
│                                                                  │
│  3. VÁLASZ (opcionális)                                         │
│  ──────────────────────                                          │
│  Top-K Chunks + Query → LLM → Válasz                            │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

---

## Adatstruktúra

### RAG Collection

```javascript
// Fő dokumentumok
{
  "_id": "doc_abc123",
  "title": "Minőségügyi Kézikönyv",
  "source_path": "D:/docs/kezikonyv.md",
  "content_hash": "sha256:...",        // Változás detektálás
  "chunk_count": 45,
  "created_at": "2024-01-15T10:30:00Z",
  "metadata": {
    "version": "2.1",
    "author": "Kovács János"
  }
}
```

### Chunks Collection (automatikus)

```javascript
// {collection_name}_chunks
{
  "_id": "chunk_xyz789",
  "doc_id": "doc_abc123",              // → fő dokumentum
  "chunk_index": 0,                     // Sorrend
  "text": "1. Bevezetés\n\nA minőségirányítási rendszer célja...",
  "char_start": 0,
  "char_end": 1500,
  "token_count": 487,
  "block_type": "paragraph",           // paragraph | table | code | heading
  "embedding": [0.123, -0.456, ...]    // 300 dim (FastText)
}
```

---

## Komponensek

### 1. Markdown Parser

**Fájl:** `ironbase-core/src/rag/markdown.rs`

```rust
pub enum MarkdownBlock {
    Heading { level: u8, text: String },
    Paragraph(String),
    Table { headers: Vec<String>, rows: Vec<Vec<String>>, raw: String },
    CodeBlock { language: Option<String>, code: String },
    List { ordered: bool, items: Vec<String> },
    Blockquote(String),
}

pub fn parse_markdown(content: &str) -> Vec<MarkdownBlock>;
```

**Cél:** Block szintű parsing a smart chunking-hoz.

---

### 2. Smart Chunking

**Fájl:** `ironbase-core/src/rag/chunking.rs`

```rust
pub struct ChunkConfig {
    pub max_tokens: usize,        // 500
    pub overlap_tokens: usize,    // 50
    pub min_chunk_size: usize,    // 100
    pub preserve_tables: bool,    // true - táblázat egyben marad
    pub preserve_code: bool,      // true - code block egyben marad
}

pub struct Chunk {
    pub text: String,
    pub char_start: usize,
    pub char_end: usize,
    pub token_count: usize,
    pub block_type: BlockType,
}

impl Chunker {
    pub fn chunk_markdown(&self, blocks: &[MarkdownBlock]) -> Vec<Chunk>;
}
```

**Szabályok:**
- Heading előtt mindig vágás
- Táblázat EGYBEN marad (max_tokens felülírva)
- Code block EGYBEN marad
- Paragraph chunkolható overlap-pel

---

### 3. FastText Embedding

**Fájl:** `ironbase-core/src/rag/fasttext.rs`

**Modell:** `cc.hu.300.bin` (650 MB, magyar)

```rust
pub struct FastTextModel {
    vectors: HashMap<String, Vec<f32>>,  // 2M szó × 300 dim
    dim: usize,                           // 300
}

impl FastTextModel {
    /// Betölti a .bin fájlt
    pub fn load(path: &Path) -> Result<Self>;

    /// Szó vektor lekérése
    pub fn word_vector(&self, word: &str) -> Option<&[f32]>;

    /// Dokumentum vektor = szavak átlaga
    pub fn document_vector(&self, text: &str) -> Vec<f32>;
}
```

**Preprocessing:**
1. Lowercase
2. Ékezet megtartása (magyar!)
3. Stopword szűrés (opcionális)
4. Tokenizálás whitespace-re

---

### 4. Vector Search

**Fájl:** `ironbase-core/src/rag/search.rs`

```rust
/// Cosine similarity két vektor között
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32;

/// Brute-force vector search
pub fn find_similar(
    query_vec: &[f32],
    documents: &[(String, Vec<f32>)],  // (doc_id, embedding)
    top_k: usize,
) -> Vec<(String, f32)>;  // (doc_id, score)
```

**Komplexitás:** O(n) - 7K dokumentumra ~50-100ms, elfogadható.

---

### 5. MCP Tools

**Fájl:** `mcp-server/src/tools/rag.rs`

#### rag_create_collection

```javascript
{
  "name": "rag_create_collection",
  "arguments": {
    "name": "minosegugyi",
    "chunk_config": {
      "max_tokens": 500,
      "overlap": 50,
      "preserve_tables": true
    },
    "admin_key": "..."
  }
}
```

#### rag_import_file

```javascript
{
  "name": "rag_import_file",
  "arguments": {
    "collection": "minosegugyi",
    "path": "D:/docs/kezikonyv.md",
    "title": "Minőségügyi Kézikönyv",
    "metadata": { "version": "2.1" },
    "admin_key": "..."
  }
}
```

**Folyamat:**
1. Fájl beolvasása
2. Markdown parsing
3. Smart chunking
4. FastText embedding minden chunk-ra
5. Mentés: fő doc + chunks

#### rag_import_folder

```javascript
{
  "name": "rag_import_folder",
  "arguments": {
    "collection": "minosegugyi",
    "path": "D:/docs/minosegugy/",
    "pattern": "*.md",
    "admin_key": "..."
  }
}
```

#### rag_search

```javascript
{
  "name": "rag_search",
  "arguments": {
    "collection": "minosegugyi",
    "query": "Ki felelős az auditokért?",
    "top_k": 5,
    "min_score": 0.5
  }
}

// Eredmény
{
  "results": [
    {
      "doc_id": "doc_abc123",
      "doc_title": "Minőségügyi Kézikönyv",
      "chunk_index": 12,
      "text": "A minőségügyi vezető felelős...",
      "score": 0.87
    }
  ]
}
```

#### rag_delete_document

```javascript
{
  "name": "rag_delete_document",
  "arguments": {
    "collection": "minosegugyi",
    "doc_id": "doc_abc123",
    "admin_key": "..."
  }
}
```

---

## Fájlstruktúra

```
ironbase-core/src/
├── rag/
│   ├── mod.rs
│   ├── markdown.rs      # Markdown parser
│   ├── chunking.rs      # Smart chunking
│   ├── fasttext.rs      # FastText loader + embedding
│   ├── search.rs        # Vector similarity search
│   └── types.rs         # Chunk, RagConfig structs

mcp-server/src/
├── tools/
│   ├── rag.rs           # RAG MCP tools
│   └── definitions/
│       └── rag.rs       # Tool schemas
```

---

## Fejlesztési ütemterv

| Fázis | Feladat | Becsült idő |
|-------|---------|-------------|
| **1** | Markdown parser (block szintű) | 2 nap |
| **2** | Smart chunking (táblázat védelem) | 2 nap |
| **3** | FastText loader (.bin format) | 2 nap |
| **4** | Document embedding | 1 nap |
| **5** | Vector search (brute-force) | 2 nap |
| **6** | MCP tools (create, import, search) | 2 nap |
| **7** | Tesztek | 2 nap |
| **8** | TUI integráció (opcionális) | 3 nap |
| | **Összesen** | **~2 hét** |

---

## Tesztelési terv

### Unit tesztek

```rust
#[test]
fn test_markdown_table_detection();

#[test]
fn test_chunking_preserves_table();

#[test]
fn test_fasttext_word_vector();

#[test]
fn test_document_embedding();

#[test]
fn test_cosine_similarity();
```

### Integrációs tesztek

```rust
#[test]
fn test_rag_import_and_search() {
    // 1. Collection létrehozás
    // 2. MD fájl import
    // 3. Keresés
    // 4. Relevancia ellenőrzés
}
```

---

## Függőségek

### Rust crates

```toml
# ironbase-core/Cargo.toml
[dependencies]
# Meglévők...

# RAG
byteorder = "1.5"      # FastText .bin olvasás
unicode-segmentation = "1.10"  # Tokenizálás
```

### Külső fájlok

```
models/
└── cc.hu.300.bin      # 650 MB - Magyar FastText modell
                       # Letöltés: https://fasttext.cc/docs/en/crawl-vectors.html
```

---

## Későbbi fejlesztések (v2)

- [ ] HNSW index nagy adathalmazokhoz (100K+ chunk)
- [ ] Hibrid keresés (vector + fulltext kombináció)
- [ ] Incremental update (csak változott részek újraindexelése)
- [ ] Többnyelvű támogatás (angol, német FastText)
- [ ] LLM integráció válasz generáláshoz

---

## Kapcsolódó dokumentumok

- [CLAUDE.md](../CLAUDE.md) - Fejlesztési irányelvek
- [mcp-server/README.md](../mcp-server/README.md) - MCP szerver dokumentáció
