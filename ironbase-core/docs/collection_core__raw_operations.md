# Modul: `collection_core::raw_operations`

## Cél
A modul nyers adatbázis műveleteket implementál atomicitás és memóriahatékonyság biztosításával. A fő cél a versenyhelyzetek (race condition) eliminálása és a batch műveletek optimalizálása.

## Fő absztrakciók
- `UpdateOnePrepared`: Atomi read-modify-write műveletek támogatására
- `DeleteOnePrepared`: Atomi find-and-delete műveletek támogatására
- `try_direct_id_lookup`: O(1) _id alapú keresés optimalizáció
- Streaming document loading: Memóriahatékony dokumentum betöltés

## Tervezési döntések és invariánsok
- **Atomicitás biztosítása**: Write lock tartása a teljes read-modify-write ciklus alatt, kritikus $inc műveleteknél
- **Lock sorrend**: Insert műveletek storage→index sorrendet használnak, drop_index műveletek index→storage sorrendet
- **Batch műveletek**: Index-first megközelítés biztonságosabb storage-first helyett
- **Metadata flush elkerülése**: Insert műveletek nem flush-elik a metadatát teljesítmény okokból
- **Constraint ellenőrzés**: Batch műveleteknél minden constraint ellenőrzés először történik, majd az írások

## Használati minták
- Write lock megszerzése kötelező az I/O műveletek előtt (PHASE 6)
- Batch műveleteknél egyetlen lock megszerzés N dokumentumhoz N lock helyett
- Index műveletek batch-ben történnek (all-or-nothing)
- Streaming document loading használata scan_documents_via_catalog() helyett memóriaproblémák elkerülésére

## Korlátok
Nincs dokumentált információ.

---
*Forrás: /home/petitan/MongoLite/ironbase-core/src/collection_core/raw_operations.rs*
