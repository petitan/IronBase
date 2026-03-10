# Embedding Config Unification

**Dátum:** 2026-03-10
**Státusz:** Terv (nem implementált)
**Prioritás:** Közepes — manuális workaround létezik, de a divergencia ismétlődhet

---

## 1. Probléma

Az IronBase MCP szerveren **két független embedding konfigurációs rendszer** él egymás mellett, amelyek egymásról nem tudnak:

| | RAG Config | Auto-Embed Config |
|---|---|---|
| **Tároló** | `_system.rag` collection (JSON dokumentum) | Collection metadata (`CollectionMetadata.auto_embedding_config`) |
| **Ki írja** | `rag_collection_create` tool | `auto_embed_enable` tool |
| **Ki olvassa** | `rag_document_import`, `hybrid_search`, `rag_collection_stats`, Rhai `db_rag_*` | `auto_embed_status`, startup `check_model_changes_and_reembed`, insert hook |
| **Mezők** | collection, embedding_field, text_field, provider, language, dimension, created_at | enabled, source_field, target_field, provider, model, dimension, skip_if_exists, chunking, preprocessing_version |
| **Életciklus** | Csak explicit `rag_collection_create` frissíti | `auto_embed_enable` + startup model change detection frissíti |

### Konkrét incidens (2026-03-10)

1. Collection eredetileg `rag_collection_create`-tel lett létrehozva (FastText, 300 dim)
2. Később `auto_embed_enable`-lel átváltottunk vLLM/BGE-M3-ra (1024 dim)
3. Az `auto_embed_enable` frissítette az `AutoEmbeddingConfig`-ot → **helyes** (1024/vllm)
4. A `_system.rag`-ban lévő `RagConfig` érintetlen maradt → **stale** (300/fasttext)
5. `rag_document_import` a `RagConfig`-ból olvasta a dimension-t → **dimension mismatch hiba**
6. `rag_collection_stats` stale adatot mutatott

**Workaround:** Kézi `rag_collection_create` hívás az aktuális paraméterekkel.

### Miért két rendszer?

Történelmi okok:
- **RAG rendszer** (v1.0.3xx): Chunking + fulltext + vector, `rag_collection_create` → `rag_document_import` workflow
- **Auto-embed** (v1.0.3xx): Bármely collection automatikus embedding-je insert-kor, nincs chunking kontextus

A két feature különböző use-case-re készült, de **a provider/dimension/field konfiguráció átfed**.

---

## 2. Hol olvassák a config-okat?

### RAG Config olvasók (`get_rag_config`)

| Fájl | Funkció | Mit olvas |
|------|---------|-----------|
| `hybrid.rs:112,162` | `handle_hybrid_search` | embedding_field, text_field, provider |
| `rag.rs:264` | `handle_rag_document_import` | embedding_field, text_field, provider, dimension (validáció!) |
| `rag.rs:442` | `handle_rag_collection_stats` | Teljes config megjelenítés |
| `db_functions.rs:1390` | Rhai `db_rag_import` | embedding_field, text_field, provider |
| `db_functions.rs:1634` | Rhai `db_hybrid_search` | embedding_field, text_field, provider |
| `db_functions.rs:1883` | Rhai `db_rag_stats` | Teljes config |

### Auto-Embed Config olvasók

