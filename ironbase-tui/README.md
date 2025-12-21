# IronBase TUI

Terminal User Interface for IronBase NoSQL database.

> [Magyar verzió / Hungarian version](docs/README_HU.md)

## Installation

```bash
# Release build
cargo build --release -p ironbase-tui

# Binary location:
./target/release/ironbase-tui
```

## Usage

```bash
# Default (localhost:8080)
ironbase-tui

# Remote server
ironbase-tui --url https://192.168.0.136:8080/mcp

# Accept self-signed certificate
ironbase-tui --url https://192.168.0.136:8080/mcp --insecure

# With API key
ironbase-tui --url https://192.168.0.136:8080/mcp -k sk-xxx --insecure

# API key from environment variable
IRONBASE_API_KEY=sk-xxx ironbase-tui --url https://192.168.0.136:8080/mcp --insecure
```

## CLI Options

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
| `x` | Index management |
| `y` | Copy to clipboard |

### Global

| Key | Action |
|-----|--------|
| `Shift+I` | Server info |
| `Shift+U` | Check for updates |
| `Shift+K` | API Key management |
| `Shift+D` | Switch database |

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
ironbase-tui --url https://localhost:8080/mcp

# Remote machine (if CA is installed there too)
ironbase-tui --url https://192.168.0.136:8080/mcp
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
ironbase-tui --url https://192.168.0.136:8080/mcp --insecure
```

## Configuration

Config location: `~/.config/ironbase-tui/config.toml` (Linux/macOS) or `%APPDATA%\ironbase-tui\config.toml` (Windows)

```toml
# Default MCP server URL
mcp_url = "http://localhost:8080/mcp"

# API key (optional)
mcp_api_key = "sk-your-key"

# Accept self-signed certificates
mcp_insecure = false

# Theme: "dark", "light", "nord", "dracula"
theme = "dark"
```

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
