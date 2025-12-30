# Modul: `storage::mod`

## Cél
Nincs dokumentált információ.

## Fő absztrakciók
- **StorageEngine**: Deadlock-free implementáció try_lock mechanizmussal
- **WAL (Write-Ahead Log)**: JSON formátumot használ bincode helyett kompatibilitási okokból
- **Transaction commit**: 9 lépéses atomi művelet
- **Document catalog**: Kollekciókat és tombstone-okat kezel

## Tervezési döntések és invariánsok
- **Atomi commit stratégia**: POSIX rename() garantálja az atomicitást temp → final fájlok átnevezésénél
- **Index atomicitás gyengesége**: Index fájlok NEM kerülnek atomikusan commit-olásra (weak atomicity)
- **WAL és metadata sorrend**: `log_metadata_to_wal()` MINDIG `flush_metadata()` előtt hívandó a helyreállíthatóság érdekében
- **Checkpoint kritikus sorrend**: `flush_metadata()` KÖTELEZŐEN `WAL` törlése előtt hívandó
- **Collection injection centralizáció**: PHASE 5-ben történik a hívók helyett
- **Sealed trait pattern**: RawStorage szándékosan nem publikus

## Használati minták
- **Graceful shutdown**: `mark_clean_shutdown()` KÖTELEZŐEN hívandó a storage eldobása előtt
- **Crash detection**: `was_clean_shutdown()` false értéke esetén indexek újraépítése szükséges dokumentumokból
- **WAL recovery**: DatabaseCore::open() kezeli az index atomicitás miatt
- **Collection létrehozás**: Crash esetén elveszhet, ha nem megfelelően kezelve (2024-12-26-os bug)

## Korlátok
- Document hossz validáció: nem lépheti túl a document region határait
- Tombstone-ok eltávolítása után a számlálók nem csökkennek automatikusan

---
*Forrás: /home/petitan/MongoLite/ironbase-core/src/storage/mod.rs*
