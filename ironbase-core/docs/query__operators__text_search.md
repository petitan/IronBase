# Modul: `query::operators::text_search`

```markdown
## Cél
Nincs dokumentált információ.

## Fő absztrakciók
A modul két operátor típust tartalmaz:
- `FuzzyOperator` - fuzzy szöveges keresési műveletek végrehajtására
- `RegexOperator` - reguláris kifejezés alapú szöveges keresési műveletek végrehajtására

## Tervezési döntések és invariánsok
A `FuzzyOperator` implementációja közepes komplexitású (CC = 6), míg a `RegexOperator` valamivel egyszerűbb (CC = 5), ami eltérő algoritmusok használatára utal a két keresési típus között.

## Használati minták
Nincs dokumentált információ.

## Korlátok
Nincs dokumentált információ.
```

---
*Forrás: /home/petitan/MongoLite/ironbase-core/src/query/operators/text_search.rs*
