# IronBase TUI

Terminal User Interface for IronBase NoSQL database.

> [Magyar verzió / Hungarian version](docs/README_HU.md)

## Installation

```bash
# Release build
cargo build --release -p tuti-tui

# Binary location:
./target/release/tuti-tui
```

## Usage

```bash
# Default (http://127.0.0.1:8080)
tuti-tui

# Remote server
tuti-tui --url https://192.168.0.136:8080/mcp

# Accept self-signed certificate
tuti-tui --url https://192.168.0.136:8080/mcp --insecure

# With API key
tuti-tui --url https://192.168.0.136:8080/mcp -k sk-xxx --insecure

# API key from environment variable
IRONBASE_API_KEY=sk-xxx tuti-tui --url https://192.168.0.136:8080/mcp --insecure
```

## CLI Options

```
tuti-tui [OPTIONS] [DATABASE]

Arguments:
  [DATABASE]  Database file path (.mlite) - required for stdio transport

Options:
      --url <URL>          MCP server URL (alias: --http)
  -k, --api-key <API_KEY>  API key [env: IRONBASE_API_KEY]
      --insecure           Accept self-signed TLS certificates
      --server <SERVER>    MCP server executable (stdio transport)
      --log-file <LOG_FILE>  Log file path (see Logging)
  -h, --help               Print help
  -V, --version            Print version
```

## Keyboard Shortcuts

### Navigation

| Key | Action |
|-----|--------|
| `Tab` | Next panel |
| `Shift+Tab` | Previous panel |
| `j` / `↓` | Down |
| `k` / `↑` | Up |
| `PgUp` / `PgDn` | Page up/down |
| `g` / `Home` | Go to top |
| `G` / `End` | Go to bottom |
| `Enter` | Select |
| `Esc` | Back / Close |

### Functions

| Key | Action |
|-----|--------|
| `/` | Search |
| `a` | Actions menu |
| `?` | Help |
| `r` | Refresh data |
| `R` (Shift+R) | Refresh collection list |
| `t` | Toggle theme |
| `q` | Quit |

### Detail Panel

| Key | Action |
|-----|--------|
| `e` | Edit document |
| `d` | Delete document |
| `f` | Visual filter |
| `i` | Insert document |
| `a` then `x` | Index management (via the actions menu) |
| `y` | Copy to clipboard |

### Global

| Key | Action |
|-----|--------|
| `Shift+I` | Server info |
| `Shift+U` | Check for updates |
| `Shift+K` | API Key management |
| `Shift+D` | Drop the selected collection (Collections panel, asks for confirmation) |
| `a` then `b` | Open / switch database (via the actions menu) |

## HTTPS Setup with mkcert

Certificates created with `mkcert` are trusted on the local machine, no `--insecure` flag needed.

### Install mkcert

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

### Install CA and Generate Certificate

```bash
# Install root CA to system trust store (once)
mkcert -install

# Generate certificate for MCP server
cd /path/to/mcp-server
mkcert -key-file key.pem -cert-file cert.pem localhost 127.0.0.1 ::1 192.168.0.136

# Multiple hostnames/IPs:
mkcert -key-file key.pem -cert-file cert.pem \
    localhost 127.0.0.1 ::1 \
    192.168.0.136 myserver.local
```

### MCP Server config.toml

```toml
[tls]
enabled = true
cert_file = "./cert.pem"
key_file = "./key.pem"
```

### Usage with mkcert Certificate

```bash
# No --insecure needed when CA is trusted
tuti-tui --url https://localhost:8080/mcp

# Remote machine (if CA is installed there too)
tuti-tui --url https://192.168.0.136:8080/mcp
```

### WSL2 / Cross-machine Usage

If the server runs in WSL2 and you connect from Windows:

```bash
# In WSL2, generate cert with Windows IP too
mkcert -key-file key.pem -cert-file cert.pem localhost 127.0.0.1 172.19.152.126

# On Windows, install the mkcert CA
# (copy ~/.local/share/mkcert/rootCA.pem and import it)
```

If you don't want to install the CA, use the `--insecure` flag:

```bash
tuti-tui --url https://192.168.0.136:8080/mcp --insecure
```

## Configuration

Config location: `~/.config/ironbase-tui/config.toml` (Linux/macOS) or `%APPDATA%\ironbase-tui\config.toml` (Windows)

```toml
# Default MCP server URL
mcp_url = "http://127.0.0.1:8080"

# API key (optional)
mcp_api_key = "sk-your-key"

# Accept self-signed certificates
mcp_insecure = false

# Theme: "Claude", "Dark", "Light", "Ocean"
theme = "Claude"
```

## Logging

The TUI logs to a file only (never to the terminal): `~/.local/state/ironbase-tui/tuti-tui.log` (Linux), `~/Library/Application Support/ironbase-tui/tuti-tui.log` (macOS) or `%LOCALAPPDATA%\ironbase-tui\tuti-tui.log` (Windows). `--log-file <path>` overrides the location.

| Setting | Value |
|---------|-------|
| Level | `info` by default; `TUTI_TUI_LOG=debug` (off/error/warn/info/debug/trace) overrides it |
| Content | Startup, connection, every MCP tool call (name, duration, outcome), errors shown in the status bar (errors shown inside a modal window are not logged yet), config load/save, shutdown |
| Secrets | API key, admin key and headers are never logged; tool arguments only at `debug`, with secret fields redacted (name contains `password`, `secret`, `token`, `authorization`, `apikey`, `adminkey`, `credential`, or ends in `key`; case, `_` and `-` ignored) |
| Error text | Sanitised before it is written: the API key, the admin key and a key created in the session are masked wherever they occur (values shorter than 6 characters are not tracked); URLs keep scheme, host and path, the userinfo and the query string are masked; control characters are escaped (`\n`). A secret in a URL path is not recognised |
| Size | Above 5 MB the file is rotated to `tuti-tui.log.1` (one previous generation is kept), at startup and while running. If the rotation fails, logging continues in the current file. Two TUI processes sharing one file are not coordinated: no line is lost from the live file, but the previous generation can be replaced early |
| Permissions | `0600` on Unix, for the previous generation too |

## Architecture

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

## Requirements

- Running MCP server (`mcp-ironbase-server`)
- Terminal with UTF-8 support
- Minimum 80x24 character terminal

## Related Projects

- [ironbase-core](../ironbase-core/) - Rust core library
- [mcp-server](../mcp-server/) - MCP protocol server
- [ironbase-bridge](../ironbase-bridge/) - STDIO-HTTP bridge for Claude Desktop
