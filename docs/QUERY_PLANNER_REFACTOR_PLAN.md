# Query Planner Refactor Plan (kicsi, biztonságos lépések)

## Cél
Javítani a planner kódminőségét és döntési stabilitását minimális kockázattal.

## 1) Jelölt indexek gyűjtése
- A korai `return` helyett gyűjtsünk `Vec<CandidatePlan>`‑t.
- `CandidatePlan` mezők:
  - `plan: QueryPlan`
  - `estimated_cost: f64` (kezdetben csak heuristic)
  - `reason: String` (explain/debug)
- Ezután egységes választó függvény dönt.

## 2) Minimális költségbecslés
- Használjuk a `distinct_count` mezőt az `IndexPrefixInfo`‑ban.
- Szabály: kisebb `distinct_count` → jobb index.
- Ha nincs stat (`0`), fallback a jelenlegi prioritás.

## 3) Explain kibővítése
- `explain_query_with_fields` adjon vissza:
  - `chosenPlan`
  - `candidates` (plan + reason + estimated_cost)
- Cél: átlátható, miért lett az adott index kiválasztva.

## 4) Regex helper összevonás
- Hozzunk létre közös helper függvényt:
  - `parse_regex_prefix(pattern, options) -> Result<RegexPrefixInfo>`
- Ezt használja:
  - `analyze_regex_query`
  - `analyze_in_with_regex`
- Kevesebb duplikáció, kevesebb regresszió.

## 5) Compound range támogatás (külön PR)
- Csak akkor aktiváljuk, ha:
  - prefix mezők fixen EQ‑val szűrtek
  - 1 mező range ($gt/$lt)
- Önálló tesztekkel, külön PR‑ban.

## Tesztek
- Jelöltek kiválasztása több indexből (selectivity‑alapú döntés).
- Regex prefix helper: edge case‑ek (escaping, inline flags).
- Explain tartalmazza a rationale‑t.

## Kimenet
- Tiszta, determinisztikus indexválasztás
- Jobb debugolhatóság
- Stabilabb planner API
