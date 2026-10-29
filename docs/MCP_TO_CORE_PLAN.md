# MCP → core: a szerverbe nem való logika áthelyezése

**Állapot:** a döntések (D1–D5) jóváhagyva (2026-10-29). Kész: 1. pont (háttér-karbantartás),
a halott `preprocessing/` törlése, 2. pont (qualification), 3. pont 1–2. lépése
(`ironbase_core::search::hybrid`: rerank, merge, MMR, `hybrid_fuse`, `hybrid_group_documents`).
Hátra: 3/3. (Python-kötés) és a 4. pont.

## Miért

Az MCP-szerverben él néhány olyan algoritmus, amely nem kiszolgálói feladat (nem
hálózat, jogosultság, job, prompt), hanem az adatbázis viselkedése. Két következménye
van:

- **Két példány, eltérő viselkedés.** A karbantartás ütemezése két másolatban élt
  (HTTP: tokio-taskok, stdio: szál), és a stdio-példányból kimaradt a periodikus
  checkpoint (1. pont, javítva).
- **A beágyazott felhasználó nem kapja meg.** Hibrid keresés, dokumentum-szintű AND,
  chunkolás csak MCP-n át érhető el; a Python-kötésben vektoros keresés sincs.

Alapelv: **egy implementáció, a motorban**; az MCP vékony réteg lesz (paraméter-JSON →
típusos motor-API, eredmény → MCP-kontraktus). Minden lépés **viselkedésmegtartó**
áthelyezés, és a meglévő tesztek változatlanul futnak tovább.

## 1. Háttér-karbantartás — kész

`ironbase_core::spawn_maintenance(target, MaintenanceConfig)`
(`ironbase-core/src/database/background.rs`): periodikus checkpoint (60 s) és
bloat alapú compaction, közös politikával (`AutoCompactPolicy`, a korábbi
`[compaction]` alapértékekkel). A `MaintenanceTarget` trait mögött a cél lehet maga a
`DatabaseCore` (beágyazott: a compaction a karbantartó szálon fut) vagy a szerver
(`compaction::ServerMaintenance`: jobként futó compaction, `switch_database`-t követi).
Az MCP HTTP- és stdio-módja ugyanazt indítja, és leálláskor a zárás **előtt** állítja
le. Python: `IronBase.start_maintenance(...)`, a `close()` leállítja.

## 2. Dokumentum-szintű AND („qualification") → `CollectionCore` — kész

**Megvalósítva:** `CollectionCore::fulltext_qualify_documents(fields, query) ->
Result<DocumentQualification>` (`collection_core/search.rs`), `to_filter()`-rel. Az MCP
`apply_document_qualification` és a Rhai `db_fulltext_search` ezt hívja; a hat primitív
(`fulltext_tokenize_query`, …) a motorból és az adapterből törölve. A `find`-alapú
fallback halott kód volt (a `parent_doc_id_field` sehol nem állítódik be, így az ága nem
futott; a régi RAG-indexek mappingját az induló migráció tölti fel) — nem került át.

Az eredeti terv:

**Most:** `adapter.rs:1882` (`fulltext_qualify_documents_fast`) és `fusion.rs:565–700`
(`qualify_documents`, `apply_document_qualification`, két fallback: posting-list és
`find`-alapú). A motor öt alacsony szintű függvényt tesz publikussá csak ehhez
(`fulltext_tokenize_query`, `…_posting_counts`, `…_token_chunk_ids`,
`…_resolve_chunk_doc_ids`, `…_has_chunk_doc_mapping`).

**Cél:** `CollectionCore::fulltext_qualify_documents(fields: &[&str], query: &str)
-> Result<Qualification>` (`Qualification::{NotRequired, Docs(Vec<String>)}`), a
gyors út és mindkét fallback egy helyen a `collection_core/search.rs` mellett.

**Lépések**
1. A logika áthelyezése a motorba, a `fusion.rs` qualification-tesztjeivel együtt.
2. Az adapter és a `fusion.rs` az új függvényt hívja; a régi kód törlése.
3. Az öt primitív `pub(crate)` lesz (publikus API szűkül; csak az MCP használta).

**Kockázat:** kicsi; a viselkedést a meglévő tesztek és a `search` P8-tesztjei fogják.
**Méret:** S (1 PR).

## 3. Hibrid keresés → motor-modul

**Most:** `tools/hybrid.rs` + `tools/fusion.rs` (~3 800 sor): `retrieve_and_fuse`
(vektor- és fulltext-lane, RRF), rerank, MMR, szomszédos chunk-összevonás,
dokumentum-csoportosítás (`build_doc_groups`). Az adapterből csak vékony
motor-hívásokat használ (`fulltext_search[_multi]`, `vector_search_with_options`,
`find`, a 2. pont primitívjei). Az embeddinget (`EmbeddingManager`) és a RAG-configot
(`get_rag_config`: melyik szöveg- és vektormező) az MCP adja.

