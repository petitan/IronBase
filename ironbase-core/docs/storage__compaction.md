# Modul: `storage::compaction`

```markdown
## Cél
A modul célja a storage compaction végrehajtása, amely eltávolítja a tombstone-okat és a dokumentumok régi verzióit.

## Fő absztrakciók
- `CompactionConfig`: Konfiguráció a compaction folyamathoz
- `CompactionStats`: Statisztikák a compaction műveletről
- Chunked processing: A compaction darabolva történő feldolgozást használ

## Tervezési döntések és invariánsok
- Catalog-alapú iteráció használata szekvenciális fájl scan helyett
- Atomi fájlcsere és adatbázis állapot újratöltés a finalizálás során
- Az atomi fájlcsere a legtöbb fájlrendszeren atomi művelet

## Használati minták
Nincs dokumentált információ.

## Korlátok
Nincs dokumentált információ.
```

---
*Forrás: /home/petitan/MongoLite/ironbase-core/src/storage/compaction.rs*
