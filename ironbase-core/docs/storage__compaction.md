# Modul: `storage::compaction`

## Cél
A storage compaction célja a tombstone-ok és régi dokumentum verziók eltávolítása az adatbázisból.

## Fő absztrakciók
- `CompactionConfig`: Konfigurációs típus a compaction folyamat beállításaihoz
- `CompactionStats`: Statisztikák gyűjtése a compaction művelet eredményeiről
- Chunked processing: A compaction darabolva dolgozza fel az adatokat

## Tervezési döntések és invariánsok
- Katalógus-alapú iteráció használata szekvenciális fájl szkennelés helyett
- Atomi fájlcsere biztosítása (a legtöbb fájlrendszeren atomi művelet)
- A compaction végén az adatbázis állapot újratöltése szükséges

## Használati minták
Nincs dokumentált információ.

## Korlátok
Nincs dokumentált információ.

---
*Forrás: /home/petitan/MongoLite/ironbase-core/src/storage/compaction.rs*
