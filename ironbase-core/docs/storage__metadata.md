# Modul: `storage::metadata`

```markdown
## Cél
Nincs dokumentált információ.

## Fő absztrakciók
Nincs dokumentált információ.

## Tervezési döntések és invariánsok
- A fájl csonkítás szándékosan nem történik meg az egyidejű olvasásokkal való versenyhelyzetek elkerülése érdekében
- A metaadat és header írása atomikus műveletként történik
- A v3 verzióban kritikus javítás: közvetlenül a `header.data_end_offset` értéket használja
- A header tartalmazza a helyes `data_end_offset` értéket

## Használati minták
Nincs dokumentált információ.

## Korlátok
Nincs dokumentált információ.
```

---
*Forrás: /home/petitan/MongoLite/ironbase-core/src/storage/metadata.rs*
