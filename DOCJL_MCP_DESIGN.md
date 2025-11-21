# DOCJL + IronBase + MCP Design

## Cél
AI-asszisztált szerkesztés DOCJL dokumentumokra. Korlát:
- Struktúra betartása (`docjll` blokkok)
- Sémakontroll (DOCJL JSON schema)
- Audit, visszakövethetőség

## Architektúra
```
Claude (MCP client)
   ↓ JSON-RPC
MCP Host (Rust modul)
   ↓ domain API
IronBase adapter
   ↓
IronBase core (Rust)
```

### MCP Host
- MCP JSON-RPC implementáció (parancs-regiszter, authentikáció, audit log).
- Security: whitelisted parancsok (AI csak ezeket hívhatja).

### Domain API
- Fejlécek, bekezdések, listák reprezentációja.
- Parancsok: `insert_heading`, `move_block`, `update_paragraph`, `delete_block`, `list_headings`.
- Label és cross-reference manager:
  - új blokknál label generálás
  - move update -> label order fix
  - cross-ref ellenőrzés (hibás hivatkozás = warning).
- Schema helper:
  - validate parancs bejövő diffen (mielőtt CRUD fut)
  - fallback: IronBase core schema check

### IronBase Adapter
- `set_collection_schema` (startkor).
- CRUD (find/insert/update/delete).
- Transaction wrapper (opcionális rollback).

### Rust Core
- Sémát enforce-olja (`docjl-schema.json`).
- `cargo run --example strip_level` jellegű utilityk.
- Storage/WAL/Stability.

## Folyamat
1. MCP parancs érkezik (pl. `insert_heading`).
2. Host -> domain API:
   - doc ID / blokkszülő lekérése (IronBase `find_one`).
   - Domain logika módosítja az AST-et.
3. CRUD:
   - `update_one` / `insert_one`.
   - Automatikus schema validáció.
4. Visszaválasz: módosított rész, audit entry.

## Mappa struktúra (javaslat)
```
/mcp-server
  /src
    /host
    /domain
    /adapters
  /tests
  Cargo.toml
docs/MCP_DOCJL.md (parancs-spec)
```

## További ACP
- Stress: docjl nagy dokumentum (80k blok), move tests.
- Logging: minden MCP call -> log entry.
- Tools: Python binding marad utility-nek, de core MCP Rust.
