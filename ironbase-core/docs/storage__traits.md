# Modul: `storage::traits`

## Cél
Egységes interfész biztosítása minden storage backend számára.

## Fő absztrakciók
- `Storage` trait: az egységes interfész, amelyet minden storage backend implementálni köteles
- `sealed_raw` modul: olyan implementáció, amely nem módosít megosztott állapotot (file position)

## Tervezési döntések és invariánsok
- File-alapú storage `&mut self` referenciát igényel az I/O műveletek (seek/read) miatt
- A `sealed_raw` implementáció `&self` referenciával működik, mivel nem módosít megosztott állapotot
- In-memory storage esetén az indexek újraépítése mindig hamis értéket ad vissza

## Használati minták
- Konkurens olvasás támogatott read lock-kal write lock helyett a `sealed_raw` implementációban
- File-alapú storage mutable hozzáférést igényel az I/O műveletekhez

## Korlátok
Nincs dokumentált információ.

---
*Forrás: /home/petitan/MongoLite/ironbase-core/src/storage/traits.rs*
