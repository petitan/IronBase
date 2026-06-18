# IronBase — Score Fusion / hibrid keresés architektúra

RRF score fusion, reranking, adjacent-chunk merge, MMR, fulltext mode,
document-level AND, multi-field FTS, search-mode preset-ek. Kivonva a
CLAUDE.md-ből (v1.0.544, 2026-06-18). Operatív RAG-referencia: `docs/RAG_PIPELINE.md`.

**Döntés: Score fusion MCP tool szinten marad, NEM query operátor.**

**Indoklás:**
- Query operátorok (`OperatorMatcher`) = stateless boolean predikátumok: `fn(doc_value, filter_value) -> bool`
- Score fusion = ranked retrieval: score-okat ad vissza, nem igaz/hamis
- Index hozzáférés szükséges (fulltext + HNSW), de operátorok stateless-ek

**Implementáció (2026-02-18 — unified):**

| Felület | Fájl | Algoritmus |
|------|------|-----------|
| `search` MCP tool (intent-only) + Rhai `db_hybrid_search` (paraméterezhető) | `mcp-server/src/tools/hybrid.rs` (közös motor: `retrieve_and_fuse`) | RRF fusion (explicit vector VAGY auto-embed) |

> A `hybrid_search` MCP tool megszűnt (v1.0.504, `search` váltotta). Az alábbi paraméterek
> (mode, match_scope, text_fields, weights, rrf_k, mmr_lambda…) a **közös `hybrid.rs` motoré**:
> a Rhai `db_hybrid_search` teszi ki őket, a `search` MCP tool NEM (server-owned, intent-only).

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

**Fulltext mode paraméter (45f74bf7, #47, cd70eae4 #61, BREAKING: diszjunktív default):**
- `mode`: **`"or"` (default, v1.0.537+)** = bármely szó elég, BM25 score rangsorol (iparági standard diszjunktív retrieval); `"and"` = MINDEN szó kell (opt-in precízió-szűrő)
- Elérhető: Rhai `db_hybrid_search` + `fulltext_search` MCP tool — **mindkettő OR default** (v1.0.537+; **korábban AND volt v1.0.389–536**). A `search` MCP tool intent-only (a motor diszjunktívan keres).
- **BREAKING CHANGE indok:** a bináris AND-overlay a BM25 fölött non-standard RRF-ben — eldobja a graded BM25 scoringot és a top-1 BM25 részleges-egyezést (keyword_buries). A standard: diszjunktív lane + BM25/RRF/rerank dönt. Mérés: specific Recall@10 0.625→0.782. Részletek: `memory/search-fuzzy-coverage-fusion-2026-06-16`.
- `mode` hiánya = `"or"` (v1.0.537+)

| Fájl | Változás |
|------|----------|
| `params.rs` | `pub mode: Option<String>` mindkét struct-ban |
| `definitions/index.rs` | `"mode"` schema entry (default: **"or"**, v1.0.537+) |
| `fusion.rs` | `resolve_and_mode(mode) = mode == Some("and")` (None→OR, v1.0.537+; egyetlen döntéspont, mindhárom hívóra) |

**Document-level AND mode (0f208d41, #56, cd70eae4 #61):**
- `match_scope`: `"document"` (default) = szavak a dokumentum különböző chunkjaiban is lehetnek, `"chunk"` = minden szó egyetlen chunkban. **Csak `mode="and"` mellett él** (a default OR-ban inert, v1.0.537+).
- **Keresési logika (mode=and):** kvalifikáció (AND) → minden query token megjelenik a dokumentumban; chunk retrieval (OR) → bármely tokent tartalmazó chunk visszajön
- Korábban default "chunk" volt, de 3+ szavas query-knél túl szigorú (egyetlen chunk ritkán tartalmaz minden tokent)
- Aktiválódik: **explicit `mode="and"`** + `match_scope` != `"chunk"` (v1.0.537+, korábban a default AND miatt automatikus volt)
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
- Elérhető: Rhai `db_hybrid_search` (a `fulltext_search` MCP tool már korábban támogatta `fields` néven). A `search` MCP tool a RAG-config `text_fields`-éből oldja fel, server-owned.
- `text_fields` felülírja a `text_field` (string) paramétert ha mindkettő megadva
- Előfeltétel: minden megadott mezőn fulltext index kell (`index_create` `type:"fulltext"`)
- Backward compatible: `text_fields` hiánya = single-field (régi viselkedés)

```json
{"collection": "docs", "query": "Juhai ajánlat",
 "text_fields": ["content_text", "title", "customer"]}
```

**Search mode presets (220679f3):**
- `search_mode`: `"balanced"` (default), `"semantic"`, `"keyword"` — LLM-barát nevesített preset a numerikus weight-ek helyett
- Elérhető: Rhai `db_hybrid_search` (a `search` MCP tool nem teszi ki — server-owned weights)
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

**Közös fusion pipeline (hybrid.rs — `search` MCP tool + Rhai `db_hybrid_search`):**
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
MCP tools (search)                           → score fusion, ranked retrieval
```
