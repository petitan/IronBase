# Modul: `aggregation::pipeline`

```markdown
## Cél
Nincs dokumentált információ.

## Fő absztrakciók
- `Pipeline` - aggregációs pipeline implementáció
- `execute_streaming` függvény - streaming végrehajtási mód

## Tervezési döntések és invariánsok
A `$sort` operáció minden dokumentumot meg kell hogy lásson a rendezés végrehajtásához, ami korlátozza a streaming feldolgozás lehetőségeit.

## Használati minták
Nincs dokumentált információ.

## Korlátok
Nincs dokumentált információ.
```

---
*Forrás: /home/petitan/MongoLite/ironbase-core/src/aggregation/pipeline.rs*
