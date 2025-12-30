# Modul: `wal::recovery`

```markdown
## Cél
Nincs dokumentált információ.

## Fő absztrakciók
- `TransactionGrouper`: Aktív tranzakciók kezelésére szolgáló struktúra
- `CommittedTransaction`: Commitált tranzakciókat reprezentáló típus

## Tervezési döntések és invariánsok
Mindkét fő komponens (`TransactionGrouper` és `CommittedTransaction`) memóriahasználata O(aktív tranzakciók) helyett O(összes bejegyzés) komplexitással lett optimalizálva.

## Használati minták
Nincs dokumentált információ.

## Korlátok
Nincs dokumentált információ.
```

---
*Forrás: /home/petitan/MongoLite/ironbase-core/src/wal/recovery.rs*
