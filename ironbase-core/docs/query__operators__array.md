# Modul: `query::operators::array`

## Cél
Nincs dokumentált információ.

## Fő absztrakciók
A modul array műveletek operátorait implementálja:
- `InOperator` - értékek meglétének ellenőrzése tömbben
- `NinOperator` - értékek hiányának ellenőrzése tömbben  
- `AllOperator` - minden szükséges érték meglétének ellenőrzése dokumentum tömbben
- `ElemMatchOperator` - legalább egy tömb elem illeszkedésének ellenőrzése összes feltételre
- `SizeOperator` - tömb méret ellenőrzése

## Tervezési döntések és invariánsok
- `AllOperator`: minden szükséges értéknek jelen kell lennie a dokumentum tömbben
- `ElemMatchOperator`: legalább egy tömb elemnek illeszkednie kell a filter_value összes feltételére
- Ciklomatikus komplexitás értékek: InOperator/NinOperator/SizeOperator (CC=4), AllOperator (CC=6-8), ElemMatchOperator (CC=8)

## Használati minták
Nincs dokumentált információ.

## Korlátok
Nincs dokumentált információ.

---
*Forrás: /home/petitan/MongoLite/ironbase-core/src/query/operators/array.rs*
