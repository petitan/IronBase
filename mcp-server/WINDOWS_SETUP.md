# Windows Claude Desktop Setup Guide

## Overview

This guide shows how to connect Claude Desktop (Windows) to the MCP DOCJL Server running in WSL.

**Architecture:**
```
Windows Claude Desktop
        ↓ (stdin/stdout)
    mcp_bridge.py (Python)
        ↓ (HTTP)
    WSL: MCP DOCJL Server (Rust)
        ↓
    IronBase Database
```

## Prerequisites

1. ✅ WSL2 installed and working
2. ✅ Python 3.x installed on Windows (with `requests` library)
3. ✅ Claude Desktop installed on Windows
4. ✅ MCP DOCJL Server built in WSL

## Step 1: Install Python Dependencies (Windows)

Open PowerShell or Command Prompt:

```powershell
pip install requests
```

## Step 2: Copy Bridge Script to Windows

The bridge script is at: `/home/petitan/MongoLite/mcp-server/mcp_bridge.py`

Copy it to Windows, for example:
```powershell
# From WSL, copy to Windows user directory
cp /home/petitan/MongoLite/mcp-server/mcp_bridge.py /mnt/c/Users/YourUsername/
```

Or access it directly via WSL path:
```
\\wsl$\Ubuntu\home\petitan\MongoLite\mcp-server\mcp_bridge.py
```

## Step 3: Start MCP Server in WSL

In WSL terminal:

```bash
cd /home/petitan/MongoLite/mcp-server

# Start the HTTP server
DOCJL_CONFIG=config.toml ./target/release/mcp-docjl-server
```

You should see:
```
Starting MCP DOCJL Server v0.1.0
Server listening on 127.0.0.1:8080
```

**Keep this terminal open!** The server must be running for Claude Desktop to work.

## Step 4: Test the Bridge (Optional)

Before configuring Claude Desktop, test the bridge manually:

```powershell
# From Windows PowerShell
echo '{"jsonrpc":"2.0","method":"mcp_docjl_list_documents","params":{},"id":1}' | python C:\Users\YourUsername\mcp_bridge.py
```

Expected output:
```json
{"jsonrpc":"2.0","result":{"documents":[...]},"id":1}
```

## Step 5: Configure Claude Desktop

### Find Claude Desktop Config File

The config file is at:
```
%APPDATA%\Claude\claude_desktop_config.json
```

Full path example:
```
C:\Users\YourUsername\AppData\Roaming\Claude\claude_desktop_config.json
```

### Edit Configuration

Open the file in Notepad or your favorite editor and add:

**Option A: Bridge script in Windows user directory**
```json
{
  "mcpServers": {
    "docjl-editor": {
      "command": "python",
      "args": ["C:\\Users\\YourUsername\\mcp_bridge.py"]
    }
  }
}
```

**Option B: Bridge script accessed via WSL path**
```json
{
  "mcpServers": {
    "docjl-editor": {
      "command": "python",
      "args": ["\\\\wsl$\\Ubuntu\\home\\petitan\\MongoLite\\mcp-server\\mcp_bridge.py"]
    }
  }
}
```

**Important Notes:**
- Use double backslashes (`\\`) in JSON paths
- Replace `YourUsername` with your actual Windows username
- Replace `Ubuntu` with your WSL distribution name if different
- If you have other MCP servers, add this to the existing `mcpServers` object

### Full Example Config

If you have other servers:
```json
{
  "mcpServers": {
    "docjl-editor": {
      "command": "python",
      "args": ["C:\\Users\\YourUsername\\mcp_bridge.py"]
    },
    "other-server": {
      "command": "node",
      "args": ["C:\\path\\to\\other-server.js"]
    }
  }
}
```

## Step 6: Restart Claude Desktop

1. **Quit Claude Desktop completely** (right-click tray icon → Quit)
2. **Start Claude Desktop** again
3. The MCP server should connect automatically

## Step 7: Test in Claude Desktop

Try these commands in Claude Desktop:

### List Documents
```
List all DOCJL documents
```

Expected: Claude will show available documents.

### Get Document Outline
```
Show me the outline of test_doc_1
```

Expected: Claude will display the document structure.

### Add Content
```
Add a new paragraph to test_doc_1 with text "Testing from Claude Desktop"
```

Expected: Claude will insert the paragraph and confirm.

## Troubleshooting

### Problem: "Cannot connect to WSL server"

**Solution:**
1. Check if WSL server is running:
   ```bash
   # In WSL
   curl http://localhost:8080/health
   ```
   Should return: `{"status":"ok"}`

2. Start the server if not running:
   ```bash
   cd /home/petitan/MongoLite/mcp-server
   DOCJL_CONFIG=config.toml ./target/release/mcp-docjl-server
   ```

