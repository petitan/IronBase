# Modul: `query::operators::comparison`

## Cél
Nincs dokumentált információ.

## Fő absztrakciók
A modul összehasonlító operátorokat implementál az `OperatorMatcher` trait-en keresztül:
- `EqOperator` - egyenlőség operátor (CC = 2)
- `NeOperator` - nem egyenlő operátor (CC = 3)
- `LtOperator` - kisebb mint operátor (CC = 3)
- `LteOperator` - kisebb vagy egyenlő operátor (CC = 4)
- `GtOperator` - nagyobb mint operátor (CC = 3-4)
- `GteOperator` - nagyobb vagy egyenlő operátor (CC = 4)

## Tervezési döntések és invariánsok
A komplexebb összehasonlító operátorok (LTE, GTE, GT) magasabb ciklomatikus komplexitással (CC = 4) rendelkeznek, míg az egyszerűbb operátorok (EQ, NE, LT) alacsonyabb komplexitással (CC = 2-3).

## Használati minták
Nincs dokumentált információ.

## Korlátok
Nincs dokumentált információ.

---
*Forrás: /home/petitan/MongoLite/ironbase-core/src/query/operators/comparison.rs*
