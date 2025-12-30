# Modul: `wal::recovery`

## Cél
Nincs dokumentált információ.

## Fő absztrakciók
- `CommittedTransaction`: Tranzakció reprezentáció memóriaoptimalizált tárolással
- `TransactionGrouper`: Aktív tranzakciók csoportosítására szolgáló struktúra

## Tervezési döntések és invariánsok
A modul memóriahasználat-optimalizált architektúrát követ: mind a `CommittedTransaction`, mind a `TransactionGrouper` O(aktív tranzakciók) memóriahasználatot biztosít az O(összes bejegyzés) helyett.

## Használati minták
Nincs dokumentált információ.

## Korlátok
Nincs dokumentált információ.

---
*Forrás: /home/petitan/MongoLite/ironbase-core/src/wal/recovery.rs*
