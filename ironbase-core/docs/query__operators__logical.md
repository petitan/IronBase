# Modul: `query::operators::logical`

## Cél
Nincs dokumentált információ.

## Fő absztrakciók
A modul logikai operátorokat implementál:
- `OrOperator` - logikai VAGY művelet
- `AndOperator` - logikai ÉS művelet  
- `NotOperator` - logikai NEM művelet
- `NorOperator` - logikai NOR művelet

Mindegyik operátor implementálja az `OperatorMatcher` trait-et.

## Tervezési döntések és invariánsok
Az `AndOperator` tömb validációt és iterációt végez a működése során. A ciklomatikus komplexitás értékek alapján a logikai operátorok különböző összetettségűek:
- Az `OrOperator`, `AndOperator` és `NorOperator` magasabb komplexitással (CC=5) rendelkeznek
- A `NotOperator` egyszerűbb implementációval (CC=3) bír

## Használati minták
Nincs dokumentált információ.

## Korlátok
Nincs dokumentált információ.

---
*Forrás: /home/petitan/MongoLite/ironbase-core/src/query/operators/logical.rs*
