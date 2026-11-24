# IronBase TUI

Terminal User Interface az IronBase NoSQL adatbázishoz.

> [English version](../README.md)

## Telepítés

```bash
# Release build
cargo build --release -p tuti-tui

# A bináris itt lesz:
./target/release/tuti-tui
```

## Használat

```bash
# Alapértelmezett (http://127.0.0.1:8080)
tuti-tui

# Távoli szerver
tuti-tui --url https://192.168.0.136:8080/mcp

# Self-signed tanúsítvány elfogadása
tuti-tui --url https://192.168.0.136:8080/mcp --insecure

# API key-el
tuti-tui --url https://192.168.0.136:8080/mcp -k sk-xxx --insecure

# API key környezeti változóból
IRONBASE_API_KEY=sk-xxx tuti-tui --url https://192.168.0.136:8080/mcp --insecure

# Helyi adatbázis a TUI által indított szerverfolyamaton át (stdio)
tuti-tui --server ./mcp-ironbase-server mydata.mlite
```

## CLI Opciók

```
tuti-tui [OPTIONS] [DATABASE]

Arguments:
  [DATABASE]  Database file path (.mlite) - required for stdio transport

Options:
      --url <URL>          MCP server URL (alias: --http)
  -k, --api-key <API_KEY>  API key [env: IRONBASE_API_KEY]
      --insecure           Accept self-signed TLS certificates
      --server <SERVER>    MCP server executable (stdio transport)
      --log-file <LOG_FILE>  Log file path (lásd: Naplózás)
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
| `Esc` | Vissza / Bezár; oldalbetöltés közben: a betöltés megszakítása |

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
| `a`, majd `x` | Index kezelés (az akciók menüből) |
| `y` | Másolás vágólapra |

### Globális

| Billentyű | Művelet |
|-----------|---------|
| `Shift+I` | Szerver információk |
| `Shift+U` | Frissítés ellenőrzés |
| `Shift+K` | API Key kezelés |
| `Shift+D` | A kiválasztott kollekció törlése (Kollekciók panel, megerősítést kér) |
| `a`, majd `b` | Adatbázis megnyitása / váltása (az akciók menüből) |

## Nyelv

A felület ékezetes magyar vagy angol.

- A `config.toml`-ban `language = "hu"` vagy `"en"` választja ki (kis- és nagybetű mindegy).
- Ha nincs megadva, a rendszer nyelvét követi: az első beállított `LC_ALL`, `LC_MESSAGES`, `LANG` változót. `hu`-val kezdődő értéknél magyar, minden másnál angol (a `C` és a hiányzó locale is, például konténerben vagy Windows-szolgáltatásban).
- Ismeretlen érték esetén a rendszer nyelve marad, és figyelmeztetés kerül a naplóba; a konfiguráció többi része betöltődik.

A fejléc, a parancssáv és a négy panel már le van fordítva. A modalok és az állapot- és hibaüzenetek fordítása a következő lépésekben jön; addig ezek egy része angol felületen is magyar.

## Szövegszerkesztés

Minden szövegmező ugyanazokat a szerkesztőbillentyűket használja.

### Egysoros mezők

Keresés, szűrő, nevek, útvonalak, a lekérdező vektor, az API-kulcs neve, a listener mezői, a RAG mezői.

| Billentyű | Művelet |
|-----------|---------|
| `←` / `→` | Kurzor mozgatása |
| `Home` / `End`, `Ctrl+A` / `Ctrl+E` | A mező eleje / vége |
| `Backspace` / `Delete` | Törlés a kurzor előtt / alatt |
| `Ctrl+U` / `Ctrl+K` | Törlés az elejéig / a végéig |
| `Ctrl+W` | A kurzor előtti szó törlése |

A hosszú érték görgetve fér el a mezőben; a terminál saját kurzora mutatja, hova írsz. Kivételek: a fulltext- és a RAG-keresésben a `Ctrl+J` / `Ctrl+K` a találatok között lép (RAG-ban csak, amíg van találat), a vektorkeresésben a `j` / `k` és a nyilak.

### Többsoros szerkesztők

A dokumentumszerkesztő (beszúrás / szerkesztés), a lekérdezés-szerkesztő, az IronRhai script kódja és a RAG import tartalma. A fenti billentyűk az aktuális sorban működnek, és ezen felül:

| Billentyű | Művelet |
|-----------|---------|
| `Enter` | Új sor (lekérdezésnél, ha van találat: szűrőként alkalmazza; RAG importnál: indítja az importot) |
| `Tab` | Két szóköz (script-szerkesztőben: következő mező) |
| `↑` / `↓`, `PgUp` / `PgDn` | Előző / következő sor, 10 sor |
| `Ctrl+Home` / `Ctrl+End` | A szöveg eleje / vége |
| `Ctrl+S` | Mentés (dokumentum), futtatás (lekérdezés), mentés és futtatás (script) |

Lekérdezés-szerkesztő: `Ctrl+E` explain, `Ctrl+T` sablonok, `Ctrl+J` / `Ctrl+K` lépés a találatok között. A sor elején a Backspace, a sor végén a Delete összevonja a sorokat; a hosszú sor oldalra görget.

### Beillesztés

A terminál bracketed paste módja be van kapcsolva: a beillesztett szöveg egyben érkezik, és csak a fókuszban lévő mezőbe kerül.

- A többsoros szerkesztő megtartja a sorokat.
- Az egysoros mező egy sort kap: a sortörések (`\n`, `\r\n`, `\r`) és a tabulátorok szóközök lesznek.
- A számmező (vektordimenzió, RAG chunk size és overlap) a számjegyeket veszi át.
- Az új kollekció neve csak a névben megengedett karaktereket tartja meg.
- Ha nincs fókuszban szövegmező, vagy nincs nyitott ablak, a beillesztés elvész: a beillesztett karakterek sosem futnak parancsként.

Windows-on a konzol billentyűleütésekként adja át a beillesztést, ezért ott ezek a szabályok nem érvényesek.

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
tuti-tui --url https://localhost:8080/mcp

# Távoli gép esetén (ha a CA telepítve van ott is)
tuti-tui --url https://192.168.0.136:8080/mcp
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
tuti-tui --url https://192.168.0.136:8080/mcp --insecure
```

