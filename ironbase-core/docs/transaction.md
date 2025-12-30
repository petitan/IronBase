# Modul: `transaction`

## Cél
A modul ACID (Atomicity, Consistency, Isolation, Durability) tulajdonságokkal rendelkező tranzakció kezelést biztosít.

## Fő absztrakciók
- **Transaction**: Több műveletet csoportosít atomikus végrehajtáshoz
- **IndexChange**: Index változtatások atomikus alkalmazásához
- **OrderedFloat**: Index változtatások atomikus alkalmazásához használt típus

## Tervezési döntések és invariánsok
Az atomicitás központi szerepet játszik - mind a tranzakciók, mind az index változtatások atomikus végrehajtásra vannak tervezve.

## Használati minták
Nincs dokumentált információ.

## Korlátok
Nincs dokumentált információ.

---
*Forrás: /home/petitan/MongoLite/ironbase-core/src/transaction.rs*
