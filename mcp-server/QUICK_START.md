# Quick Start Guide - Claude Desktop Integration

## 🚀 TL;DR - 5 Minute Setup

### Step 1: Start the WSL Server (in WSL terminal)

```bash
cd /home/petitan/MongoLite/mcp-server
DOCJL_CONFIG=config.toml ./target/release/mcp-docjl-server
```

Keep this terminal open! You should see:
```
Starting MCP DOCJL Server v0.1.0
Server listening on 127.0.0.1:8080
```

### Step 2: Copy Bridge to Windows

Open Windows PowerShell and copy the bridge script:

```powershell
# Copy from WSL to Windows Desktop
copy \\wsl$\Ubuntu\home\petitan\MongoLite\mcp-server\mcp_bridge.py $env:USERPROFILE\Desktop\
```

### Step 3: Install Python Requests (Windows)

```powershell
pip install requests
```

### Step 4: Configure Claude Desktop (Windows)

Edit: `%APPDATA%\Claude\claude_desktop_config.json`

Add this configuration:

```json
{
  "mcpServers": {
    "docjl-editor": {
      "command": "python",
      "args": ["C:\\Users\\YourUsername\\Desktop\\mcp_bridge.py"]
    }
  }
}
```

**Replace `YourUsername` with your actual Windows username!**

To find your username:
```powershell
echo $env:USERNAME
```

### Step 5: Restart Claude Desktop

1. Quit Claude Desktop completely (System Tray → Right Click → Quit)
2. Start Claude Desktop again

### Step 6: Test It!

In Claude Desktop, type:

```
List all DOCJL documents
```

You should see Claude respond with available documents!

---

## ✅ Success Checklist

- [ ] WSL server is running (`curl http://localhost:8080/health` returns `{"status":"ok"}`)
- [ ] Bridge script copied to Windows
- [ ] Python `requests` installed on Windows
- [ ] Claude Desktop config file updated with correct path
- [ ] Claude Desktop restarted
- [ ] Claude responds to "List all DOCJL documents"

---

## 🐛 Quick Troubleshooting

### "Cannot connect to WSL server"

Check if server is running:
```bash
# In WSL
curl http://localhost:8080/health
```

Should return: `{"status":"ok","version":"0.1.0"}`

### "Python not found"

Install Python for Windows: https://www.python.org/downloads/

Or use full path in config:
```json
"command": "C:\\Python39\\python.exe"
```

### "Module requests not found"

```powershell
pip install requests
```

### Claude doesn't see the MCP server

1. Check config file syntax (use https://jsonlint.com/)
2. Check file path (use `\\` not `\`)
3. Restart Claude Desktop COMPLETELY
4. Check Claude Desktop logs

---

## 📝 Example Commands to Try

Once connected, try these in Claude Desktop:

**List documents:**
```
List all DOCJL documents
```

**Get outline:**
```
Show me the structure of document 1
```

**Add content:**
```
Add a new paragraph to document 1 with text "Testing from Claude Desktop"
```

**Search:**
```
Search for blocks containing "requirement" in all documents
```

**Validate:**
```
Validate all cross-references in document 2
```

---

## 📚 Full Documentation

For detailed setup, troubleshooting, and advanced features:
- **Windows Setup:** See `WINDOWS_SETUP.md`
- **API Reference:** See `README.md`
- **Testing:** See `demo_real_usage.py`

---

## 🎯 Architecture Overview

```
Windows Claude Desktop
        ↓ JSON-RPC stdin/stdout
    mcp_bridge.py (Python)
        ↓ HTTP POST localhost:8080
    WSL: MCP DOCJL Server (Rust)
        ↓ CRUD operations
    WSL: IronBase Database (.mlite file)
```

---

**Status:** ✅ Ready for use
**Version:** 0.1.0
**Last Updated:** 2025-11-21
