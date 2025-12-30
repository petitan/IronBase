# Modul: `document`

## Cél
Nincs dokumentált információ.

## Fő absztrakciók
- `Document`: Dokumentum reprezentáció, amely `_id` mezővel rendelkezik
- `DocumentId`: Dokumentum azonosító típus, amely hiányzó vagy érvénytelen típusú `_id` esetén None értéket ad vissza

## Tervezési döntések és invariánsok
- A `_id` mező speciális kezelést igényel a query engine-ben, mivel nem lehet rá referenciát visszaadni
- A `#[serde(rename = "_id")]` annotáció az id mezőt "elfogyasztja" a szerializáció során
- Tulajdonjog-alapú API tervezés: a `from_value_owned` függvény átveszi a `Value` tulajdonjogát, elkerülve a klónozást amikor a hívó már rendelkezik tulajdonjoggal

## Használati minták
Nincs dokumentált információ.

## Korlátok
- A `_id` mező hiánya vagy érvénytelen típusa esetén a műveletek None értéket adnak vissza
- HashMap használat miatt a nevek sorrendje változhat wildcard keresések során

---
*Forrás: /home/petitan/MongoLite/ironbase-core/src/document.rs*
