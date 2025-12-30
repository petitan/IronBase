# Modul: `query::operators::comparison`

## Cél
Nincs dokumentált információ.

## Fő absztrakciók
A modul összehasonlító operátorokat implementál az `OperatorMatcher` trait-en keresztül:
- `EqOperator` - egyenlőség operátor (CC = 2)
- `NeOperator` - nem egyenlő operátor (CC = 2-3)
- `GtOperator` - nagyobb mint operátor (CC = 3-4)
- `GteOperator` - nagyobb vagy egyenlő operátor (CC = 4)
- `LtOperator` - kisebb mint operátor (CC = 3)
- `LteOperator` - kisebb vagy egyenlő operátor (CC = 4)

## Tervezési döntések és invariánsok
Az operátorok ciklomatikus komplexitása 2-4 között mozog, ahol az egyenlőség operátorok (EqOperator, NeOperator) a legegyszerűbbek (CC = 2), míg a nagyobb-egyenlő és kisebb-egyenlő operátorok a legkomplexebbek (CC = 4).

## Használati minták
Nincs dokumentált információ.

## Korlátok
Nincs dokumentált információ.

---
*Forrás: /home/petitan/MongoLite/ironbase-core/src/query/operators/comparison.rs*
