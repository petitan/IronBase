# Modul: `database::collections`

## Cél
Nincs dokumentált információ.

## Fő absztrakciók
- Collection-level write lock mechanizmus Safe mode atomicitáshoz
- Index manager thread-safe létrehozással
- Collection flags perzisztálás

## Tervezési döntések és invariánsok
- Double-checked locking pattern biztosítja a thread-safe létrehozást
- Hybrid locking optimalizáció unique indexek esetén
- READ műveletek optimalizálva - csak READ lockokat használ a hot path-on
- Prepare-WAL-persist szekvencia atomicitása biztosított collection-level write lock-kal
- Race condition-ök megelőzése unique constraint ellenőrzésekben
- Collection flags crash esetén elveszhetnek megfelelő kezelés nélkül
- Védett collection-ök nem törölhetők

## Használati minták
- `get_collection` READ műveletekre optimalizált, implicit létrehozás nélkül
- Collection-level write lock használata Safe mode atomicitáshoz
- Duplicate key hibák logolása silent ignorálás helyett (BUG #4 javítás)

## Korlátok
Nincs dokumentált információ.

---
*Forrás: /home/petitan/MongoLite/ironbase-core/src/database/collections.rs*
