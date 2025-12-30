# Modul: `query::operators::logical`

## Cél
Nincs dokumentált információ.

## Fő absztrakciók
A modul logikai operátorokat implementál: `AndOperator`, `OrOperator`, `NorOperator` és `NotOperator` struktúrákat, amelyek mind implementálják az `OperatorMatcher` trait-et.

## Tervezési döntések és invariánsok
Az `AndOperator` implementációja tömb validációt és iterációt használ. A ciklomatikus komplexitás értékek:
- `AndOperator`: CC = 5 (tömb validáció és iteráció miatt)
- `OrOperator`: CC = 5 
- `NorOperator`: CC = 5 (struktúra), CC = 3 (implementáció)
- `NotOperator`: CC = 3

## Használati minták
Nincs dokumentált információ.

## Korlátok
Nincs dokumentált információ.

---
*Forrás: /home/petitan/MongoLite/ironbase-core/src/query/operators/logical.rs*
