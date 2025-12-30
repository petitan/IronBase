# Modul: `database::collections`

## Cél
Nincs dokumentált információ.

## Fő absztrakciók
- **Collection Write Lock**: `Arc<Mutex<()>>` Safe mode atomicitáshoz
- Index manager thread-safe létrehozáshoz és kezeléshez
- Collection flags perzisztens tároláshoz

## Tervezési döntések és invariánsok
- Double-checked locking pattern biztosítja a thread-safe létrehozást
- Safe mode-ban a prepare-WAL-persist szekvencia atomikus, megelőzve race condition-öket unique constraint ellenőrzésekben
- Collection flags crash esetén elveszhetnek, ha nem megfelelően kezelve (2024-12-26-án felfedezett bug)
- B+ tree indexek optimalizálása: átugorja azokat, amelyek már rendelkeznek adatokkal (.idx fájlokból betöltve)
- Duplicate key hibák naplózása a csendes figyelmen kívül hagyás helyett (BUG #4 javítás)

## Használati minták
- READ műveletek optimalizálva: csak READ lock-okat használ a hot path-on
- Hybrid locking optimalizáció unique index-szel rendelkező collection-ök esetén
- Lock contention minimalizálása thread-safe létrehozás biztosítása mellett
- Collection lekérése implicit létrehozás nélkül - hibát ad vissza, ha nem létezik

## Korlátok
- Protected collection-ök nem törölhetők

---
*Forrás: /home/petitan/MongoLite/ironbase-core/src/database/collections.rs*
