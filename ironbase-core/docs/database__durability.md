# Modul: `database::durability`

## Cél
Nincs dokumentált információ.

## Fő absztrakciók
- **WAL (Write-Ahead Log)**: Tranzakciós naplózási mechanizmus fsync támogatással
- **Atomic Point**: WAL fsync művelet, amely az atomicitás garantálásának központi eleme
- **Batch műveletek**: Csoportos adatbázis operációk támogatása

## Tervezési döntések és invariánsok
- **WAL-first commit stratégia**: A WAL fsync művelet szolgál atomic pointként minden tranzakcióban
- **Atomi read-modify-write**: Az `update_one` művelet lock alatt atomi módon olvassa, módosítja és írja az adatokat
- **Késleltetett láthatóság**: Az `insert_one` művelet tudatos kompromisszumot köt a késleltetett láthatóság és a garantált tartósság között
- **Validáció és constraint ellenőrzés**: Az `insert_many` művelet minden validációt és megszorítás-ellenőrzést atomikusan végez

## Használati minták
- A batch műveletek (`update_many`, `delete_many`) háromfázisú protokollt követnek, ahol a 3. fázis a WAL commit
- Az egyedi műveletek (`update_one`, `delete_one`) PREPARE fázisban végzik az adatmódosítást lock alatt
- Minden kritikus művelet a WAL fsync atomic pointra támaszkodik a konzisztencia biztosításához

## Korlátok
Az `update_one` implementáció korlátai: a jelenlegi `update_one_prepare` már közvetlenül a storage-ba ír, ami megnehezíti a batch feldolgozást. Batch támogatáshoz `update_one_prepare_batch()` implementáció szükséges, amely puffereli a műveleteket azonnali perzisztálás helyett.

---
*Forrás: /home/petitan/MongoLite/ironbase-core/src/database/durability.rs*
