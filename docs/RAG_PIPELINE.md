# RAG Pipeline — Mérnöki áttekintés

**Hatály:** `mcp-server` v1.0.494 – v1.0.500. Egységes referencia a chunkolási,
embedelési, idempotens import, multi-field fulltext indexelési és hibrid keresési
pipeline-ról. Lezárt issue-k: #67, #64, #65, #63, #66.

---

## 1. Adatfolyam

```
                                ┌─────────────────────────────────────┐
       chunk_content()  ───►    │  markdown::split / text::split      │
                                │  • header detect (fenced-code-safe) │
                                │  • section_path follow              │
                                │  • table header propagation (#63)   │
                                └────────────┬────────────────────────┘
                                             ▼
                                 Chunk { text, section_path,
                                         table_header, heading, ... }
                                             │
                                             ▼
                        build_embed_text()  ──►  embedding provider
                        (breadcrumb + tábla-flat)        │
                                             │           ▼
                                             │      Vec<f32>
                                             ▼
              insert_chunks_idempotent() (#67, safe ordering)
                 ├─ capture old _ids (`_id` projekció)
                 ├─ insert new chunks
                 └─ delete old _ids
                                             │
                                             ▼
                                       _system.rag (RagConfig)
                                             │
                                             ▼
                                       hybrid_search
                  ├── effective_text_fields ◄── config ∩ indexed (#66)
                  ├── vector search       ┐
                  ├── fulltext (single|multi-field, AND/OR, match_scope)
                  └── RRF fusion → rerank → merge_adjacent_chunks (#63 dedup)
                                             │
                                             ▼
                                      response
```

A pipeline minden ágában érvényes alapelvek:

- **Embed-text ≠ store-text.** Amit beágyazol és amit tárolsz, **különböző szöveg**;
  a tárolt `content` mindig az eredeti chunk-slice, az embed-text breadcrumb-fel és
  tábla-lapítással gazdagítva. (v1.0.494)
