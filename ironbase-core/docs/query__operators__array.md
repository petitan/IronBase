# Modul: `query::operators::array`

## Cél
Nincs dokumentált információ.

## Fő absztrakciók
A modul array-specifikus query operátorokat implementál:
- `AllOperator` - minden megadott érték jelenlétét ellenőrzi a dokumentum tömbben
- `ElemMatchOperator` - legalább egy tömb elem illeszkedését vizsgálja az összes filter feltételre
- `InOperator` - érték jelenlétét ellenőrzi megadott értékek között
- `NinOperator` - érték hiányát ellenőrzi megadott értékek között  
- `SizeOperator` - tömb méret ellenőrzésére szolgál

Mindegyik implementálja az `OperatorMatcher` trait-et.

## Tervezési döntések és invariánsok
Az operátorok ciklomatikus komplexitása 4-8 között mozog, ami közepes összetettségű logikára utal. Az `AllOperator` és `ElemMatchOperator` rendelkeznek a legmagasabb komplexitással (8 és 6-8), míg a többi operátor egyszerűbb (4-6).

## Használati minták
Nincs dokumentált információ.

## Korlátok
Nincs dokumentált információ.

---
*Forrás: /home/petitan/MongoLite/ironbase-core/src/query/operators/array.rs*
