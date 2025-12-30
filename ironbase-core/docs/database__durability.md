# Modul: `database::durability`

## Cél
Nincs dokumentált információ.

## Fő absztrakciók
- **WAL (Write-Ahead Log)**: Központi komponens a tartósság biztosításához
- **Atomic Point**: WAL fsync művelet, amely után az adatok crash-safe állapotba kerülnek
- **Batch műveletek**: Több művelet atomikus végrehajtása

## Tervezési döntések és invariánsok
- **Atomic Point**: WAL fsync után az adat crash-safe, ez a rendszer atomicitási pontja
- **Trade-off**: Késleltetett láthatóság a garantált tartósság érdekében
- **Invariáns**: `doc_id` mindig `Some` értékű, amikor `modified > 0` vagy `deleted > 0`
- **Háromfázisú commit protokoll**: PREPARE → WAL COMMIT → véglegesítés
- **Lock-alapú atomicitás**: Dokumentum keresés és módosítás lock alatt történik

## Használati minták
- **WAL commit elsőbbsége**: WAL commit mindig megelőzi az egyéb műveleteket
- **Atomikus validáció**: Minden validáció és constraint ellenőrzés atomikusan történik
- **Tombstone pattern**: Törlés esetén tombstone írása a tényleges törlés helyett

## Korlátok
Nincs dokumentált információ.

---
*Forrás: /home/petitan/MongoLite/ironbase-core/src/database/durability.rs*
