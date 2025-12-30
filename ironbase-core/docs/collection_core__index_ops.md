# Modul: `collection_core::index_ops`

## Cél
Nincs dokumentált információ.

## Fő absztrakciók
- **Index műveletek**: Egyszerű és összetett indexek létrehozása és törlése
- **Compound index**: Többmezős indexek, ahol a mezők sorrendje meghatározó
- **Unique constraint**: Egyediségi megszorítás támogatása indexeken

## Tervezési döntések és invariánsok
- **Mező sorrend jelentősége**: Összetett indexeknél a mezők sorrendje kritikus - a lekérdezések csak akkor tudják használni az indexet, ha a megfelelő mezőkre kérdeznek rá
- **Atomi lock kezelés**: Index törléskor mindkét lock atomikusan kerül felszabadításra
- **Race condition védelem**: Index metaadatok frissítése során index lock tartása szükséges a versenyhelyzetek elkerülésére

## Használati minták
- **Index építés**: Rendezett bejegyzésekből történik O(n) komplexitással
- **Write lock újraszerzés**: Index létrehozáskor szükséges a write lock újbóli megszerzése az építési fázisban

## Korlátok
Nincs dokumentált információ.

---
*Forrás: /home/petitan/MongoLite/ironbase-core/src/collection_core/index_ops.rs*
