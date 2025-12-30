# Modul: `document`

## Cél
Nincs dokumentált információ.

## Fő absztrakciók
- `Document`: Dokumentum reprezentáció, amely `Value` típusú adatot kezel
- `DocumentId`: Dokumentum azonosító enum típus, amely `_id` mezőből származtatható

## Tervezési döntések és invariánsok
- A `_id` mező speciális kezelést igényel a lekérdező motorban, mivel nem lehet rá referenciát visszaadni
- A `DocumentId` létrehozása sikertelen, ha az `_id` mező hiányzik vagy érvénytelen típusú
- A serde `#[serde(rename = "_id")]` annotáció az `id` mezőn elfogyasztja azt

## Használati minták
- Tulajdonjog-alapú API használata ajánlott a klónozás elkerülése érdekében, amikor a hívó rendelkezik saját `Value` példánnyal

## Korlátok
Nincs dokumentált információ.

---
*Forrás: /home/petitan/MongoLite/ironbase-core/src/document.rs*
