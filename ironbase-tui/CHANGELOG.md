# Changelog

## [0.2.8] - 2024-12-13

### Added
- **Footer refresh shortcut**: `r` gomb megjelenik a footer menüben (Collections és Documents panel)
- **Auto-disappearing status messages**: A status üzenetek (pl. "Collections refreshed!") 2 másodperc után automatikusan eltűnnek

### Changed
- `set_status()` most időbélyeget is tárol az auto-clear funkcióhoz
- Main loop minden iterációban ellenőrzi a lejárt status üzeneteket

## [0.2.7] - 2024-12-13

### Added
- **`--insecure` CLI flag**: Self-signed TLS tanúsítványok elfogadása
- **`mcp_insecure` config option**: Konfig fájlban is beállítható
- **Shift+R shortcut**: Kollekció lista frissítése (help modal-ban dokumentálva)

### Changed
- `HttpTransport::with_options()` támogatja az insecure módot
- `McpClient::connect_http_with_options()` új API

## [0.2.6] - 2024-12-12

### Added
- API Key kezelés modal (Shift+K)
- Új kulcs létrehozása, visszavonása, törlése

## [0.2.5] - 2024-12-11

### Added
- Database váltás modal (Shift+D)
- Több adatbázis támogatása

## [0.2.0] - 2024-12-09

### Changed
- **MCP protokollra átállás**: Közvetlen ironbase-core függőség eltávolítva
- HTTP/HTTPS transport az MCP szerver felé
- Minden művelet MCP tool hívásokon keresztül

## [0.1.0] - 2024-12-01

### Added
- Első verzió
- Kollekció böngészés
- Dokumentum megtekintés, szerkesztés, törlés
- Keresés és szűrés
- Index kezelés
- Export funkció
- Téma váltás
