# Modul: `index::manager`

## Cél
Nincs dokumentált információ.

## Fő absztrakciók
- **IndexManager**: Index kezelő komponens
- **Compound index**: Összetett kulcsú indexek
- **B-tree index**: B-fa alapú indexek  
- **Fulltext index**: Teljes szöveges indexek
- **Fuzzy index**: Fuzzy keresési indexek

## Tervezési döntések és invariánsok
- **Unique indexek**: Null kulcsok beszúrásra kerülnek és explicit eltávolítást igényelnek
- **Fuzzy indexek**: Dokumentum ID alapú eltávolítást használnak
- **Index típusok**: Különböző index típusok eltérő eltávolítási stratégiákat alkalmaznak

## Használati minták
Nincs dokumentált információ.

## Korlátok
Nincs dokumentált információ.

---
*Forrás: /home/petitan/MongoLite/ironbase-core/src/index/manager.rs*