- **Safe ordering.** Részleges hiba sosem ronthat el meglévő jó adatot — a drága,
  hibázó lépés (embedding) sosem fut a DB-mutáció előtt. (#67)
- **Silent fallback tilos.** Eldobott paraméter, kihagyott index, csendes degradáció
  mindig `tracing::warn!` + látható válaszmező. (egész pipeline)

---

## 2. Komponensek

### 2.1 Chunkolás — `mcp-server/src/chunking/`

`chunk_content(content, options)` → `Vec<Chunk>`. Két splitter (`markdown::split`,
`text::split`) ugyanazon `Chunk` struktúrát adja. Auto-detekció markdown vs text:
az első 20 sorban `#`-heading-keresés.

`Chunk` (egységesen, v1.0.494 + v1.0.499):

```rust
pub struct Chunk {
    pub index: usize, pub total: usize,
    pub text: String,            // overlap-extended slice OR header-prepended (continuation)
    pub start_char: usize, pub end_char: usize,
    pub heading: Option<String>, pub heading_level: Option<u8>,
    pub section_path: Option<Vec<String>>,
    pub table_header: Option<String>, // #63: prepended block on continuations
}
```

**Markdown chunkolás (`markdown::split`)** négy állapotot követ a chunk-iteráció során:

1. `section_path` (heading hierarchia) — `extract_heading` + `update_section_path`.
2. `fence_open` (fenced code, ` ``` ` / `~~~`) — `count_fence_lines % 2`. Megakadályozza,
   hogy egy bash `# comment` headingként szennyezze a section_path-ot. (v1.0.494)
3. `current_table_header` (a *nyers, overlap-mentes* slice-on detektálva) — `find_table_header`.
   Heading-nél resetelődik (új szekció → új tábla-kontextus). Folytatás-chunk elé fűződik,
   és a `table_header` mezőben tárolódik. (#63, v1.0.499)
4. Overlap-szövegezés `overlap_start_byte` segítségével, UTF-8 safe, szóhatárra
   illesztve.

A shared tábla-prediktumok (`chunking/mod.rs`-ben pub(crate)):
- `is_table_row(line)` — `trim_start().starts_with('|')`
- `is_table_separator(line)` — pipe + `-` + (`-`,`:`,space) only
- `find_table_header(text)` — utolsó `header_row + separator_row` blokk visszaadása
- `strip_markdown_tables(text)` — sorok → vesszős érték; **mostantól ugyanazt az
  `is_table_separator` predikátumot használja** (review konzisztencia).

### 2.2 Embed-text — `build_embed_text(body, section_path)` (`chunking/mod.rs`)

A chunk-import 3 útvonala (`rag.rs`, `embedding.rs`, `db_functions.rs db_rag_import`)
**kizárólag ezt a helpert hívja** az embed-input előállítására:

```rust
pub fn build_embed_text(body: &str, section_path: Option<&[String]>) -> String {
    let clean = strip_markdown_tables(body);
    match section_path {
        Some(path) if !path.is_empty() => format!("{}\n\n{}", path.join(" > "), clean),
        _ => clean,
    }
}
```

Tehát az embedelt szöveg = `"Árlista > PEF-széria\n\n<tábla-lapított törzs>"`.
A tárolt `content` érintetlen (eredeti chunk-slice, esetleg overlap-pal). A generikus
auto-embed (`auto_embed.rs`, `crud.rs apply_auto_embedding`) **szándékosan verbatim
embedel** a saját szerződése szerint; ha valaki `auto_embed_enable`-t kapcsol egy
chunk-importált collection-re, az MCP tool egy `warning` mezővel jelzi a határt.

### 2.3 Idempotens chunk-import (#67) — `tools/helpers.rs`

```rust
pub fn insert_chunks_idempotent(
    adapter, collection, doc_id, documents, if_exists
) -> Result<IdempotentInsert>
```

`IfExists`: `Replace` (default) | `Skip` | `Error` | `Append`.

**Safe ordering** (sosem veszít jó adatot):

```
1. find(collection, {doc_id}, projection={_id:1})   → existing_ids (embeddings NEM töltődnek)
2. (policy gate: Error → Err, Skip → return)
3. insert_many(documents)                            → fail esetén a régi chunkok érintetlenek
4. delete_many({_id: {$in: existing_ids}}) (csak Replace)
```

Hiányzó collection (`CollectionNotFound`) explicit kezelése: `existing_ids = []`, az
`insert_many` később auto-create-eli. Más hibák propagálódnak. Részleges hiba a
3-as lépés UTÁN, 4-es ELŐTT → átmeneti duplikátum a `doc_id` alatt; a következő
`Replace` import önjavít (mindkét generációt begyűjti és törli).

**Pre-check az embedding ELŐTT** (v1.0.498):
```rust
should_skip_before_embedding(adapter, collection, doc_id, if_exists)
```
`Skip`/`Error` esetén `count_documents({doc_id})` → ha létezik, rövidre zár az embed
loop előtt — a drága provider-hívást megspóroljuk. `Replace`/`Append` esetén `Ok(false)`.

**Foglalt mezők** (`RESERVED_METADATA_KEYS` `helpers.rs`-ben, mind a 3 útvonalon közös):
```rust
&["_id", "doc_id", "chunk_index", "chunk_total", "start_char", "end_char", "table_header"]
```
User metadata ezeket nem írhatja felül → `tracing::warn!` + skip.

### 2.4 Fulltext indexelés + nyelv (#65, v1.0.497)

A `rag_collection_create` és `rag_document_import` (+ Rhai `db_rag_create`/
`db_rag_import` options) **language paramétere** (`"none"` default) továbbadódik
az auto-létrehozott FTS indexnek. Snowball stemmerek: hungarian, english, german.

A `language` csak az auto-create útvonalon érvényes. Ha az index már létezik és a
kért nyelv eltér a tárolttól: `tracing::warn!` + `language_ignored: true` válaszmező
(silent-fallback tilos). Repeated import ugyanazzal a nyelvvel: nincs warn (csak
ha *eltér* a tárolttól — review hardening, v1.0.500).

`fulltext_analyze` opcionális `collection`+`field` → örökli az index valós
`FtsOptions`-ját az új `adapter.get_fulltext_index_options` metóduson keresztül.
Válaszmezők: `inherited_from_index: true`, `language: "Hungarian"`. Fél-pár (csak
egyik megadva) → hiba, nem silent fallback.

### 2.5 Multi-field fulltext (#66, v1.0.500)

**Setup-időben** (`rag_collection_create` + `rag_document_import` + Rhai):
új `text_fields: Vec<String>` paraméter → FTS index minden megadott mezőre,
deduplikált sorrenden a primary `text_field`-fel együtt (`resolve_fulltext_fields`).
A lista a `RagConfig.text_fields`-be mentődik.

**`RagConfig`** (`tools/rag.rs`):
```rust
pub struct RagConfig {
    pub collection: String,
    pub embedding_field: String,
    pub text_field: String,
    #[serde(default)] pub text_fields: Vec<String>, // legacy → []
    pub provider: String, pub language: String,
    pub dimension: usize, pub created_at: String,
}
impl RagConfig {
    pub fn effective_text_fields(&self) -> Vec<String> {
        if self.text_fields.is_empty() { vec![self.text_field.clone()] }
        else { self.text_fields.clone() }
    }
}
```

A `#[serde(default)]` garantálja, hogy a 1.0.500 előtt mentett configok hibátlanul
deszerializálódnak (üres `text_fields` → `effective_text_fields()` fallback).

**Kereséskor** (`hybrid_search` MCP + Rhai `db_hybrid_search`): ha a hívó nem ad
explicit `text_fields`-et, a tool a config `text_fields`-ére default-ol — de a
közös `resolve_search_text_fields(explicit, config_fields, indexed)` helper
**metszi a ténylegesen indexelt mezőkkel** (`get_fulltext_field_names`). Egy
sikertelen index-build vagy egy későbbi `index_drop` után a default keresés
**nem hasal el** (`fulltext_search_multi` hard-error → most kimarad a hiányzó
mező a default-ból; explicit `text_fields` hívásnál a felelősség a hívóé).

A Rhai oldal **konzisztens**: `get_rag_config` 5-tuple (ef, tf, text_fields, prov,
lang), `db_hybrid_search` ugyanúgy default-ol, `db_rag_stats.config.text_fields`
jelzi a setup-ot.

`rag_document_import` auto-create esetén szintén **ment RagConfig-ot** (text_fields-szel),
így az import-only workflow is élvezi a default-ot. Az új mező nem írható felül
user metadata által (`RESERVED_METADATA_KEYS`).

### 2.6 Determinisztikus single-field feloldás (#64, v1.0.496)

`pick_text_field(fields) -> String` (`tools/hybrid.rs`):

```rust
fn pick_text_field(mut fields: Vec<String>) -> String {
    if fields.iter().any(|f| f == DEFAULT_TEXT_FIELD) { return DEFAULT_TEXT_FIELD.into(); }
    fields.sort();
    fields.into_iter().next().unwrap_or_else(|| DEFAULT_TEXT_FIELD.into())
}
```

A korábbi `get_fulltext_field_names().next()` HashMap-sorrend miatt
nemdeterminisztikusan választott; tartalmazta a `merge_adjacent_chunks`-nak átadott
mezőt, így néha a `title`-t fűzte össze (#64 bug). A `content`-preferáló, deterministic
feloldás kiküszöböli.

### 2.7 Adjacent chunk merge (`fusion.rs`)

`merge_adjacent_chunks(results, text_field)`: a futamokban (`doc_id` + szomszédos
`chunk_index`) össze­vonja a chunkokat. Két fontos részlet:

- **Overlap-vágás**: `start_char`/`end_char` alapján a 2…N. chunk elejéről `prev_end −
  curr_start` karakter levágva (UTF-8 safe `char_indices().nth()`-tel).
- **Tábla-fejléc dedup** (#63): a 2…N. chunkból a stored `table_header` mező +
  `"\n"` prefix **pontosan** kivágva — **NEM** újraszámolva (CRLF/whitespace-safe).
  Ezután fut az overlap-vágás. A merged eredmény egyszer mutatja a fejlécet.

---

## 3. Publikus API (paraméterek)

`rag_collection_create`:

| Param | Típus | Default | Verzió | Leírás |
|-------|-------|---------|--------|--------|
| `collection` | string | — | — | gyűjtemény neve |
| `embedding_field` | string | `"embedding"` | — | vektor-tárolás mező |
| `text_field` | string | `"content"` | — | primary szöveg-mező |
| `text_fields` | string[] | — | **1.0.500** | extra FTS-mezők; primary mindig hozzáadva, dedupolt |
| `provider` | string | manager default | — | embedding provider |
| `language` | enum | `"none"` | — | FTS stemming nyelv (none/hungarian/english/german) |

`rag_document_import`:

| Param | Típus | Default | Verzió | Leírás |
|-------|-------|---------|--------|--------|
| `collection`, `content` | string | — | — | célgyűjtemény + dokumentum |
| `doc_id` | string | UUID | — | logikai dokumentum-azonosító (idempotenciához *kell*) |
| `title`, `metadata`, `provider` | — | — | — | szabad mező + szolgáltató felülírás |
| `chunk_size`, `overlap`, `mode` | — | 1000 / 100 / `"auto"` | — | chunker beállítások |
| `if_exists` | enum | `"replace"` | **1.0.495** | `replace`/`skip`/`error`/`append` (#67) |
| `language` | enum | `"none"` | **1.0.497** | auto-create FTS nyelve (#65) |
| `text_fields` | string[] | — | **1.0.500** | auto-create extra FTS mezői (#66) |

`fulltext_analyze`:

| Param | Típus | Default | Verzió | Leírás |
|-------|-------|---------|--------|--------|
| `text` | string | — | — | elemzendő szöveg |
| `language`, `accent_folding`, `min_word_length` | — | — | — | explicit tokenizáció-config |
| `collection`, `field` | string | — | **1.0.497** | együtt: örökli az index FtsOptions-ját |

`auto_embed_enable` (v1.0.494): a sikeres válasz `warning` mezővel jelzi, ha
chunk-importált collection-re kapcsolják (verbatim embed határ).

---

## 4. Belső adatszerkezetek (storage-perzisztált)

### `_system.rag` doc shape

```json
{
  "collection": "docs",
  "embedding_field": "embedding",
  "text_field": "content",
  "text_fields": ["content", "title", "customer"],
  "provider": "ollama",
  "language": "hungarian",
  "dimension": 768,
  "created_at": "2026-05-27T19:08:37Z"
}
```

A `text_fields` 1.0.500 előtti configokon hiányzik → `#[serde(default)]` üres
vektor → `effective_text_fields()` fallback `[text_field]`.

### Chunk doc shape (a RAG collection-ben)

```json
{
  "_id": <auto>,
  "doc_id": "arajanlat-001",
  "chunk_index": 3, "chunk_total": 12,
  "start_char": 1024, "end_char": 1156,
  "content": "<eredeti chunk-szöveg, esetleg overlap-pal és táblafejléccel a folytatáshoz>",
  "embedding": [/*Vec<f32>*/],
  "title": "Árajánlat — BKV Zrt",       // opcionális
  "section": "Tételek",                  // chunk-saját heading, ha van
  "heading_level": 2,
  "section_path": ["Árlista", "PEF-széria"],
  "table_header": "| Megnevezés | Ár |\n|---|---|"  // csak tábla-folytatás chunkon
}
```

---

## 5. Konzisztencia & visszamenőleges kompatibilitás

| Aspektus | Megoldás |
|---------|----------|
| Legacy RagConfig (text_fields nélkül) | `#[serde(default)]` → `effective_text_fields()` fallback `[text_field]` |
| Legacy chunk (table_header nélkül) | merge dedup csak akkor fut, ha a mező jelen van — régi chunkok érintetlenek |
| MCP ↔ Rhai konzisztencia | `RagConfig` szerializáció azonos doc shape; a Rhai `get_rag_config` 5-tuple-t ad ami azonos formátumot olvas; `db_hybrid_search` és `db_rag_stats` ugyanúgy default-ol és jelez |
| Generikus auto-embed határ | Szándékosan verbatim (saját szerződése); `auto_embed_enable` warning ha chunk-importált collection-re kapcsolják |
| Idempotencia auto-UUID-nél | Auto-UUID minden hívásnál új doc → az `if_exists` nem lép életbe; az idempotenciához `doc_id` *kell* |
| User metadata védelem | `RESERVED_METADATA_KEYS` mind a 3 import-úton közös (`helpers.rs`) — chunk-tracking + `table_header` védve |

---

## 6. Hibakezelési invariánsok

1. **Embedding-hiba** (provider 429, hálózat, NaN): a DB-mutáció ELŐTT történik → 0 chunk perzisztálódik. Retry biztonságos.
2. **Insert-hiba**: a régi chunkok érintetlenek, új chunkok sem perzisztálódnak (insert_many atomi a sikerre).
3. **Delete-hiba** (insert UTÁN): átmeneti duplikátum a `doc_id` alatt; a következő `Replace` import önjavít. Auto-UUID `doc_id`-nál (sosem reused) tartós dup — ezért a doku ajánlja explicit `doc_id`-t.
4. **Hiányzó collection** (`find` → `CollectionNotFound`): `insert_chunks_idempotent` explicit kezeli (empty existing_ids).
5. **Hiányzó FTS index egy listázott mezőre**: a default keresés nem hasal el, csak kimarad (intersect with indexed); explicit `text_fields` esetén a `fulltext_search_multi` hibája propagálódik a hívóhoz.
6. **Nem-egyező nyelv** (`rag_document_import` létező collection-ön): `language_ignored: true` + warn; nem csendes elhagyás.
7. **`fulltext_analyze` fél-pár** (csak `collection` vagy csak `field`): `invalid_params` hiba; nincs silent fallback.

---

## 7. Operatív ajánlások

**RAG collection inicializálás (egyszer)**:
```json
{"name":"rag_collection_create","arguments":{
  "collection":"rdocs",
  "language":"hungarian",
  "text_fields":["title","customer"]
}}
```
→ FTS index `content`-re, `title`-re, `customer`-re, magyar stemmeléssel. A
config tárolja a listát → minden későbbi `hybrid_search` mindhárom mezőre keres
default-ban.

**Dokumentum-import retry-biztosan**:
```json
{"name":"rag_document_import","arguments":{
  "collection":"rdocs",
  "doc_id":"arajanlat-2026-001",
  "content":"...",
  "title":"BKV árajánlat",
  "if_exists":"replace"
}}
```
→ idempotens; ismételt hívás ugyanazzal a `doc_id`-val lecseréli a chunkokat.

**Olcsó "csak ha még nincs"**:
```json
{"...":"...", "if_exists":"skip"}
```
→ az embedding-lépés is kimarad ha a `doc_id` már létezik.

**Multi-field keresés (automatikus)**:
```json
{"name":"hybrid_search","arguments":{"collection":"rdocs","query":"BKV Zrt árajánlat"}}
```
→ a config `text_fields`-éből (intersect indexed) mindhárom mezőre keres.
Explicit `text_fields` átadása csak akkor kell, ha el akarsz térni.

**Index-tokenizáció debug**:
```json
{"name":"fulltext_analyze","arguments":{
  "text":"fékpadon fékpadot fékpad",
  "collection":"rdocs",
  "field":"content"
}}
```
→ `inherited_from_index: true`, hungarian stemmer alapján közös stem (`fekp`).

---

## 8. Verzió-szerinti hozzájárulások

| Verzió | Issue | Lényeg | Kulcs file |
|--------|-------|--------|------------|
| 1.0.494 | — | Contextual chunk embedding: `build_embed_text` (breadcrumb + tábla-lapítás); fenced-code heading fix; `db_rag_import` mezőkészlet egységesítve (section_path, heading_level); `auto_embed_enable` verbatim-warning | `chunking/mod.rs`, `chunking/markdown.rs`, `tools/auto_embed.rs`, 3 import |
| 1.0.495 | #67 | Idempotens chunk import: `if_exists` param, `insert_chunks_idempotent` helper, safe ordering | `tools/helpers.rs`, 3 import |
| 1.0.496 | #64 | Determinisztikus `pick_text_field` (HashMap-order bug fix) | `tools/hybrid.rs` |
| 1.0.497 | #65 | RAG fulltext language param; `fulltext_analyze` `collection+field` öröklés; `adapter.get_fulltext_index_options` | `tools/rag.rs`, `tools/index.rs`, `adapter.rs` |
| 1.0.498 | review | `embed_document` reserved-key filter (metadata.doc_id korábbi kiskapu); shared `RESERVED_METADATA_KEYS`; `should_skip_before_embedding` pre-check; nem-csendes language-drop | `tools/helpers.rs`, 3 import |
| 1.0.499 | #63 | Tábla-fejléc propagáció (nyers-slice detect, `table_header` mező, heading-reset); merge-dedup pontos prefix-vágással; shared tábla-helperek | `chunking/markdown.rs`, `chunking/mod.rs`, `tools/fusion.rs` |
| 1.0.500 | #66 | Multi-field FTS (`text_fields` param + `RagConfig.text_fields`); auto multi-field `hybrid_search` default; intersect with indexed (robusztusság); Rhai konzisztencia (5-tuple, `db_hybrid_search`, `db_rag_stats`); shared `resolve_search_text_fields` és `resolve_fulltext_fields` | `tools/rag.rs`, `tools/hybrid.rs`, `scripting/db_functions.rs`, `params.rs`, `definitions/rag.rs` |

---

## 9. Nyitott pontok

- **#68 / #69** — API-alak konzisztencia (`fulltext_search` ≠ `hybrid_search` hit-shape; `group_by_document` doc-szintű mező-emelés). Még nyitott.
- **`test_write_lock_timeout` flake** (`ironbase-core/src/database/mod.rs:1260`) — szűk <200ms felső korlát terhelt macOS CI-n túllő. NEM RAG-munka okozta; lazítandó. Részletek: `memory/todo-flaky-write-lock-timeout-test.md`.
- **`rag_document_import` text_fields létező collection-ön**: jelenleg `tracing::warn!` + ignorálva. Ha valódi szükség mutatkozik, későbbi feature: meglévő collection FTS-bővítése a `text_fields` mezőkkel (jelenleg `index_create_fulltext` + manual `RagConfig` patch szükséges).
