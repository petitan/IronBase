# Modul: `durability`

## Cél
Nincs dokumentált információ.

## Fő absztrakciók
**DurabilityMode enum**: Különböző teljesítményszinteket biztosít az adatbázis műveletek számára:
- Alacsony teljesítmény: ~1,000-5,000 insert/sec
- Közepes teljesítmény: ~20,000-50,000 insert/sec  
- Magas teljesítmény: ~50,000-100,000 insert/sec

## Tervezési döntések és invariánsok
Nincs dokumentált információ.

## Használati minták
Bizonyos konfigurációknál a felhasználónak explicit módon kell meghívnia a `checkpoint()` függvényt. Ha None értékre van állítva, szintén kötelező az explicit checkpoint hívás.

## Korlátok
Nincs dokumentált információ.

---
*Forrás: /home/petitan/MongoLite/ironbase-core/src/durability.rs*
