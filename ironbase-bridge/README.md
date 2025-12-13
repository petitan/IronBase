# IronBase Bridge

STDIO to HTTP/HTTPS bridge for MCP IronBase Server.

Compatible with:
- Claude Desktop (Anthropic)
- ChatGPT Desktop (OpenAI)
- VS Code Copilot
- JetBrains AI Assistant
- Cursor
- Any MCP-compatible client

## Features

- **Connection Pooling** - HTTP keep-alive for better performance
- **Self-Signed Certificates** - `--insecure` flag for dev environments
- **Graceful Shutdown** - Handles SIGINT/SIGTERM properly
- **Health Check** - Automatic server availability check with retry
- **Batch Requests** - Full JSON-RPC 2.0 batch support
- **Cross-Platform** - Windows, Linux, macOS

## Installation

Download from releases or build from source:

```bash
cd ironbase-bridge
cargo build --release
```

Binary will be at `target/release/ironbase-bridge` (or `.exe` on Windows).

## Usage

```bash
# Default (localhost:8080)
ironbase-bridge

# Remote server with API key
ironbase-bridge --server https://192.168.0.136:8080/mcp --api-key sk-xxx

# Self-signed certificate (dev/WSL)
ironbase-bridge --server https://localhost:8080/mcp --insecure

# Debug mode
ironbase-bridge --debug
```

## CLI Options

| Option | Environment Variable | Default | Description |
|--------|---------------------|---------|-------------|
| `-s, --server` | `MCP_SERVER_URL` | `http://localhost:8080/mcp` | Server URL |
| `-k, --api-key` | `IRONBASE_API_KEY` | - | API key for authentication |
| `--insecure` | `MCP_INSECURE` | false | Accept self-signed certs |
| `-d, --debug` | `MCP_DEBUG` | false | Enable debug logging |
| `--health-retries` | - | 3 | Health check retry count (0=skip) |

## Client Configuration

### Claude Desktop

Edit `claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "ironbase": {
      "command": "C:\\Program Files\\IronBase\\ironbase-bridge.exe",
      "env": {
        "MCP_SERVER_URL": "http://localhost:8080/mcp",
        "IRONBASE_API_KEY": "sk-your-key"
      }
    }
  }
}
```

### ChatGPT Desktop

Same configuration format as Claude Desktop (MCP standard).

## Cross-Compilation

```bash
# Windows
cargo build --release --target x86_64-pc-windows-msvc

# Linux
cargo build --release --target x86_64-unknown-linux-gnu

# macOS Intel
cargo build --release --target x86_64-apple-darwin

# macOS Apple Silicon
cargo build --release --target aarch64-apple-darwin
```

## License

MIT