**Cél:** `ironbase-core/src/search/hybrid/` — típusos API:
`CollectionCore::hybrid_search(&HybridQuery) -> Result<HybridResult>`, ahol a
`HybridQuery` a **kész query-vektort** kapja (vagy `None`: csak BM25), explicit
mezőkkel, súlyokkal, `rrf_k`, rerank/MMR/merge/group kapcsolókkal; az eredmény
`FusedResult`-ok vagy `DocGroup`-ok.

**Ami az MCP-ben marad:** embedding (provider, query-prefix, cache), a RAG-config
feloldása (mely mezők), a `search` válaszkontraktusa és budgetje (`contract.rs`), a
JSON-paraméterek (`HybridSearchParams`) → `HybridQuery` fordítás, Rhai-glue.

**Lépések (3 PR)**
1. **Tiszta függvények** — *kész:* `ironbase_core::search::hybrid` (`FusedResult`,
   `rerank_results`, `merge_adjacent_chunks/values`, `mmr_reorder`, `apply_projection`,
   `id_to_string`) a 30 tesztjükkel; az MCP a motorét hívja, viselkedés változatlan.
   Az RRF-pontozás és a csoportosítás a `retrieve_and_fuse`/`build_doc_groups` része, a
   2. lépéssel megy.
2. *Kész:* `CollectionCore::hybrid_fuse(&HybridQuery, ctx)` és `hybrid_group_documents`
   (`search/hybrid/fuse.rs`, `group.rs`), a dokumentum-szintű AND orkesztrációja
   `CollectionCore::fulltext_document_scope`. Ellenőrzés: ideiglenes aranyfájl-harness (240
   eset: 10 lekérdezés × 24 paraméterkombináció, flat + grouped) — a kimenet a váltás előtt
   és után bitre azonos (normalizált JSON). Az eredeti terv:
   **`retrieve_and_fuse` + `build_doc_groups`** a motorba, `HybridQuery`-vel; az MCP
   `search` és a Rhai `db_hybrid_search` erre vált. Ellenőrzés: a
   `mcp-server/eval/measure_lanes.py` `mirror`-konfigurációja és a `search`
   ugyanazokat a dokumentumokat adja a váltás előtt és után (aranyfájl a címkeszet
   62 kérdésére), plusz a meglévő `hybrid.rs`-tesztek.
3. **Kötések:** Python `Collection.vector_search(vector, field, limit)` (ma hiányzik) és
   `Collection.hybrid_search(query, vector=None, ...)`; C# a következő körben, igény
   szerint.

**Kockázat:** közepes — a legnagyobb kódmozgás; a `search` minősége a tét. Ezért a 2.
lépés aranyfájl-összevetéssel zárul, és a régi kód csak utána törlődik.
**Méret:** L.

## 4. Chunkolás → motor (opcionális `rag` feature)

**Most:** `mcp-server/src/chunking/` (~1 200 sor; `text_splitter` crate, markdown- és
szövegdarabolás, kontextus-prefix) és az import-folyamat a `tools/rag.rs`-ben
(darabolás → embedding → írás chunkonként, `if_exists` kezelés).

**Cél:** `ironbase-core` `rag` cargo feature: `chunk_document(text, ChunkOptions) ->
Vec<Chunk>` és a kontextus-prefix; az **import** szétválik: darabolás és
chunk-dokumentum-építés a motorban, az embedding és a provider az MCP-ben / a
hívónál (Python: a hívó adja a vektorokat, vagy embedding nélkül importál).

**Lépések (2 PR)**
1. `chunking/` áthelyezése a motor `rag` feature-je mögé, tesztjeivel; az MCP a
   feature-rel fordítja a motort.
2. `rag_document_import` a motor darabolójára vált; Python `chunk_text(...)` és
   `rag_import(collection, text, embeddings=None, ...)`.

**Kockázat:** kicsi–közepes (determinisztikus kód, jó tesztek); a feature-kapcsoló
miatt a motor függőségei alapból nem nőnek.
**Méret:** M.

## Sorrend

1. *(Gyors, külön PR)* a halott `mcp-server/src/tools/preprocessing/` (569 sor,
   sehonnan nem hívott magyar query-előfeldolgozó, a motor stemmerétől eltérő
   szabályokkal) törlése.
2. 2. pont (qualification) — a 3. pont erre épül.
3. 3. pont, három PR-ben.
4. 4. pont, két PR-ben.

Minden PR: verzióemelés, CHANGELOG, `mcp-server` + `ironbase-core` tesztek, clippy;
a 3/2. lépés után `measure_lanes.py`-összevetés a szerver hostján.

## Döntések (a CLAUDE.md szerint jóváhagyást igényelnek)

| # | Döntés | Javaslat |
|---|---|---|
| D1 | Új publikus motor-API: `fulltext_qualify_documents`, `search::hybrid`, `rag` modul | igen; a 2. pont az öt primitívet `pub(crate)`-re szűkíti |
| D2 | Új motor-függőség: `text_splitter` | csak a `rag` feature mögött (alapból ki) |
| D3 | A RAG-config (mely mezők) helye | marad az MCP-ben; a motor-API explicit mezőket kap |
| D4 | Python API bővítése (`vector_search`, `hybrid_search`, `chunk_text`, `rag_import`) | igen, a 3/3. és 4/2. lépésben |
| D5 | C#-kötés | a Python után, igény szerint |
