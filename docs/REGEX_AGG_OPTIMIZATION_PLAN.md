# Regex + Aggregation Performance Plan

## Cél
Csökkenteni a $regex-es és aggregációs lekérdezések idejét nagy adatállományon, úgy hogy a találati halmaz helyes maradjon és ne legyen rejtett regresszió.

## Előfeltételek és jelenlegi korlátok
- Regex prefix optimalizáció csak `^prefix` + üres `$options` esetén aktív.
- Multikey indexnél az index‑szintű `limit` le van tiltva (helyes, de lassú).
- Compound index range scan nincs implementálva (planner tiltja).
- `$limit` → `$match` nem ekvivalens az eredeti query‑vel.

## Fázis 1 — Gyors nyereségek (alacsony kockázat)
1) Regex prefix limit kiterjesztés multikey esetekre
   - Megközelítés: index‑scan oldalon `limit * K` overfetch + doc‑szintű dedup + valós limit.
   - Heurisztika: kezdetben K=5 (konfigurálható), fallback full scan ha alulfedés.
   - Várható eredmény: nagyságrenddel kevesebb doc‑betöltés gyakori `^prefix` regexeknél.

2) `$regex` + `$in` részleges optimalizáció
   - Az `$in` listán belül az `^prefix` jellegű regexeket index prefix scan‑nel kezelni.
   - A nem optimalizálható elemek doc‑szűrésben maradnak.
   - Várható eredmény: jelentősen gyorsabb listás szűrések.

3) Regex prefix felismerés bővítése
   - Engedélyezett minimális kiterjesztés: `(?i)` inline flag **csak** akkor, ha külön case‑insensitive index rendelkezésre áll.
   - Enélkül marad a jelenlegi konzervatív viselkedés.

## Fázis 2 — Közepes kockázatú, nagy hatás
4) Compound index range scan támogatás
   - Range start/end kulcs képzés prefix feltételekkel: pl. (field1=EQ, field2=RANGE).
   - Planner: csak akkor válassza, ha a prefix mezők fixen szűrtek.
   - Várható eredmény: dátum + típus jellegű queryk gyorsulása.

5) Aggregációs gyorsútvonalak bővítése
   - `$match` (indexelt, szelektív) → `$group {_id:null, $sum:1}` → count‑fast‑path.
   - `$group` egyszerű statisztikákra (min/max/first/last) előindexelt mezőn.

## Fázis 3 — Mérnöki finomhangolás
6) Selectivity meta és planner prioritás
   - Index stat gyűjtése (doc count, distinct becslés, multikey arány).
   - Planner választás: a legkisebb becsült eredményhalmaz index preferálása.

7) Explain kiterjesztés
   - Regex prefix: jelölje, ha index‑limit, overfetch, multikey skip aktív.
   - Aggregáció: jelezze az optimizált gyorsútvonalat.

## Tesztelési terv
- Új tesztek:
  - Regex prefix multikey overfetch nem ad alulfedést.
  - `$regex` + `$in` vegyes lista helyes eredmény.
  - Compound range query helyes indexválasztás és eredmény.
  - Explain jelzi az új optimalizációkat.
- Regresszió:
  - `test_regex_prefix_query_analysis`
  - `test_regex_prefix_multikey_limit_returns_full_page`
  - Index persistence és dedup tesztek

## Kockázatok és kontroll
- Overfetch heuristika túl alacsony → alulfedés: fallback full scan logolással.
- Case‑insensitive regex index: csak explicit opt‑in (külön index).
- Compound range scan: óvatos planner feltételek, ha nincs prefix‑fixálás → fallback.

## Mérőszámok
- p95 query idő regex prefix esetekre.
- Dokumentum‑betöltések száma (before/after).
- Index‑scan elem szám (before/after).

## Kimenetek
- Új planner útvonalak és limit‑heurisztika.
- Kibővített explain mezők.
- Új tesztek a regresszió elkerülésére.
