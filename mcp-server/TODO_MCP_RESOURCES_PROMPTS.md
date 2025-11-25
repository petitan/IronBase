# TODO: MCP Resources és Prompts Implementáció

## Cél

Teljes MCP protokoll implementáció **Rust szerverben** (nem Python-ban!), hogy a szerver HTTP-n keresztül is teljes funkcionalitású legyen.

## Architektúra Döntés

### ❌ ROSSZ (Jelenlegi)
```
Claude Desktop → Python Bridge (MCP logika) → Rust Server (csak tools)
```
**Probléma**: HTTP kliensek nem kapják meg az MCP funkciókat!

### ✅ HELYES (Cél)
```
Claude Desktop → Python Bridge (STDIO↔HTTP) → Rust Server (TELJES MCP)
```
**Előnyök**:
- Rust szerver önállóan is MCP kompatibilis
- Python bridge csak "dumb translator"
- HTTP kliensek teljes MCP funkcionalitás

---

## Fázis 1: Rust Szerver MCP Metódusok (src/main.rs)

### 1.1 `initialize` Handler
- [ ] `InitializeParams` struct létrehozása
- [ ] `InitializeResult` struct (capabilities + serverInfo)
- [ ] Capabilities visszaadása:
  - `tools: {}`
  - `resources: {}`
  - `prompts: {}`
- [ ] ServerInfo: name="docjl-editor", version="0.1.0"
- [ ] protocolVersion: "2025-06-18"

### 1.2 `tools/list` Handler
- [ ] Tool lista generálás (jelenleg Python bridge-ben van)
- [ ] 9 tool definíció áthelyezése Rust-ba:
  - mcp_docjl_create_document
  - mcp_docjl_list_documents
  - mcp_docjl_get_document
  - mcp_docjl_list_headings
  - mcp_docjl_search_blocks
  - mcp_docjl_search_content
  - mcp_docjl_insert_block (javított label guidance)
  - mcp_docjl_update_block
  - mcp_docjl_delete_block
- [ ] InputSchema JSON definiálás minden tool-hoz

### 1.3 `tools/call` Handler
- [x] ✅ Már implementálva (tools/call unwrapping)
- [x] ✅ Content wrapping válaszban

### 1.4 `resources/list` Handler
- [ ] Backend hívás: `mcp_docjl_list_documents`
- [ ] Resource lista generálás:
  - URI: `docjl://document/{doc_id}`
  - name: metadata.title vagy doc_id
  - description: Automatikus generálás metadata-ból
  - mimeType: "application/json"
- [ ] Hibakezelés (üres lista, ha nincs dokumentum)

### 1.5 `resources/read` Handler
- [ ] URI parsing: `docjl://document/{doc_id}`
- [ ] Backend hívás: `mcp_docjl_get_document`
- [ ] Resource válasz formázás:
  - uri: az eredeti URI
  - mimeType: "application/json"
  - text: JSON.stringify(document)
- [ ] Hibakezelés (404, ha nem létezik)

### 1.6 `prompts/list` Handler

#### Általános Prompts (20 db)
- [ ] **Dokumentum Szerkezet** (4 db):
  1. analyze-structure - Szerkezet elemzése
  2. suggest-subsections - Alszekciók javaslat
  3. create-toc - Tartalomjegyzék generálás
  4. validate-hierarchy - Hierarchia validáció

- [ ] **Szöveg Feldolgozás** (4 db):
  5. summarize-section - Szekció összefoglalás
  6. expand-section - Szekció kibővítés
  7. simplify-text - Egyszerűsítés
  8. extract-key-points - Kulcspontok kiemelés

- [ ] **Tartalom Keresés** (4 db):
  9. find-content - Tartalom keresés
  10. explain-concept - Koncepció magyarázat
  11. compare-sections - Szekciók összehasonlítás
  12. find-related - Kapcsolódó szekciók

- [ ] **Minőség & Javítás** (4 db):
  13. suggest-improvements - Javaslatok
  14. check-consistency - Konzisztencia ellenőrzés
  15. validate-references - Hivatkozások ellenőrzés
  16. add-examples - Példák hozzáadás

- [ ] **Generálás** (4 db):
  17. generate-outline - Vázlat generálás
  18. continue-writing - Folytatás
  19. create-introduction - Bevezető
  20. create-conclusion - Összefoglalás

#### ISO 17025 Audit Specifikus Prompts (10 db)
- [ ] **Compliance Check** (3 db):
  21. audit-requirements - Követelmények ellenőrzés
  22. gap-analysis - Hiányosságok elemzés
  23. validate-procedures - Eljárások validálás

