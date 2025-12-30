# Modul: `transaction`

## Cél
A modul ACID (Atomicity, Consistency, Isolation, Durability) tulajdonságokkal rendelkező tranzakció kezelést biztosít.

## Fő absztrakciók
- **Transaction**: Több műveletet csoportosít atomikus végrehajtáshoz
- **IndexChange**: Index változtatások atomikus alkalmazásához
- **OrderedFloat**: Index változtatások atomikus alkalmazásához használt típus

## Tervezési döntések és invariánsok
Az index változtatások atomikus alkalmazása központi követelmény - mind az `IndexChange` struktúra, mind az `OrderedFloat` típus ezt a célt szolgálja.

## Használati minták
Nincs dokumentált információ.

## Korlátok
Nincs dokumentált információ.

---
*Forrás: /home/petitan/MongoLite/ironbase-core/src/transaction.rs*
