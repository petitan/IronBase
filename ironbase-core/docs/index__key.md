# Modul: `index::key`

## Cél
Nincs dokumentált információ.

## Fő absztrakciók
- `IndexKey` típus, amely `serde_json::Value` értékekből konvertálható

## Tervezési döntések és invariánsok
- A `serde_json::Value` referenciából való konverzió során a stringeket klónozni kell
- A tulajdonolt `serde_json::Value`-ból való konverzió zero-copy módon történik stringek esetében
- Az `IndexKey` típus összehasonlítható (`PartialOrd` implementáció)

## Használati minták
Nincs dokumentált információ.

## Korlátok
Nincs dokumentált információ.

---
*Forrás: /home/petitan/MongoLite/ironbase-core/src/index/key.rs*
