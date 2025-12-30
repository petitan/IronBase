# Modul: `aggregation::types`

## Cél
Nincs dokumentált információ.

## Fő absztrakciók
- **Pipeline**: Aggregációs pipeline reprezentáció, amely támogatja a vezető `$match` szakasz optimalizált kezelését
- **extract_leading_match**: Függvény a pipeline első `$match` szakaszának kinyerésére és eltávolítására

## Tervezési döntések és invariánsok
- A vezető `$match` szakasz külön kezelése lehetővé teszi indexelt `find()` használatát teljes kollekció scan helyett
- Push állapotban minden értéket tárolni kell, optimalizáció nem lehetséges

## Használati minták
- A hívó fél használhatja a kinyert `$match` query-t JSON formátumban indexelt kereséshez
- Ha az első szakasz nem `$match`, a függvény `None`-t ad vissza

## Korlátok
Nincs dokumentált információ.

---
*Forrás: /home/petitan/MongoLite/ironbase-core/src/aggregation/types.rs*
