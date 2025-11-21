# Claude Desktop Integration Guide

## Overview

This guide shows how to integrate the MCP DOCJL Server with Claude Desktop, enabling Claude to directly edit and manage DOCJL documents.

## Prerequisites

1. Claude Desktop installed
2. MCP DOCJL Server built (release mode)
3. Configuration file set up

## Step 1: Build Release Binary

```bash
cd /home/petitan/MongoLite/mcp-server
cargo build --release --features real-ironbase
```

This creates: `target/release/mcp-docjl-server`

## Step 2: Configure Claude Desktop

### Location of Config File

Claude Desktop config is typically at:
- **Linux**: `~/.config/Claude/claude_desktop_config.json`
- **macOS**: `~/Library/Application Support/Claude/claude_desktop_config.json`
- **Windows**: `%APPDATA%/Claude/claude_desktop_config.json`

### Add MCP Server Configuration

Edit the config file and add:

```json
{
  "mcpServers": {
    "docjl-editor": {
      "command": "/home/petitan/MongoLite/mcp-server/target/release/mcp-docjl-server",
      "args": [],
      "env": {
        "DOCJL_CONFIG": "/home/petitan/MongoLite/mcp-server/config.toml",
        "RUST_LOG": "info"
      }
    }
  }
}
```

If you already have other MCP servers, add the `docjl-editor` entry to the existing `mcpServers` object.

## Step 3: Prepare the Server Config

Ensure `config.toml` exists:

```toml
# MCP DOCJL Server Configuration
host = "127.0.0.1"
port = 8080
ironbase_path = "./docjl_storage.mlite"
audit_log_path = "./audit.log"
require_auth = false  # Set to true for production

[[api_keys]]
key = "dev_key_12345"
name = "Development Key"
allowed_commands = ["mcp_docjl_*"]
allowed_documents = ["*"]

[api_keys.rate_limit]
requests_per_minute = 100
writes_per_minute = 10
```

## Step 4: Create Test Documents

Seed the database with test documents:

```bash
python3 seed_real_db.py
```

This creates sample DOCJL documents that Claude can edit.

## Step 5: Restart Claude Desktop

After modifying the config:
1. Quit Claude Desktop completely
2. Restart Claude Desktop
3. The MCP server will start automatically when Claude needs it

## Step 6: Test the Integration

In Claude Desktop, try commands like:

```
List all documents in the DOCJL database
```

```
Show me the outline of document "test_doc_1"
```

```
Add a new paragraph to test_doc_1 with content "This is a test paragraph"
```

```
Search for blocks containing "requirement" in all documents
```

## Available MCP Commands

Claude can use these commands through the MCP protocol:

1. **mcp_docjl_list_documents** - List all documents
2. **mcp_docjl_get_document** - Get full document
3. **mcp_docjl_list_headings** - Get table of contents
4. **mcp_docjl_insert_block** - Add new blocks
5. **mcp_docjl_update_block** - Modify blocks
6. **mcp_docjl_move_block** - Reorganize structure
7. **mcp_docjl_delete_block** - Remove blocks
8. **mcp_docjl_search_blocks** - Find content
9. **mcp_docjl_validate_references** - Check cross-references
10. **mcp_docjl_validate_schema** - Validate DOCJL compliance
11. **mcp_docjl_get_audit_log** - View operation history

## Troubleshooting

### Server doesn't start

Check logs in `audit.log`:
```bash
tail -f /home/petitan/MongoLite/mcp-server/audit.log
```

### Permission errors

Ensure binary is executable:
```bash
chmod +x target/release/mcp-docjl-server
```

### Database not found

Check that `ironbase_path` in config.toml points to a valid location.

### Claude can't see the MCP server

1. Check Claude Desktop config syntax (valid JSON)
2. Restart Claude Desktop completely
3. Check server binary path is absolute (not relative)
4. Verify environment variables are set correctly

## Example Usage Session

**User:** "List all available DOCJL documents"

**Claude (using mcp_docjl_list_documents):**
```
Found 2 documents:
1. test_doc_1 - "Requirements Specification" (5 blocks)
2. test_doc_2 - "Test Plan" (3 blocks)
```

**User:** "Show me the structure of test_doc_1"

**Claude (using mcp_docjl_list_headings):**
```
Document Outline for test_doc_1:
1. Introduction (sec:1)
   1.1 Purpose (sec:1.1)
2. Requirements (sec:2)
   2.1 Functional Requirements (sec:2.1)
   2.2 Non-Functional Requirements (sec:2.2)
```

**User:** "Add a new requirement about performance under sec:2.1"

**Claude (using mcp_docjl_insert_block):**
```
✅ Added new requirement block (req:5)
   Content: "The system shall respond to user requests within 200ms"
   Location: Inside sec:2.1
```

## Advanced: Custom Document Types

You can create custom DOCJL schemas for different document types:

```json
{
  "_id": "my_custom_doc",
  "title": "My Custom Document",
  "version": "1.0",
  "docjll": [
    {
      "type": "section",
      "title": "Custom Section",
      "label": "sec:1",
      "children": [...]
    }
  ]
}
```

Then import via Python:

```python
from ironbase import IronBase

db = IronBase("docjl_storage.mlite")
coll = db.collection("documents")
coll.insert_one(custom_doc)
```

## Security Notes

For production use:
1. Set `require_auth = true` in config.toml
2. Use strong API keys
3. Configure rate limits appropriately
4. Review audit logs regularly
5. Restrict `allowed_documents` to specific document IDs

## Performance Tips

- Use `--release` build for production (10x faster)
- Enable memory-mapped I/O for large documents
- Monitor with `cargo flamegraph` for bottlenecks
- Run compaction periodically if database grows large

## Support

For issues or questions:
- Check the audit log: `tail -f audit.log`
- Run tests: `cargo test`
- View server status: Check Claude Desktop → Settings → MCP Servers

---

**Status:** ✅ Ready for production use
**Version:** 0.1.0
**Last Updated:** 2025-11-21
