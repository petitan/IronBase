# Modul: `index::btree`

## Cél
B+ fák implementálása, amelyek önkiegyensúlyozó, rendezett keresési struktúrák lemez-alapú tárolásra optimalizálva.

## Fő absztrakciók
- **BPlusTree**: Fő fa struktúra egyedi kulcs támogatással
- **IndexMetadata**: Index metaadatok egyediség kényszerrel
- **Compound index**: Összetett kulcsú indexek egyediség támogatással

## Tervezési döntések és invariánsok
- **Batch műveletek optimalizálása**: O(n * k) helyett O(n log n + k) komplexitás HashMap alapú kinyeréssel és újraépítéssel
- **BTreeMap használata**: IndexKey nem implementálja a Hash-t OrderedFloat miatt
- **Egyediség ellenőrzés**: Rendezett adatoknál O(n) szomszédos duplikátum keresés
- **Kulcs keresés**: binary_search csak egy pozíciót ad vissza, ezért minden azonos kulcsú bejegyzést végig kell szkennelni
- **Újraépítés előtt törlés**: Duplikált bejegyzések megelőzése érdekében
- **Two-Phase Commit**: Atomi fájl átnevezés temp fájlból véglegesbe

## Használati minták
- **Rendezett bemeneti adat**: `build_from_sorted` és `apply_batch_updates` megköveteli kulcs szerinti növekvő sorrendet
- **Egyediség kényszer**: `insert()` `DuplicateKey` hibát ad vissza egyedi indexeknél
- **Optimalizált lekérdezések**: Fordított tartomány szkennelés O(log n) levelek megtalálásával, majd visszafelé iterálással
- **Thread safety**: BPlusTree NEM thread-safe belső implementációban, konkurenciát külső réteg kezeli

## Korlátok
Nincs dokumentált információ.

---
*Forrás: /home/petitan/MongoLite/ironbase-core/src/index/btree.rs*