### Problem: "Python not found"

**Solution:**
- Install Python 3 for Windows from python.org
- Or use full path in config:
  ```json
  "command": "C:\\Python39\\python.exe"
  ```

### Problem: "Module 'requests' not found"

**Solution:**
```powershell
pip install requests
```

### Problem: Claude Desktop doesn't see the server

**Solutions:**
1. Check JSON syntax in `claude_desktop_config.json` (use JSONLint)
2. Check file paths (use double backslashes)
3. Restart Claude Desktop completely
4. Check Windows Python can access WSL:
   ```powershell
   python -c "import requests; print(requests.get('http://localhost:8080/health').text)"
   ```

### Problem: Server starts then stops immediately

**Solution:**
- Keep the WSL terminal open
- Don't close the terminal window
- Consider using `tmux` or `screen` in WSL to keep server running

## Starting Server Automatically

### Option A: Windows Startup Script

Create `start_mcp_server.bat`:
```batch
@echo off
wsl -d Ubuntu -e bash -c "cd /home/petitan/MongoLite/mcp-server && DOCJL_CONFIG=config.toml ./target/release/mcp-docjl-server"
```

Add to Windows Startup folder:
```
%APPDATA%\Microsoft\Windows\Start Menu\Programs\Startup\
```

### Option B: WSL systemd Service (if WSL2 with systemd)

Create `/etc/systemd/system/mcp-docjl.service`:
```ini
[Unit]
Description=MCP DOCJL Server
After=network.target

[Service]
Type=simple
User=petitan
WorkingDirectory=/home/petitan/MongoLite/mcp-server
Environment="DOCJL_CONFIG=config.toml"
ExecStart=/home/petitan/MongoLite/mcp-server/target/release/mcp-docjl-server
Restart=on-failure

[Install]
WantedBy=multi-user.target
```

Enable:
```bash
sudo systemctl enable mcp-docjl
sudo systemctl start mcp-docjl
```

## Debugging

### Enable Bridge Debug Mode

Edit `mcp_bridge.py` and change:
```python
DEBUG = True  # Was: DEBUG = False
```

Now check Claude Desktop logs for debug messages.

### Check Server Logs

In WSL:
```bash
tail -f /home/petitan/MongoLite/mcp-server/audit.log
```

### Test Bridge Manually

```powershell
# Test list_documents
echo '{"jsonrpc":"2.0","method":"mcp_docjl_list_documents","params":{},"id":1}' | python mcp_bridge.py

# Test get_document
echo '{"jsonrpc":"2.0","method":"mcp_docjl_get_document","params":{"document_id":"test_doc_1"},"id":2}' | python mcp_bridge.py
```

## Performance Notes

- HTTP overhead is minimal (< 1ms for local WSL connection)
- Bridge script is lightweight (< 1MB memory)
- Server handles multiple concurrent requests
- No data is cached in bridge (stateless)

## Security Notes

- Server listens on `127.0.0.1` only (localhost)
- No external network access required
- Auth can be enabled in `config.toml`
- All operations logged in `audit.log`

## Example Session

**User:** "List all documents"

**Claude (via MCP):**
```
Found 2 documents:
1. test_doc_1 - "Test Document 1" (3 blocks)
2. test_doc_2 - "Requirements Specification" (4 blocks)
```

**User:** "Add a heading 'Performance' to test_doc_2"

**Claude (via MCP):**
```
✅ Added heading "Performance" (label: sec:3)
   Document: test_doc_2
   Position: end
```

## Architecture Diagram

```
┌─────────────────────────────────────┐
│  Windows: Claude Desktop            │
│  (Native Windows App)               │
└───────────────┬─────────────────────┘
                │ JSON-RPC via stdin/stdout
                ↓
┌─────────────────────────────────────┐
│  Windows: mcp_bridge.py             │
│  (Python script)                    │
└───────────────┬─────────────────────┘
                │ HTTP POST to localhost:8080
                ↓
┌─────────────────────────────────────┐
│  WSL2: MCP DOCJL Server             │
│  (Rust HTTP server)                 │
└───────────────┬─────────────────────┘
                │ CRUD operations
                ↓
┌─────────────────────────────────────┐
│  WSL2: IronBase Database            │
│  (docjl_storage.mlite)              │
└─────────────────────────────────────┘
```

## Support

### Check Server Status
```bash
# In WSL
curl http://localhost:8080/health
```

### View Recent Operations
```bash
# In WSL
tail -20 /home/petitan/MongoLite/mcp-server/audit.log
```

### Test Database Connection
```bash
# In WSL
python3 /home/petitan/MongoLite/mcp-server/demo_real_usage.py
```

---

**Status:** ✅ Ready for testing
**Version:** 0.1.0
**Last Updated:** 2025-11-21
