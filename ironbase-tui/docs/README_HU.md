# IronBase TUI

Terminal User Interface az IronBase NoSQL adatbázishoz.

> [English version](../README.md)

## Telepítés

```bash
# Release build
cargo build --release -p ironbase-tui

# A bináris itt lesz:
./target/release/ironbase-tui
```

## Használat

```bash
# Alapértelmezett (localhost:8080)
ironbase-tui

# Távoli szerver
ironbase-tui --url https://192.168.0.136:8080/mcp

# Self-signed tanúsítvány elfogadása
ironbase-tui --url https://192.168.0.136:8080/mcp --insecure

# API key-el
ironbase-tui --url https://192.168.0.136:8080/mcp -k sk-xxx --insecure

# API key környezeti változóból
IRONBASE_API_KEY=sk-xxx ironbase-tui --url https://192.168.0.136:8080/mcp --insecure
```

## CLI Opciók

```
ironbase-tui [OPTIONS] [DATABASE]

Arguments:
  [DATABASE]  Database file path (.mlite) - required for stdio transport

Options:
      --url <URL>          MCP server URL (alias: --http)
  -k, --api-key <API_KEY>  API key [env: IRONBASE_API_KEY]
      --insecure           Accept self-signed TLS certificates
      --server <SERVER>    MCP server executable (stdio transport)
  -h, --help               Print help
  -V, --version            Print version
```

## Billentyűparancsok

### Navigáció

| Billentyű | Művelet |
|-----------|---------|
| `Tab` | Következő panel |
| `Shift+Tab` | Előző panel |
| `j` / `↓` | Le |
| `k` / `↑` | Fel |
| `PgUp` / `PgDn` | Lapozás |
| `g` / `Home` | Lista elejére |
| `G` / `End` | Lista végére |
| `Enter` | Kiválaszt |
| `Esc` | Vissza / Bezár |

### Funkciók

| Billentyű | Művelet |
|-----------|---------|
| `/` | Keresés |
| `a` | Akciók menü |
| `?` | Súgó |
| `r` | Adatok frissítése |
| `R` (Shift+R) | Kollekció lista frissítés |
| `t` | Téma váltás |
| `q` | Kilépés |

### Detail panel

| Billentyű | Művelet |
|-----------|---------|
| `e` | Dokumentum szerkesztése |
| `d` | Dokumentum törlése |
| `f` | Vizuális szűrő |
| `i` | Dokumentum beszúrása |
| `x` | Index kezelés |
| `y` | Másolás vágólapra |

### Globális

| Billentyű | Művelet |
|-----------|---------|
| `Shift+I` | Szerver információk |
| `Shift+U` | Frissítés ellenőrzés |
| `Shift+K` | API Key kezelés |
| `Shift+D` | Adatbázis váltás |

## HTTPS beállítás mkcert-tel

A `mkcert` eszközzel létrehozott tanúsítványok megbízhatóak lesznek a helyi gépen, nem kell `--insecure` flag.

### mkcert telepítés

```bash
# Linux (Debian/Ubuntu)
sudo apt install mkcert

# Linux (Arch)
sudo pacman -S mkcert

# macOS
brew install mkcert

# Windows (Chocolatey)
choco install mkcert

# Windows (Scoop)
scoop install mkcert
```

### CA telepítés és tanúsítvány generálás

```bash
# Root CA telepítése a rendszer trust store-ba (egyszer kell)
mkcert -install

# Tanúsítvány generálás az MCP szerverhez
cd /path/to/mcp-server
mkcert -key-file key.pem -cert-file cert.pem localhost 127.0.0.1 ::1 192.168.0.136

# Több hostname/IP is megadható:
mkcert -key-file key.pem -cert-file cert.pem \
    localhost 127.0.0.1 ::1 \
    192.168.0.136 myserver.local
```

### MCP szerver config.toml

```toml
[tls]
enabled = true
cert_file = "./cert.pem"
key_file = "./key.pem"
```

### Használat mkcert tanúsítvánnyal

```bash
# Nem kell --insecure, mert a CA megbízható
ironbase-tui --url https://localhost:8080/mcp

# Távoli gép esetén (ha a CA telepítve van ott is)
ironbase-tui --url https://192.168.0.136:8080/mcp
```

### WSL2 / Cross-machine használat

Ha a szerver WSL2-ben fut és Windows-ról csatlakozol:

```bash
# WSL2-ben generáld a cert-et a Windows IP-vel is
mkcert -key-file key.pem -cert-file cert.pem localhost 127.0.0.1 172.19.152.126

# Windows-on telepítsd a mkcert CA-t
# (másold át a ~/.local/share/mkcert/rootCA.pem fájlt és importáld)
```

Ha nem akarod a CA-t telepíteni, használd az `--insecure` flaget:

```bash
ironbase-tui --url https://192.168.0.136:8080/mcp --insecure
```

## Konfiguráció

A konfiguráció helye: `~/.config/ironbase-tui/config.toml` (Linux/macOS) vagy `%APPDATA%\ironbase-tui\config.toml` (Windows)

```toml
# Alapértelmezett MCP szerver URL
mcp_url = "http://localhost:8080/mcp"

# API key (opcionális)
mcp_api_key = "sk-your-key"

# Self-signed tanúsítvány elfogadása
mcp_insecure = false

# Téma: "dark", "light", "nord", "dracula"
theme = "dark"
```

## Architektúra

```
ironbase-tui/
├── src/
│   ├── main.rs          # Entry point, event loop, key handlers
│   ├── app.rs           # App state, business logic
│   ├── config.rs        # Configuration loading
│   ├── theme.rs         # Color themes
│   ├── db.rs            # Database wrapper (MCP client)
│   ├── mcp/             # MCP protocol client
│   │   ├── client.rs    # High-level MCP client
│   │   ├── transport.rs # HTTP/STDIO transport
│   │   ├── protocol.rs  # JSON-RPC types
│   │   └── error.rs     # Error types
│   ├── modals/          # Modal dialogs
│   │   ├── help.rs      # Help modal
│   │   ├── search.rs    # Search modal
│   │   └── ...
│   ├── panes/           # Main UI panels
│   └── widgets/         # Reusable UI components
└── Cargo.toml
```

## Követelmények

- Futó MCP szerver (`mcp-ironbase-server`)
- Terminal UTF-8 támogatással
- Minimum 80x24 karakteres terminál

## Kapcsolódó projektek

- [ironbase-core](../../ironbase-core/) - Rust core library
- [mcp-server](../../mcp-server/) - MCP protocol server
- [ironbase-bridge](../../ironbase-bridge/) - STDIO-HTTP bridge Claude Desktop-hoz