| Fájl | Funkció | Mit olvas |
|------|---------|-----------|
| `hybrid.rs:156-160` | `handle_hybrid_search` | provider (prioritás #2) |
| `rag.rs:258-262` | `handle_rag_document_import` | provider (prioritás #2) |
| `auto_embed.rs:691+` | `check_model_changes_and_reembed` | model, preprocessing_version, provider, dimension |
| `auto_embed.rs:277` | `handle_auto_embed_status` | Teljes config |
| `crud.rs` | insert hook | enabled, source_field, target_field, provider |
| `db_functions.rs` | Rhai hybrid/import | provider (prioritás #2) |

### Provider resolution sorrend (jelenleg)

```
1. User explicit provider paraméter
2. AutoEmbeddingConfig.provider     ← FRISS (auto_embed_enable frissíti)
3. RagConfig.provider               ← LEHET STALE!
4. manager.default_provider_name()
```

A probléma: ha nincs explicit provider és nincs auto-embed config, a stale RAG config dönt.

---

## 3. Opciók

### Opció A: Szinkronizáció (kis változás)

**`auto_embed_enable` frissíti a RAG configot is (ha létezik).**

```rust
// auto_embed.rs — handle_auto_embed_enable végén:
if let Ok(Some(mut rag_cfg)) = get_rag_config(adapter, &p.collection) {
    rag_cfg.provider = p.provider.clone();
    rag_cfg.dimension = dimension;
    rag_cfg.embedding_field = p.target_field.clone();  // target_field = embedding_field
    save_rag_config(adapter, &rag_cfg)?;
}
```

Hasonlóan: `rag_collection_create` frissíti az auto-embed configot is (ha létezik).

| Pro | Kontra |
|-----|--------|
| Minimális kódváltozás | Két config marad, szinkronban kell tartani |
| Backward compatible | Új tool-ok is figyelniük kell mindkettőre |
| Gyorsan implementálható | Nem oldja meg a koncepcionális problémát |

**Kockázat:** Új feature hozzáadásakor elfelejtik a szinkronizációt → újra divergál.

### Opció B: RAG Config eliminálás (közepes változás)

**A `_system.rag` collection megszűnik. Minden adat az `AutoEmbeddingConfig`-ba kerül.**

Hiányzó mezők az `AutoEmbeddingConfig`-ból, amiket át kell venni:
- `language` (fulltext index nyelvhez)
- `text_field` (jelenleg `source_field` az auto-embed-ben, de a RAG-ban `text_field`)

```rust
pub struct AutoEmbeddingConfig {
    // ... meglévő mezők ...
    pub language: Option<String>,        // ÚJ: fulltext index nyelv
    // source_field ≈ text_field (átnevezés vagy alias)
}
```

| Pro | Kontra |
|-----|--------|
| Single source of truth | `_system.rag` migráció kell |
| Nincs szinkronizációs probléma | AutoEmbeddingConfig ironbase-core-ban van (a language MCP-specifikus) |
| Tisztább architektúra | Breaking change: `rag_collection_create` viselkedése változik |

**Migráció:** Startup hook: ha `_system.rag`-ban van config és `AutoEmbeddingConfig`-ban nincs → átmásolás, majd `_system.rag` doc törlése.

### Opció C: Unified Embedding Config (nagy változás)

**Új, egységes `EmbeddingConfig` struct a core-ban, ami mindkét use-case-t lefedi.**

```rust
pub struct EmbeddingConfig {
    pub enabled: bool,
    pub provider: String,
    pub model: Option<String>,
    pub dimension: Option<usize>,
    pub preprocessing_version: Option<String>,

    // Mezők
    pub source_field: String,       // Honnan olvassa a szöveget
    pub embedding_field: String,    // Hova írja a vektort

    // RAG-specifikus
    pub language: Option<String>,
    pub chunking: Option<ChunkingConfig>,

    // Auto-embed-specifikus
    pub skip_if_exists: bool,
}
```

| Pro | Kontra |
|-----|--------|
| Teljes egységesítés | Legnagyobb implementációs költség |
| Jövőbiztos | Breaking change mindkét API-ban |
| Egy helyen konfigurálható | Migráció + tesztek |

---

## 4. Javaslat

**Opció A (szinkronizáció) azonnali fix-ként, Opció B (RAG config eliminálás) hosszú távon.**

### Fázis 1: Szinkronizáció (azonnali)

1. `auto_embed_enable`: ha `_system.rag`-ban létezik config → frissíti a provider/dimension/embedding_field mezőket
2. `rag_collection_create`: ha `AutoEmbeddingConfig` létezik → frissíti a provider/dimension mezőket
3. `check_model_changes_and_reembed`: dimension/provider változáskor a RAG configot is frissíti

**Érintett fájlok:**
- `mcp-server/src/tools/auto_embed.rs` — `handle_auto_embed_enable` végére szinkron logika
- `mcp-server/src/tools/rag.rs` — `handle_rag_collection_create` végére szinkron logika
- `mcp-server/src/tools/auto_embed.rs` — `check_model_changes_and_reembed` kiegészítés

### Fázis 2: RAG Config eliminálás (következő major)

1. `AutoEmbeddingConfig` bővítése `language` mezővel
2. `_system.rag` → `AutoEmbeddingConfig` migráció startup hook-ban
3. `get_rag_config()` hívók átírása `get_auto_embedding_config()`-ra
4. `_system.rag` collection deprecated, majd eltávolítva
5. `rag_collection_create` → `auto_embed_enable` wrapper (backward compat)

### Nem érintett

- `hybrid_search` pipeline (STEP 1-7) — csak a config olvasás változik
- `rag_document_import` chunking logika — változatlan
- ironbase-core motor — változatlan (az `AutoEmbeddingConfig` struct bővül, de `#[serde(default)]`)

---

## 5. Mező mapping

| RAG Config | Auto-Embed Config | Megjegyzés |
|---|---|---|
| `collection` | (implicit, collection-hez kötött) | Auto-embed a collection metadata-ban él |
| `embedding_field` | `target_field` | Ugyanaz a koncepció, más név |
| `text_field` | `source_field` | Ugyanaz a koncepció, más név |
| `provider` | `provider` | Azonos |
| `dimension` | `dimension` | Azonos (RAG: usize, Auto: Option\<usize\>) |
| `language` | — | Csak RAG-ban, fulltext index nyelvhez |
| `created_at` | — | Csak RAG-ban, informatív |
| — | `enabled` | Csak Auto-ban, be/ki kapcsoló |
| — | `model` | Csak Auto-ban, modell név |
| — | `skip_if_exists` | Csak Auto-ban, backfill viselkedés |
| — | `chunking` | Csak Auto-ban (de RAG import-nál is van chunking param) |
| — | `preprocessing_version` | Csak Auto-ban, startup detection |

---

## 6. Kockázatok

| Kockázat | Hatás | Mitigáció |
|----------|-------|-----------|
| Fázis 1 szinkron logika elfelejtődik új tool-nál | Újra divergál | Kód komment + teszt |
| Fázis 2 migráció adatvesztés | Stale config marad | Startup migration + logging |
| `source_field` vs `text_field` névütközés | API zavar | Alias vagy egyértelmű dokumentáció |
| `language` az ironbase-core-ban nem tartozik oda | Réteg sértés | `Option<String>`, MCP tölti ki |

---

## 7. Teszt terv

### Fázis 1 tesztek
- `auto_embed_enable` → ellenőrizd hogy `_system.rag` frissült
- `rag_collection_create` → ellenőrizd hogy `AutoEmbeddingConfig` frissült
- Provider váltás `auto_embed_enable`-lel → `rag_collection_stats` helyes adatot mutat
- Dimension váltás → mindkét config tükrözi

### Fázis 2 tesztek
- Startup migration: régi `_system.rag` → `AutoEmbeddingConfig` átmásolás
- `get_rag_config` backward compat (deprecated path)
- `rag_collection_create` → `auto_embed_enable` delegálás
