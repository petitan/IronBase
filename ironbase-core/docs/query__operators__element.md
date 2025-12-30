# Modul: `query::operators::element`

## Cél
Nincs dokumentált információ.

## Fő absztrakciók
- **ExistsOperator**: MongoDB-stílusú létezés ellenőrzés operátor (`{ field: { $exists: true/false } }`)
- **TypeOperator**: Típus ellenőrzés operátor
- **OperatorMatcher**: Trait az operátorok illesztéséhez

## Tervezési döntések és invariánsok
- **Típus validáció**: int32 értékeknek egész számnak kell lenniük ÉS i32 tartományba kell esniük
- **Tárolási formátum**: int64 értékek PosInt/NegInt formában tárolódnak, nem Float-ként
- **Komplexitási korlátok**: ExistsOperator CC=4, TypeOperator CC=10, OperatorMatcher implementáció CC=10

## Használati minták
- ExistsOperator használata: `{ field: { $exists: true } }` mező létezésének ellenőrzésére, `{ field: { $exists: false } }` mező hiányának ellenőrzésére

## Korlátok
Nincs dokumentált információ.

---
*Forrás: /home/petitan/MongoLite/ironbase-core/src/query/operators/element.rs*
