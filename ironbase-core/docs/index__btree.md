# Modul: `index::btree`

## Cél
Nincs dokumentált információ.

## Fő absztrakciók
- **IndexMetadata**: Metaadat típus unique constraint támogatással
- **Compound key indexek**: Összetett kulcsok kezelése unique constraint opcionális támogatással
- **Two-Phase Commit**: Kétfázisú commit protokoll atomi változtatásokhoz

## Tervezési döntések és invariánsok
- **BTreeMap használata HashMap helyett**: IndexKey nem implementálja a Hash trait-et OrderedFloat miatt
- **Sorted entries követelmény**: `build_from_sorted` és kapcsolódó műveletek megkövetelik, hogy a bemeneti adatok kulcs szerint növekvő sorrendben legyenek
- **Atomi fájlműveletek**: Temp fájlból végső fájlba történő rename művelettel biztosított atomicitás
- **Unique constraint validáció**: O(n) időben szomszédos duplikátumok ellenőrzése rendezett adatokon
- **Clear before rebuild**: Duplikált bejegyzések megelőzése érdekében a fa törlése rebuild előtt kötelező

## Használati minták
- **Batch update optimalizáció**: O(n*k) helyett O(n log n + k) komplexitás HashMap-es kinyeréssel, rendezéssel és újraépítéssel
- **Reverse traversal optimalizáció**: Nagy adathalmazok (49,200+ ID) esetén jobbról-balra bejárás a teljes gyűjtés és megfordítás helyett
- **O(n) extraction**: `get_all_entries` batch rebuild műveletekhez optimalizált kinyerést biztosít
- **Two-phase commit pattern**: Előkészített változtatások atomi commitja temp→final fájl rename-mel

## Korlátok
Nincs dokumentált információ.

---
*Forrás: /home/petitan/MongoLite/ironbase-core/src/index/btree.rs*
