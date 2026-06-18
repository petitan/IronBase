# IronBase — Dokumentált bugok (referencia)

Korábbi kritikus bugok katalógusa (tünet → root cause → fix). Kivonva a
CLAUDE.md-ből (v1.0.544, 2026-06-18) a kontextus-ablak tehermentesítésére.
Részletes elemzések: `memory/critical-bugs.md`.

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
| **BM25 fails on 4+ words** | cd70eae4, #61 | `hybrid_search` text_score=0 multi-word query-knél | `fulltext_search` OR default vs `hybrid_search` AND default inkonzisztencia + chunk-level AND túl szigorú (1 chunk ritkán tartalmaz 4+ stemelt tokent) | Konzisztens AND default mindkettőnél + `match_scope` default "document" (dok szintű AND kvalifikáció, chunk szintű OR retrieval). ⚠️ **SUPERSEDED v1.0.537 (#109):** az AND default OR-ra váltott (diszjunktív, iparági standard) — lásd a "Fulltext mode paraméter" szekciót lentebb; ez a sor a #61-korabeli állapotot rögzíti |
| **RAG import not idempotent** | #67 | `rag_document_import` retry-nál duplikált chunkok (éles: ~11K duplikátum) | `insert_many` előtt nincs delete-by-doc_id → ugyanazon doc_id újra-importja hozzáfűz | `if_exists` param (default `replace`) + közös `insert_chunks_idempotent` helper (`helpers.rs`) mind a 3 import-úton (rag/embed_document/db_rag_import). Safe ordering: capture old _ids (proj `_id`) → insert new → delete old → sosem veszít adatot |
| **merge_chunks title concat** | #64 | `merge_chunks=true` + `text_fields`: a `title` N-szer ismételve, a `content` nem merge-elődik | rag_config nélkül az `effective_text_field` = `get_fulltext_field_names().next()` — `HashMap` sorrend nemdeterminisztikus → `title`-re oldódhat, `merge_adjacent_chunks` azt fűzi | `pick_text_field()` (`hybrid.rs`): determinisztikus, `content`-preferáló mező-feloldás (különben lexikálisan első) |
| **fulltext_search shape divergence** | #68 | `fulltext_search` hit-alak ≠ `hybrid_search` (`{document:{...},score,matched_tokens}` vs lapos `{<doc fields>, _final_score, _text_score}`) → kliensnek 2 parser | **BREAKING (v1.0.501)** — `fulltext_search` mostantól lapos: doc-mezők top szinten, `_score`/`_matched_tokens`/`_highlights` `_`-prefix metadata. Mindkét tool egy parserrel olvasható |
| **group_by_document doc fields** | #69 | Csoportosított hibrid-keresés a `title`/`customer`/`year`-t `chunks[0]`-ban hagyta, nem emelte csoport-szintre | **BREAKING (v1.0.501)** — `lift_common_fields()` (`hybrid.rs`): a chunkok közt **azonos értékű** kulcsok automatikusan a csoport top szintjére kerülnek és **kikerülnek a chunk-okból**. Generikus, nincs hardcode mező-lista |
| **RAG single-field FTS** | #66 | `rag_*` csak a `content`-re hozott FTS indexet; `title`/`customer` indexeletlen → multi-field `hybrid_search` csendben degradál | egyetlen `create_fulltext_index` a `text_field`-re; `RagConfig` csak single `text_field`-et tárolt | `text_fields` param (`rag_collection_create`/`rag_document_import`/Rhai) → FTS minden mezőre, `RagConfig.text_fields`-be mentve (`#[serde(default)]` legacy-safe); `hybrid_search` default-ol rá ha a hívó nem ad explicit `text_fields`-et (`resolve_fulltext_fields`, `effective_text_fields`) |
| **Markdown table fragmentation** | #63 | nagy/blank-line-os tábla darabolásakor a folytatás-chunk elveszti a fejléc+separator sort → értelmezhetetlen értékek | `MarkdownSplitter` a táblát blank-line-nál és méret-limitnél vágja; a header csak az első szeletben marad | Fejléc-propagáció a **nyers (overlap nélküli) slice**-on detektálva (`markdown.rs`: `current_table_header`, heading-nél reset) → folytatás-chunk elé fűzi ÉS `table_header` mezőbe írja; merge-dedup a **mező** alapján vág pontosan (`fusion.rs`, nincs újraszámolás → CRLF/whitespace-safe). Default overlap=100-zal is működik |
| **RAG fulltext no stemming** | #65 | RAG import úton nem lehetett magyar stemmelést kérni; `fulltext_analyze` `"None"`-t jelzett hungarian index mellett | `rag_document_import`/`db_rag_import` hardcode `"none"` (nem volt `language` param); `fulltext_analyze` default `"none"` + nem örökölte az index nyelvét | `language` param a RAG import utakra (→ auto-FTS, default `"none"`) + `fulltext_analyze` opcionális `collection`+`field`-del örökli az index valós FtsOptions-ját (`get_fulltext_index_options`) |

**Részletes bug elemzések:** Lásd `memory/critical-bugs.md` (Lazy Index, read_data boundary, Fulltext candidate limit, Fulltext flush dirty flag, Btree delete not dirty, Workaround-ok)
