# Modul: `index::manager`

## Cél
Nincs dokumentált információ.

## Fő absztrakciók
- `IndexManager` - központi típus az indexek kezelésére
- B-tree indexek egyedi értékek támogatásával
- Compound (összetett) indexek egyedi kulcs támogatással
- Fulltext indexek
- Fuzzy indexek dokumentum ID alapú kezeléssel

## Tervezési döntések és invariánsok
- Egyedi indexeknél null kulcsok beszúrásra kerülnek és eltávolításkor külön kezelendők
- Fuzzy indexeknél a dokumentumok ID alapján kerülnek eltávolításra
- Query optimalizáció a `select_best_index()` metódussal implementált

## Használati minták
- `IndexManager` maga NEM thread-safe - a konkurencia kezelés külső mechanizmusokkal történik
- Fulltext index törlése Ok(()) értéket ad vissza sikeres eltávolítás esetén, Err-t ha nem található

## Korlátok
Nincs dokumentált információ.

---
*Forrás: /home/petitan/MongoLite/ironbase-core/src/index/manager.rs*
