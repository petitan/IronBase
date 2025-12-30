# Modul: `query_planner`

```markdown
## Cél
Nincs dokumentált információ.

## Fő absztrakciók
- `QueryPlan` enum: Lekérdezési tervek reprezentálására szolgál
- `analyze_query()` függvény: Lekérdezések elemzésére szolgál

## Tervezési döntések és invariánsok
- A `CollectionScan` variant eltávolításra került a `QueryPlan` enum-ból
- Az `analyze_query()` függvény `None` értéket ad vissza teljes táblaszkennelés esetén, a korábbi `CollectionScan` variant helyett

## Használati minták
Nincs dokumentált információ.

## Korlátok
Nincs dokumentált információ.
```

---
*Forrás: /home/petitan/MongoLite/ironbase-core/src/query_planner.rs*
