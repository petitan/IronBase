# Modul: `collection_core::index_ops`

## Cél
Nincs dokumentált információ.

## Fő absztrakciók
- Index műveletek egyszerű és összetett indexekhez
- Egyediség (uniqueness) támogatás mind egyszerű, mind összetett kulcsokhoz
- `CollectionCore<S>` implementáció index műveletekhez

## Tervezési döntések és invariánsok
- Mező sorrend kritikus az összetett indexeknél - lekérdezések csak akkor használhatják az indexet, ha a mező sorrendet követik
- Index építés rendezett bejegyzésekből történik O(n) komplexitással
- Race condition megelőzése: index lock tartása kötelező a metaadat frissítés során
- Atomikus lock feloldás biztosított az index törlés során

## Használati minták
- Index létrehozásnál write lock újra-megszerzése szükséges az építés előtt
- Mindkét lock (index és metaadat) atomikusan kerül feloldásra

## Korlátok
Nincs dokumentált információ.

---
*Forrás: /home/petitan/MongoLite/ironbase-core/src/collection_core/index_ops.rs*
