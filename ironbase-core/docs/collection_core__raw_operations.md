# Modul: `collection_core::raw_operations`

## Cél
Nincs dokumentált információ.

## Fő absztrakciók
- `UpdateOnePrepared` - Atomi read-modify-write műveletek előkészített állapota
- `DeleteOnePrepared` - Atomi find-and-delete műveletek előkészített állapota
- Kétfázisú műveletek: prepare fázis (I/O write lock alatt) és persist fázis

## Tervezési döntések és invariánsok
- **Atomi műveletek**: Write lock tartása a teljes read-modify-write ciklus alatt a lost update-ek elkerülésére, kritikus $inc műveleteknél
- **Lock sorrend**: Insert műveletek storage→index sorrendet használnak, drop_index műveletek index→storage sorrendet
- **Index-first stratégia**: Batch műveleteknél az indexek frissítése történik először (atomi szinten), majd a storage írás
- **Race condition elkerülés**: Write lock megszerzése a dokumentumok olvasása előtt megelőzi a konkurens hozzáférési problémákat
- **Streaming document loading**: Memória optimalizáció a teljes dokumentum betöltés helyett

## Használati minták
- **O(1) _id lookup optimalizáció**: Közvetlen _id alapú keresés teljes scan helyett
- **Batch optimalizáció**: Egyetlen lock megszerzéssel több dokumentum olvasása N darab lock helyett
- **HashMap pre-allokáció**: Ismert kapacitással történő memória optimalizáció
- **Metadata flush elkerülés**: Teljesítmény optimalizáció érdekében nincs metadata flush minden insert után

## Korlátok
- Zárt kollekciókban az insert műveletek sikertelenek
- Konkurens műveletek esetén az eredmények változhatnak, de a teljes konzisztencia megmarad

---
*Forrás: /home/petitan/MongoLite/ironbase-core/src/collection_core/raw_operations.rs*
