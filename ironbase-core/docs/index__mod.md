# Modul: `index::mod`

## Cél
Nincs dokumentált információ.

## Fő absztrakciók
Nincs dokumentált információ.

## Tervezési döntések és invariánsok
- A `build_from_sorted` függvény korábban a meglévő fához adta hozzá az elemeket ahelyett, hogy lecserélte volna azt
- Non-unique indexekben több bejegyzés oszthatja meg ugyanazt a kulcsot, ezért az összes egyező bejegyzést végig kell szkennelni a keresés során
- Eltávolított dokumentumok nem találhatók meg keresés során

## Használati minták
Nincs dokumentált információ.

## Korlátok
Nincs dokumentált információ.

---
*Forrás: /home/petitan/MongoLite/ironbase-core/src/index/mod.rs*
