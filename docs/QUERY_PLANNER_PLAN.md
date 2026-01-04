# Query Planner Fejlesztési Terv

## Cél
- Gyorsabb query-k index jobb kihasználásával
- Kiszámítható explain kimenet
- Bővíthető architektúra (új operátorok, indexek)

## Fázis 1 (gyors nyereség, alacsony kockázat)
1) Többmezős $match támogatás (szelektív mező választás)
   - Elemzés: minden top-level mező értékelése, indexes mezőlista alapján
   - Heurisztika: preferált index = egyenlőség + konkrét érték, majd range
   - Kimenet: QueryPlan kiválasztása legjobb indexre

2) $and és $or minimális támogatás
   - $and: válassz legjobb index-tervet a tagok közül
   - $or: index-tervek union doc_id szinten (később merge-elt)
   - Explain: mutasson "CompositePlan" jellegű leírást

3) Regex prefix optimalizáció bővítése
   - ^prefix + $options: i támogatása normalizált index esetén
   - Feltétel: mezőn kisbetűsített index (pl. külön field: email_lc)

4) Explain bővítés
   - Több jelölt terv felsorolása és indoklás
   - Becsült k (match count) heurisztikával

## Fázis 2 (közepes kockázat, nagyobb haszon)
1) Compound index teljes kihasználás
   - Equality prefix + range a következő mezőn
   - Pl. {a:1, b:{$gte:5}} használja [a,b] indexet

2) Sort + limit integráció
   - Index-scan + early termination, ha sort field indexelt
   - Top-K tervezés filterrel kombinálva

3) Index intersection ($and)
   - Két index doc_id listájának metszete
   - Threshold: csak ha mindkettő szelektív

4) Covering index alapú projektálás
   - Ha csak indexelt mezők kellenek, doc betöltés nélkül
   - Explain jelzi: "covered"

## Fázis 3 (magasabb komplexitás)
1) Statisztika-alapú cost model
   - Doc count, distinct count, index fanout, histograms
   - Plan választás költség alapján

2) Partial / sparse index preferencia
   - $exists / null gyakoriság figyelembevétele
   - Egyértelmű jelzés explain-ben

3) Multi-key szelektivitás kezelése
   - Array mezők indexében becsült duplikációs faktor
   - Cost modellbe beépítés

## Technikai lépések (keresztszekció)
- QueryPlan bővítése (kompozit tervek: Union/Intersection)
- Explain struktúra formális JSON sémával
- Tesztek:
  - planner: regex prefix, $and/$or, compound equality+range
  - explain: expected plan type + index name

## Ajánlott sorrend (1–2 sprint)
- Sprint 1: többmezős $match + $and/$or minimal, explain bővítés
- Sprint 2: compound equality+range, sort+limit index, index intersection
