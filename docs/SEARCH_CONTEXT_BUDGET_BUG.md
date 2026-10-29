# `search` — a válaszréteg kontextus-budgetje egész dokumentumokat ejt ki

**Állapot:** feltárva és megmérve, **nincs javítva** (v1.0.645).
**Érintett:** `search` MCP tool (a Rhai `db_hybrid_search` NEM érintett).
**Mérés dátuma:** 2026-10-28, lokális `docs.mlite` / `rdocs` (18 987 chunk, 500 dokumentum).

---

## 1. A hiba egy mondatban

A `search` a fúzió után egy 12 000 karakteres kontextus-budgetet érvényesít, és ha egy
dokumentum **első bekezdése** nem fér a maradékba, akkor nem a szöveget rövidíti, hanem a
**teljes dokumentumot kihagyja** a válaszból — miközben a nála rosszabbul rangsorolt, de
rövidebb dokumentumok bekerülnek. A relevancia-sorrend így a válaszban megtörik.

## 2. Hol

| Elem | Hely |
|------|------|
| Budget-konstans | `mcp-server/src/tools/retrieval/contract.rs:29` — `DEFAULT_MAX_CONTEXT_CHARS = 12_000` |
| A kiejtő ág | `contract.rs:133-136` (Pass 1 — breadth) |
| Pass 2 (passage-szintű csonkolás) | `contract.rs:142-155` |
| A jelzés a válaszban | `contract.rs:191-205` — `trimmed`, `dropped_due_to_budget` |
| A passage-méret forrása | `mcp-server/src/tools/fusion.rs:232` — `merge_adjacent_runs` (méretkorlát nélkül) |

A kiejtő ág:

```rust
// Pass 1 — breadth: include each document (relevance order) with its first
// passage if it fits.
for (i, s) in staged.iter_mut().enumerate() {
    let first_len = s.texts[0].len();
    let nothing_yet = included.is_empty();
    if !nothing_yet && used_chars + first_len > DEFAULT_MAX_CONTEXT_CHARS {
        dropped_due_to_budget += 1;
        continue;                 // <-- a TELJES dokumentum kimarad
    }
    used_chars += first_len;
    s.emitted = 1;
    included.push(i);
}
```

A `continue` a dokumentumot ejti, nem a passage-t vágja. A Pass 2 már csak a
**bekerült** dokumentumok további bekezdéseire költ, tehát a kiesett dokumentum
semmilyen formában nem jut vissza.

## 3. Mi okozza

Két, önmagában szándékos viselkedés együttese:

1. **A budget tervezett** — a `search` célja, hogy egy kis modell kontextusába illő
   választ adjon (~12 000 karakter ≈ 3 000 token). A konstans a `2c0e7977` (v1.0.504,
   2026-05-29) óta létezik, a kétlépcsős (breadth/depth) allokáció az `e604e267`
   (#88, 2026-06-04) óta. **Ez a rész nem regresszió.**
2. **A chunk-merge méretkorlát nélküli** — a `v1.0.612` (2026-10-17) kiterjesztette a
   szomszédos chunkok összevonását a csoportosított útra is. A `fusion.rs`-ben nincs
   felső határ a merge-elt szöveg hosszára.

A kettő együtt: a merge tetszőlegesen nagy „első bekezdést" állít elő, a Pass 1 pedig
ezt nem tudja befogadni, és a dokumentumot kiejti. Táblázatos dokumentumoknál, ahol sok
egymást követő chunk illeszkedik a lekérdezésre, a merge nagyságrendekkel növeli a
passage-méretet.

## 4. Bizonyíték

### 4.1 Egy szélsőséges eset

Lekérdezés: `hovány kft szerződési` (`rdocs`), a címkézés szerinti releváns dokumentum:
`statprs-2026-02-01-2026-03-01`.

| Forrás | Eredmény |
|---|---|
| `db_hybrid_search` (OR, grouped, limit 10) | a releváns dokumentum a **2. helyen** |
| `search` (ugyanaz a pipeline + budget) | `count: 4`, `trimmed: true`, `dropped_due_to_budget: 6` — a releváns dokumentum **nincs benne** |

A dokumentum tárolt chunkjai 951–1019 karakteresek (`find` szerint), a merge után az
**első passage 57 267 karakter** (a csoport összesen 211 646). A `nDCG@5` ezen a
lekérdezésen `0.631` → `0.000`.

### 4.2 Reprodukálható szimuláció

Lekérdezés: `4 oszlopos emelő ár`. A Pass 1 logikáját a fúzió kimenetén lejátszva a
szimuláció **bitre ugyanazt a dokumentumhalmazt** adja, mint az élő `search`:

| # | dokumentum | 1. passage | futó összeg | sors |
|--:|---|--:|--:|---|
| 1 | vamos-daniel-… | 702 | 702 | befér |
| 2 | vincze-pap-sandor-… | 707 | 1 409 | befér |
| 3 | janoki-norbert-… | 711 | 2 120 | befér |
| 4 | szabo-attila-… | 165 | 2 285 | befér |
| 5 | aqua-globe-kkt-… | 1 089 | 3 374 | befér |
| 6 | csizmas-csaba-… | 1 031 | 4 405 | befér |
| 7 | petitan-eszkozok-hunep | 829 | 5 234 | befér |
| **8** | **sice-lifting-equipment-export-price-list-05-2025** | **8 891** | 5 234 + 8 891 = **14 125 > 12 000** | **kiesik** |
| 9 | kormoczi-csaba-… | 854 | 6 088 | befér |
| 10 | kundean-mihaly-… | 837 | 6 925 | befér |

A `search` kimenete a fúziós rangsor **sorrendtartó részhalmaza**: a 8. kiesik, a 9. és
10. bekerül, a végső felhasználás 6 925 / 12 000 — vagyis a budgetből **maradt hely**, csak
épp nem annyi, amennyit az a *konkrét* dokumentum egyben kért.

## 5. Mekkora a hatás

A júniusi címkeszet 62 válaszolható lekérdezésén (`labeled_rdocs_curated_v2.json`),
`limit: 10`:

| Mutató | Érték |
|---|---|
| `trimmed: true` | **62 / 62** lekérdezés |
| A fúzió jelöltjeiből nem jut el a kliensig | **198 / 620 (32%)** |
| Ebből releváns dokumentumot veszít | **34 / 62 lekérdezés (55%)** |
| 12 000-nél nagyobb első passage | 3 / 620 (0,5%), a legnagyobb 57 267 |

A `measure_lanes.py` metodikájával (minden konfiguráció a `search` saját pipeline-ján,
a pool újraítélve `claude-haiku-4-5` bíróval temperature 0-n, páros bootstrap CI +
sign-flip randomizációs teszt Holm-korrekcióval), a `search` és a `mirror` **ugyanaz a
kereső**, egyedül a budget különbözteti meg őket:

| Bucket | Metrika | Δ (`search` − `mirror`) | 95% CI | p (Holm) |
|---|---|--:|---|--:|
| browse (n=44) | nDCG@10 | **−0,1448** | [−0,1906, −0,1025] | **0,0001** |
| browse (n=44) | nDCG@5 | −0,0262 | [−0,0473, −0,0060] | 0,1436 |
| specific (n=18) | nDCG@10 | −0,0539 | [−0,0953, −0,0193] | 0,1250 |
| specific (n=18) | Recall@10 | −0,0843 | [−0,1491, −0,0296] | 0,1250 |

A browse-bucket `nDCG@10` romlása a **teljes lane-összehasonlítás legerősebb hatása** —
nagyobb, mint bármelyik lane- vagy súlyozás-különbség. Vagyis jelenleg a válaszréteg
vágása többet visz el, mint amennyit a keresési stratégia megválasztása nyerhet.

## 6. Amit a hiba NEM

A vágás **nem néma**. A válasz `trimmed: true`-t és `dropped_due_to_budget: <n>`-t ad
(`contract.rs:191-205`), tehát a hívó látja, hogy több találat van a `count`-on túl. A
probléma a **sorrend** megtörése: a kliens nem tudja, hogy épp a rangsor eleje esett ki.

Az sem igaz, hogy a budget önmagában új volna: a 12 000-es korlát és a breadth-first
allokáció már a júniusi mérésnél (v1.0.535) is így működött. Ami új, az a korlátlan
méretű merge, amely a budgetet kiélezi.

## 7. Javítási irányok

Prioritási sorrendben, a legkisebb beavatkozástól:

1. **A Pass 1 csonkoljon, ne ejtsen.** Ha az első passage nem fér a maradékba, a
   dokumentum kerüljön be a doc-szintű mezőivel és egy a maradékra vágott bekezdéssel.
   Egy rövidített passage és a metaadatok többet érnek, mint a semmi, és a rangsor
   érintetlen marad. (Alsó korlát kell, pl. ha a maradék < 200 karakter, akkor valóban
   kimarad.)
2. **A merge kapjon méretkorlátot** (`fusion.rs`): egy merge-elt passage ne nőhessen a
   budget töredékénél nagyobbra, különben egy dokumentum kisajátítaná a keretet.
3. **A budget legyen hívásonként állítható** (`max_context_chars` paraméter), hogy a
   nagyobb kontextusú kliensek ne fizessék meg a kis modellre szabott korlátot.

## 8. Reprodukció

A szerver hostján (a `script_exec` csak loopback hívótól érhető el):

```bash
# 1. A search válasza — figyeld a count / trimmed / dropped_due_to_budget mezőket
curl -s -X POST http://localhost:8080/mcp -H 'Content-Type: application/json' -d '{
  "jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"search",
  "arguments":{"collection":"rdocs","query":"hovány kft szerződési","limit":10}}}'

# 2. Ugyanaz a pipeline budget nélkül — itt a releváns dokumentum a 2. helyen áll
curl -s -X POST http://localhost:8080/mcp -H 'Content-Type: application/json' -d '{
  "jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"script_exec",
  "arguments":{"code":"let r = db_hybrid_search(\"rdocs\", \"hovány kft szerződési\", #{limit: 10, group_by_document: true, mode: \"or\"}); let o = []; for g in r { o.push(g.doc_id); } o"}}}'
```

A két lista összevetése mutatja a kiesett dokumentumokat; a `search` eredménye mindig a
másik **sorrendtartó részhalmaza**.

## 9. Mellékhatás a mérésre

A `mcp-server/eval/run_eval.py` és a `measure_lanes.py` a `search` toolt méri, így a
kapott szám a retrieval minőségét **és** a budget vágását együtt tartalmazza. Két
következmény:

- A `--limit 10`-zel hívott `search` átlagosan ~7 dokumentumot ad vissza, tehát a
  jelentett „Recall@10" valójában Recall@~7.
- A `measure_lanes.py` `mirror`-ellenőrzése emiatt **leállítja a futást** (50/62
  lekérdezésen eltérést lát), hiszen a `db_hybrid_search` nem alkalmazza a budgetet.
  A motor-szintű összehasonlításhoz az ellenőrzésnek a sorrendtartó részhalmazt el kell
  fogadnia, és a lane-eket a `mirror`-hoz kell mérni.
