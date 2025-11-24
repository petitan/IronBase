# Claude Code MCP Integration Setup

This guide explains how to integrate the MCP DOCJL Server with Claude Code using the stdio bridge.

## Architecture

```
Claude Code (stdio-based MCP client)
         ↓
mcp_bridge.py (stdio-to-HTTP bridge)
         ↓
mcp-docjl-server (HTTP server on port 8080)
         ↓
IronBase database (docjl_storage.mlite)
```

## Prerequisites

1. **MCP DOCJL Server built and ready**
   ```bash
   cd /home/petitan/MongoLite/mcp-server
   cargo build --release --features real-ironbase
   ```

2. **Python 3 with requests library**
   ```bash
   pip3 install requests
   ```

3. **Database seeded with documents** (optional, for testing)
   ```bash
   python3 seed_real_db.py
   ```

## Configuration

### Option 1: Using mcp_claude_launcher.sh (Recommended)

The launcher script automatically:
- Checks if the HTTP server is running (port 8080)
- Starts it if needed
- Launches the stdio bridge for Claude Code

**Steps:**

1. Make the launcher executable:
   ```bash
   chmod +x mcp_claude_launcher.sh
   ```

2. Update your Claude Code configuration (`~/.config/claude/mcp.json` or Windows equivalent):
   ```json
   {
     "mcpServers": {
       "docjl-editor": {
         "command": "/home/petitan/MongoLite/mcp-server/mcp_claude_launcher.sh",
         "cwd": "/home/petitan/MongoLite/mcp-server"
       }
     }
   }
   ```

3. Restart Claude Code to pick up the new configuration.

### Option 2: Manual Server Management

If you prefer to manage the HTTP server separately:

1. **Start the HTTP server manually:**
   ```bash
   cd /home/petitan/MongoLite/mcp-server
   DOCJL_CONFIG=config.toml ./target/release/mcp-docjl-server &
   ```

2. **Configure Claude Code to use the bridge directly:**
   ```json
   {
     "mcpServers": {
       "docjl-editor": {
         "command": "python3",
         "args": ["/home/petitan/MongoLite/mcp-server/mcp_bridge.py"],
         "cwd": "/home/petitan/MongoLite/mcp-server"
       }
     }
   }
   ```

## Available Tools

Once configured, Claude Code will have access to these DOCJL editing tools:

### Read-Only Tools

1. **mcp_docjl_list_documents** - List all DOCJL documents
2. **mcp_docjl_get_document** - Get full document by ID
3. **mcp_docjl_list_headings** - Get document outline/TOC
4. **mcp_docjl_search_blocks** - Search for blocks matching criteria
5. **mcp_docjl_search_content** - 🆕 Full-text search (solves context window problems!)

### Write Tools

6. **mcp_docjl_insert_block** - Insert new content block
7. **mcp_docjl_update_block** - Update existing block
8. **mcp_docjl_delete_block** - Delete block from document

## Example Usage in Claude Code

Once configured, you can ask Claude Code things like:

```
Find all mentions of "gázelemző" in mk_manual_v1
```

Claude will automatically use `mcp_docjl_search_content` to find matching blocks without downloading the entire 675-block document.

```
Add a new paragraph about calibration procedures to section sec:5
```

Claude will use `mcp_docjl_insert_block` to add the content.

## Testing the Integration

### Test 1: Verify tools are available

In Claude Code, type:
```
What MCP tools do you have available?
```

You should see all 8 DOCJL tools listed.

### Test 2: Search for content

```
Search for "calibration" in document mk_manual_v1
```

Claude should use the `search_content` tool and show you matching blocks.

### Test 3: Get document outline

```
Show me the outline of mk_manual_v1
```

Claude should use `list_headings` to show the table of contents.

## Troubleshooting

### Server not starting

Check if port 8080 is already in use:
```bash
lsof -i :8080
# Kill existing process if needed
pkill -f mcp-docjl-server
```

### Bridge not working

Test the bridge manually:
```bash
cd /home/petitan/MongoLite/mcp-server
echo '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' | python3 mcp_bridge.py
```

You should see a JSON response with all tools listed.

### Authentication errors

Make sure the API key in `mcp_bridge.py` (line 19) matches your `config.toml`:
```python
API_KEY = "dev_key_12345"  # Must match config.toml
```

### Debug logging

Enable debug logging:
```bash
# Start server with debug output
RUST_LOG=debug DOCJL_CONFIG=config.toml ./target/release/mcp-docjl-server 2>&1 | tee /tmp/mcp_server_debug.log
```

Check bridge stderr output in Claude Code logs (location varies by platform).

## Security Notes

1. **API Key**: Change `dev_key_12345` to a secure key in production
2. **Network**: Server binds to 127.0.0.1 (localhost only) by default
3. **Authentication**: Disabled by default (`require_auth = false` in config.toml)
4. **Rate Limiting**: Configured per-key (100 req/min, 10 writes/min for dev key)

## Advanced Configuration

### Custom port

Edit `config.toml`:
```toml
host = "127.0.0.1"
port = 9090  # Change from default 8080
```

And update `mcp_bridge.py`:
```python
SERVER_URL = "http://127.0.0.1:9090/mcp"
```

### Multiple API keys

Add additional keys in `config.toml`:
```toml
[[api_keys]]
key = "claude_code_key"
name = "Claude Code Key"
allowed_commands = ["mcp_docjl_*"]  # Wildcard for all commands
allowed_documents = ["*"]

[api_keys.rate_limit]
requests_per_minute = 200
writes_per_minute = 50
```

Then update `mcp_bridge.py` to use the new key.

## Files Reference

- **mcp_bridge.py** - stdio-to-HTTP bridge (387 lines)
- **mcp_claude_launcher.sh** - Launcher script that manages server lifecycle
- **config.toml** - Server configuration (port, API keys, rate limits)
- **mcp-docjl-server** - HTTP server binary (target/release/)
- **docjl_storage.mlite** - IronBase database file

## What's New: search_content Tool

The newly added `mcp_docjl_search_content` tool solves the context window problem:

**Before:** To find "gázelemző", Claude would have to:
1. Download entire document (675 blocks, ~500KB+)
2. Search client-side
3. Use massive context window

**After:** With search_content:
1. Send query to server
2. Server searches and returns only matching blocks
3. Minimal context usage (1 block instead of 675)

**Parameters:**
- `document_id` (required): Document to search
- `query` (required): Text to search for
- `case_sensitive` (optional): Default false
- `max_results` (optional): Default 100

**Returns:**
```json
{
  "document_id": "mk_manual_v1",
  "query": "gázelemző",
  "total_matches": 1,
  "matches": [
    {
      "block_index": 21,
      "block_type": "Heading",
      "label": "para:17",
      "text": "...",
      "block": {...}
    }
  ]
}
```

This is particularly useful for:
- Large technical manuals (100+ pages)
- Finding specific terms/requirements
- Compliance audits
- Cross-reference checking
