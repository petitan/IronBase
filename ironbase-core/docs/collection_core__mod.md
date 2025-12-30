# Modul: `collection_core::mod`

## Cél
A modul dokumentumok hatékony lekérdezését, módosítását és indexelését biztosítja nagy gyűjtemények esetén, különös tekintettel a teljesítmény-optimalizációra és memóriahasználatra.

## Fő absztrakciók
- **QueryExecutionContext**: Lekérdezés-végrehajtási konfiguráció (Clean Architecture megközelítés)
- **DocumentId**: Speciális típus az `_id` index kezelésére
- **B+ tree indexek**: Rendezett iterációhoz és gyors kereséshez
- **Document catalog**: O(1) dokumentum lookup-hoz offset alapon

## Tervezési döntések és invariánsok
- **Atomi tranzakciók**: Index változások nyomon követése, de még nem atomi alkalmazás
- **Batch műveletek**: Egyetlen lock megszerzés teljesítmény céljából (pl. insert_many)
- **Szekvenciális disk olvasás**: Offsetek rendezése az optimális I/O teljesítményért
- **Early termination**: Nagy gyűjteményeknél pagination esetén kritikus teljesítmény-optimalizáció
- **O(1) _id lookup**: Közvetlen document_catalog használat szerializáció nélkül
- **Index-alapú optimalizáció**: B+ tree használata memóriában történő rendezés helyett

## Használati minták
- **Olvasási műveletek**: `with_shared_indexes_readonly` használata már létező gyűjteményekhez
- **Batch validáció**: Minden ellenőrzés az írások előtt az atomi hiba biztosításához
- **Streaming feldolgozás**: Dokumentumok egyenkénti betöltése O(1) memóriahasználatért
- **Index nélküli szűrés**: Teljes dokumentum scan szükséges, ha nincs megfelelő index

## Korlátok
- **HashMap iteráció**: Nem-determinisztikus sorrend ASLR hash seed-ek miatt
- **Vec-alapú indexek**: O(n) költség insert/delete műveleteknél (20K frissítés 100K indexen ~8 milliárd elem mozgatás)
- **Tombstone dokumentumok**: Nem érhetők el a lookup műveletek során

---
*Forrás: /home/petitan/MongoLite/ironbase-core/src/collection_core/mod.rs*
