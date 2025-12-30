# Modul: `storage::mod`

## Cél
Nincs dokumentált információ.

## Fő absztrakciók
- **StorageEngine**: A fő tárolási absztrakció
- **WAL (Write-Ahead Log)**: Helyreállíthatóságot biztosító napló
- **RawStorage**: Belső implementációs réteg (sealed trait pattern)
- **document_catalog**: Kollekciónkénti dokumentum katalógus

## Tervezési döntések és invariánsok
- **Atomicitási garancia**: A tranzakció commit 9-lépéses atomi művelet
- **WAL-metadata sorrend**: `log_metadata_to_wal()` hívása kötelező a `flush_metadata()` előtt a helyreállíthatóság érdekében
- **Checkpoint kritikus szabály**: `flush_metadata()` hívása kötelező a WAL törlése előtt
- **Index atomicitás gyengesége**: Az index fájlok nem atomikusan kerülnek commitálásra (weak atomicity)
- **POSIX rename garancia**: A temp → final átnevezés atomicitását a POSIX rename() biztosítja
- **WAL kompatibilitás**: JSON formátum használata bincode helyett a kompatibilitás érdekében
- **Deadlock-mentes design**: `try_lock` azonnal hibával tér vissza, ha zárolva van

## Használati minták
- **Graceful shutdown**: `mark_clean_shutdown()` kötelező hívása a storage eldobása előtt
- **Crash recovery**: `was_clean_shutdown()` false értéke esetén az indexeket újra kell építeni a dokumentumokból
- **WAL recovery**: A `DatabaseCore::open()` kezeli az index atomicitás érdekében
- **Thread safety**: A StorageEngine belső szinten nem thread-safe, a thread safety külső rétegben biztosított
- **Kollekció létrehozás**: Kötelező a crash-biztos tárolás érdekében

## Korlátok
- **Gyenge index atomicitás**: Az index fájlok nem rendelkeznek teljes atomi commit garanciával
- **Belső API korlátozás**: A RawStorage szándékosan nem publikus (sealed trait pattern)

---
*Forrás: /home/petitan/MongoLite/ironbase-core/src/storage/mod.rs*
