# Modul: `fulltext`

## Cél
Nincs dokumentált információ.

## Fő absztrakciók
- **Inverted index**: Token-alapú keresési index, amely TF (term frequency) értékeket tartalmaz
- **Token entries**: Token-hez tartozó bejegyzések, amelyek hiányozhatnak ha a token nem létezik
- **Document removal**: Dokumentumok eltávolítása lazy módon, disk space visszanyerés nélkül

## Tervezési döntések és invariánsok
- A TF értékek az inverted index bejegyzéseiből származnak (korábbi Phase 2 implementáció helyett)
- A dokumentum eltávolítás csak megjelöli a dokumentumot töröltként, nem szabadítja fel a disk területet
- Lazy módban az eltávolított dokumentumok nem törölhetők azonnal a diskről
- Az inverted index frissítése Vec.retain használatával történik HashSet.remove helyett (V3 verzió)

## Használati minták
Nincs dokumentált információ.

## Korlátok
- A `remove` művelet nem nyújt azonnali disk terület visszanyerést
- Lazy módban a dokumentum törlés késleltetett

---
*Forrás: /home/petitan/MongoLite/ironbase-core/src/fulltext.rs*