## Konfiguráció

A konfiguráció helye: `~/.config/ironbase-tui/config.toml` (Linux/macOS) vagy `%APPDATA%\ironbase-tui\config.toml` (Windows)

```toml
# Alapértelmezett MCP szerver URL
mcp_url = "http://127.0.0.1:8080"

# API key (opcionális)
mcp_api_key = "sk-your-key"

# Self-signed tanúsítvány elfogadása
mcp_insecure = false

# Téma: "Claude", "Dark", "Light", "Ocean"
theme = "Claude"

# A felület nyelve: "hu" vagy "en"; ha nincs megadva: a rendszeré (LANG)
language = "hu"
```

Mind a négy téma minden szövegszíne legalább WCAG AA kontrasztú (4,5:1) a háttérhez képest. Ha a `NO_COLOR`
környezeti változó be van állítva (bármilyen nem üres értékre, lásd https://no-color.org), a TUI színek nélkül
rajzol; a kijelölt sor `▶` jelet, a kiválasztott gomb vagy lehetőség `[zárójelet]` kap.

## Naplózás

A TUI csak fájlba naplóz (a terminálra soha): `~/.local/state/ironbase-tui/tuti-tui.log` (Linux), `~/Library/Application Support/ironbase-tui/tuti-tui.log` (macOS) vagy `%LOCALAPPDATA%\ironbase-tui\tuti-tui.log` (Windows). A `--log-file <path>` felülírja a helyet.

| Beállítás | Érték |
|-----------|-------|
| Szint | Alapértelmezés `info`; a `TUTI_TUI_LOG=debug` (off/error/warn/info/debug/trace) felülírja |
| Tartalom | Indulás, kapcsolódás, minden MCP tool hívás (név, időtartam, eredmény), az állapotsorban megjelenő hibák (a modális ablakokban megjelenő hibák még nem kerülnek a naplóba), konfiguráció betöltése/mentése, leállás |
| Titkok | API kulcs, admin kulcs és fejléc soha nem kerül a naplóba; tool argumentumok csak `debug` szinten, a titkos mezők kitakarva (a név tartalmazza: `password`, `secret`, `token`, `authorization`, `apikey`, `adminkey`, `credential`, vagy `key`-re végződik; kis-/nagybetű, `_` és `-` nem számít) |
| Hibaszöveg | Írás előtt megtisztítva: az API kulcs, az admin kulcs és a munkamenetben létrehozott kulcs bárhol kitakarva (a 6 karakternél rövidebb értékeket nem követi); az URL-ekből a séma, a host és az útvonal marad, a felhasználói rész és a query string kitakarva; a vezérlőkarakterek escape-elve (`\n`). Az URL útvonalában lévő titkot nem ismeri fel |
| Méret | 5 MB fölött a fájl `tuti-tui.log.1` névre kerül (egy korábbi generáció marad meg), induláskor és futás közben is. Ha a forgatás nem sikerül, a naplózás a meglévő fájlban folytatódik. Két, ugyanabba a fájlba író TUI folyamat nincs összehangolva: az élő fájlból sor nem vész el, de a korábbi generáció idő előtt felülíródhat |
| Jogosultság | Unixon `0600`, a korábbi generációra is |

## Architektúra

```
ironbase-tui/
├── src/
│   ├── main.rs          # CLI, terminal setup, event loop
│   ├── lib.rs           # Library root (everything but the binary)
│   ├── app.rs           # App state, MCP calls
│   ├── config.rs        # config.toml loading / saving
│   ├── logging.rs       # File logging, secret redaction
│   ├── theme.rs         # Color themes
│   ├── db.rs            # Typed MCP calls (ironbase-mcp-types)
│   ├── i18n/            # Texts: Messages trait, hu.rs, en.rs
│   ├── handlers/        # Key and paste handlers, one per modal
│   ├── state/           # Modal state (filter, query, script, RAG, ...)
│   ├── modals/          # Modal rendering
│   ├── panes/           # Collections, documents, detail panels
│   ├── ui/              # Header, command bar, layout
│   ├── widgets/         # TextInput (one line), TextArea (multi-line)
│   ├── mcp/             # MCP client: JSON-RPC, HTTP / stdio transport
│   └── testing.rs       # Fake MCP server for the tests
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