- [ ] **Documentation** (3 db):
  24. generate-procedure - Eljárás generálás
  25. update-quality-manual - Minőségirányítási kézikönyv frissítés
  26. create-work-instruction - Munkautasítás létrehozás

- [ ] **Audit Preparation** (4 db):
  27. prepare-evidence - Bizonyítékok előkészítés
  28. checklist-generator - Ellenőrző lista generálás
  29. nc-response - Eltérés válasz generálás
  30. improvement-plan - Fejlesztési terv

---

## Fázis 2: Python Bridge Egyszerűsítés (mcp_bridge.py)

### 2.1 Összes MCP Logika Eltávolítása
- [ ] `handle_mcp_protocol()` függvény **TÖRLÉSE**
- [ ] `initialize` handler törlése
- [ ] `tools/list` handler törlése
- [ ] `tools/call` special handling törlése

### 2.2 "Dumb Proxy" Implementáció
- [ ] Új egyszerű architektúra:
  ```python
  def process_request(request_line: str) -> Dict[str, Any]:
      # 1. Parse JSON
      # 2. Forward to backend (HTTP POST)
      # 3. Return response unchanged
  ```
- [ ] Nincs több MCP-specifikus logika
- [ ] Csak STDIO ↔ HTTP konverzió

### 2.3 Hibakezelés Egyszerűsítése
- [ ] Connection error → forward unchanged
- [ ] Timeout → forward unchanged
- [ ] Parse error → standard JSON-RPC error

---

## Fázis 3: Tesztelés

### 3.1 Rust Szerver HTTP API Tesztek
- [ ] `test_mcp_initialize.sh`
  - initialize hívás
  - capabilities ellenőrzés

- [ ] `test_mcp_tools_list.sh`
  - tools/list hívás
  - 9 tool jelenlét ellenőrzés

- [ ] `test_mcp_resources.sh`
  - resources/list (üres és teli DB-vel)
  - resources/read (létező és nem létező URI)

- [ ] `test_mcp_prompts.sh`
  - prompts/list hívás
  - 30 prompt jelenlét ellenőrzés
  - Minden prompt kategória tesztelés

### 3.2 Python Bridge Tesztek
- [ ] `test_bridge_passthrough.py`
  - Minden MCP metódus unchanged továbbítás
  - STDIO → HTTP → STDIO round-trip

### 3.3 Integrációs Tesztek
- [ ] `test_full_mcp_flow.sh`
  - initialize → tools/list → tools/call → resources/list → prompts/list
  - Teljes MCP protokoll flow

### 3.4 Claude Desktop Teszt
- [ ] Windows Claude Desktop konfiguráció
- [ ] STDIO kommunikáció tesztelés
- [ ] Prompt use case-ek próbálása

---

## Fázis 4: Dokumentáció

### 4.1 Implementációs Dokumentáció
- [ ] `MCP_COMPLETE_IMPLEMENTATION.md`
  - Teljes architektúra leírás
  - Minden metódus specifikáció
  - Request/Response példák

### 4.2 Prompts Dokumentáció
- [ ] `MCP_PROMPTS_GUIDE.md`
  - Összes prompt részletes leírás
  - Használati példák minden prompt-hoz
  - ISO 17025 specifikus use case-ek

### 4.3 API Dokumentáció Frissítés
- [ ] README.md update
- [ ] HTTP API dokumentáció (OpenAPI?)

---

## Időbecslés

- **Fázis 1**: ~4-6 óra (Rust MCP implementáció)
- **Fázis 2**: ~1 óra (Python egyszerűsítés)
- **Fázis 3**: ~2-3 óra (Tesztelés)
- **Fázis 4**: ~1-2 óra (Dokumentáció)

**Összesen**: ~8-12 óra munka

---

## Függőségek

- Rust: serde_json már elérhető
- Python: requests már telepítve
- Nincs új dependency

---

## Prioritás

1. **P0 (Kritikus)**: Fázis 1.1-1.5 (alapvető MCP metódusok)
2. **P1 (Fontos)**: Fázis 1.6 (prompts - 20 általános)
3. **P2 (Hasznos)**: Fázis 1.6 (ISO 17025 prompts)
4. **P3 (Cleanup)**: Fázis 2 (Python egyszerűsítés)

---

## Megkezdés

**Először**: Fázis 1.1 (`initialize` handler Rust-ban)

**Parancs**:
```bash
# Tesztelés közben folyamatosan build
cargo build --release && ./test_mcp_initialize.sh
```
