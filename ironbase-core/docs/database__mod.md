# Modul: `database::mod`

## Cél
Nincs dokumentált információ.

## Fő absztrakciók
- `DatabaseCore<S>`: A fő adatbázis struktúra
- `check_not_closed` függvény: Adatbázis állapot ellenőrzése

## Tervezési döntések és invariánsok
- **Thread Safety Model**: Safe mode prepare-WAL-persist atomicitás biztosítása
- **Collection-szintű írás zárolás**: Fine-grained `Mutex` per collection Safe mode atomicitáshoz
- **Prepare-WAL-persist szekvencia**: Atomi végrehajtás collection-enként
- **Unique constraint ellenőrzések**: Race condition-ök megelőzése
- **Tranzakció életciklus**: Tranzakciók eltávolítása az aktív listából commit után

## Használati minták
- Adatbázis bezárt állapotának ellenőrzése kötelező a műveletek előtt
- Tranzakciók automatikus eltávolítása az aktív listából sikeres commit után

## Korlátok
Nincs dokumentált információ.

---
*Forrás: /home/petitan/MongoLite/ironbase-core/src/database/mod.rs*
