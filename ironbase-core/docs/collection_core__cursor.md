# Modul: `collection_core::cursor`

## Cél
Nincs dokumentált információ.

## Fő absztrakciók
- `FindCursor<'a, S>` típus
- `next` függvény

## Tervezési döntések és invariánsok
A cursor implementáció iteratív ciklusokat használ rekurzió helyett, hogy elkerülje a stack overflow-t nagy mennyiségű tombstone esetén.

## Használati minták
Nincs dokumentált információ.

## Korlátok
Nincs dokumentált információ.

---
*Forrás: /home/petitan/MongoLite/ironbase-core/src/collection_core/cursor.rs*
