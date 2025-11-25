# DOCJL MCP Server Specification

## Overview

This document specifies the MCP (Model Context Protocol) server implementation for AI-assisted editing of DOCJL (Document JSON Layout) documents. The server provides a safe, auditable interface for Claude AI to manipulate structured documents while maintaining schema compliance and referential integrity.

## Document Structure Analysis

### DOCJL Block Types

Based on the ISO17025 example, DOCJL supports these block types:

```typescript
type Block =
  | Paragraph
  | Heading
  | Table
  | List
  | Section
  | Image
  | Code
  | Reference

interface Paragraph {
  type: "paragraph"
  content: InlineContent[]
  label?: string
  compliance_note?: string
}

interface Heading {
  type: "heading"
  level: 1 | 2 | 3 | 4 | 5 | 6
  content: InlineContent[]
  label?: string
  children?: Block[]
}

interface Table {
  type: "table"
  headers: string[]
  rows: string[][]
  caption?: string
  label?: string
}

interface List {
  type: "list"
  ordered: boolean
  items: ListItem[]
  label?: string
}

interface Section {
  type: "section"
  title: string
  children: Block[]
  label?: string
}

type InlineContent =
  | { type: "text", content: string }
  | { type: "bold", content: string }
  | { type: "italic", content: string }
  | { type: "code", content: string }
  | { type: "link", href: string, content: string }
  | { type: "ref", target: string }  // Cross-reference
```

### Label System

Labels are unique identifiers for blocks that can be referenced:
- Format: `{prefix}:{number}` (e.g., `tab:1`, `fig:3`, `sec:4.2`)
- Prefixes: `tab` (table), `fig` (figure), `sec` (section), `para` (paragraph)
- Must be unique within a document
- Used for cross-references and audit trails

## MCP Commands

### 1. Document Management

#### `mcp_docjl_list_documents`
List all DOCJL documents in the database.

**Request:**
```json
{
  "method": "mcp_docjl_list_documents",
  "params": {
    "filter": {}  // Optional MongoDB-style query
  }
}
```

**Response:**
```json
{
  "documents": [
    {
      "id": "doc_123",
      "title": "ISO17025 Quality Manual",
      "version": "1.0",
      "blocks_count": 1146,
      "last_modified": "2024-07-19T10:30:00Z"
    }
  ]
}
```

#### `mcp_docjl_get_document`
Retrieve full document or specific sections.

**Request:**
```json
{
  "method": "mcp_docjl_get_document",
  "params": {
    "document_id": "doc_123",
    "sections": ["sec:4", "sec:5"],  // Optional: specific sections
    "depth": 2  // Optional: tree depth limit
  }
}
```

**Response:**
```json
{
  "document": {
    "id": "doc_123",
    "metadata": { ... },
    "docjll": [ ... ]
  }
}
```

### 2. Block Operations

#### `mcp_docjl_insert_block`
Insert a new block at a specific position.

**Request:**
```json
{
  "method": "mcp_docjl_insert_block",
  "params": {
    "document_id": "doc_123",
    "parent_label": "sec:4",  // Where to insert
    "position": "after",  // "before" | "after" | "inside" | "end"
    "anchor_label": "para:4.2",  // Optional: relative to this block
    "block": {
      "type": "paragraph",
      "content": [{ "type": "text", "content": "New paragraph text" }]
    }
  }
}
```

**Response:**
```json
{
  "success": true,
  "block_label": "para:4.3",  // Auto-generated label
  "audit_id": "audit_789",
  "affected_labels": ["para:4.3", "para:4.4"]  // Labels that were renumbered
}
```

#### `mcp_docjl_update_block`
Update an existing block's content.

**Request:**
```json
{
  "method": "mcp_docjl_update_block",
  "params": {
    "document_id": "doc_123",
    "block_label": "para:4.2",
    "updates": {
      "content": [{ "type": "text", "content": "Updated text" }],
      "compliance_note": "Revised per ISO requirement 4.2.1"
    }
  }
}
```

**Response:**
```json
{
  "success": true,
  "audit_id": "audit_790",
  "previous_version": { ... }  // For rollback
}
```

#### `mcp_docjl_move_block`
Move a block to a new location.

**Request:**
```json
{
  "method": "mcp_docjl_move_block",
  "params": {
    "document_id": "doc_123",
    "block_label": "para:4.2",
    "target_parent": "sec:5",
    "position": "end"
  }
}
```

**Response:**
```json
{
  "success": true,
  "new_label": "para:5.8",  // New label after move
  "audit_id": "audit_791",
  "affected_labels": [
    { "old": "para:4.2", "new": "para:5.8" },
    { "old": "para:4.3", "new": "para:4.2" }
  ]
}
```

#### `mcp_docjl_delete_block`
Delete a block (with cascade options for children).

**Request:**
```json
{
  "method": "mcp_docjl_delete_block",
  "params": {
    "document_id": "doc_123",
    "block_label": "sec:4",
    "cascade": true  // Delete children too
  }
}
```

**Response:**
```json
{
  "success": true,
  "deleted_count": 15,  // Including children
  "audit_id": "audit_792",
  "broken_references": ["para:6.2 -> sec:4"]  // Cross-refs that need attention
}
```

### 3. Navigation & Search

#### `mcp_docjl_list_headings`
Get document outline (table of contents).

**Request:**
```json
{
  "method": "mcp_docjl_list_headings",
  "params": {
    "document_id": "doc_123",
    "max_depth": 3
  }
}
```

**Response:**
```json
{
  "outline": [
    {
      "level": 1,
      "label": "sec:1",
      "title": "Introduction",
      "children": [
        { "level": 2, "label": "sec:1.1", "title": "Purpose" }
      ]
    }
  ]
}
```

#### `mcp_docjl_search_blocks`
Search for blocks by content, type, or label.

**Request:**
```json
{
  "method": "mcp_docjl_search_blocks",
  "params": {
    "document_id": "doc_123",
    "query": {
      "type": "paragraph",
      "content_contains": "calibration",
      "has_compliance_note": true
    }
  }
}
```

**Response:**
```json
{
  "results": [
    {
      "label": "para:4.2",
      "block": { ... },
      "path": ["sec:4", "sec:4.1", "para:4.2"]
    }
  ]
}
```

### 4. Cross-Reference Management

#### `mcp_docjl_validate_references`
Check for broken cross-references.

**Request:**
```json
{
  "method": "mcp_docjl_validate_references",
  "params": {
    "document_id": "doc_123"
  }
}
```

**Response:**
```json
{
  "valid": false,
  "issues": [
    {
      "source_label": "para:6.2",
      "target_label": "sec:99",
      "error": "Target does not exist"
    }
  ]
}
```

#### `mcp_docjl_update_references`
Update all references when a label changes.

**Request:**
```json
{
  "method": "mcp_docjl_update_references",
  "params": {
    "document_id": "doc_123",
    "label_changes": [
      { "old": "sec:4", "new": "sec:5" }
    ]
  }
}
```

**Response:**
```json
{
  "success": true,
  "updated_count": 12,
  "audit_id": "audit_793"
}
```

### 5. Schema Validation

#### `mcp_docjl_validate_schema`
Validate document against DOCJL schema.

**Request:**
```json
{
  "method": "mcp_docjl_validate_schema",
  "params": {
    "document_id": "doc_123"
  }
}
```

**Response:**
```json
{
  "valid": true,
  "errors": [],
  "warnings": [
    {
      "block_label": "tab:5",
      "message": "Table caption missing"
    }
  ]
}
```

### 6. Audit & History

#### `mcp_docjl_get_audit_log`
Retrieve change history for a document.

**Request:**
```json
{
  "method": "mcp_docjl_get_audit_log",
  "params": {
    "document_id": "doc_123",
    "limit": 50,
    "block_label": "sec:4"  // Optional: filter by block
  }
}
```

**Response:**
```json
{
  "entries": [
    {
      "audit_id": "audit_789",
      "timestamp": "2024-07-19T10:30:00Z",
      "operation": "insert_block",
      "user": "claude_mcp",
      "block_label": "para:4.3",
      "details": { ... }
    }
  ]
}
```

## Security Model

### Command Whitelist

Only these commands are allowed for AI agents:
- ✅ `mcp_docjl_list_documents` (read-only)
- ✅ `mcp_docjl_get_document` (read-only)
- ✅ `mcp_docjl_insert_block` (requires validation)
- ✅ `mcp_docjl_update_block` (requires validation)
- ✅ `mcp_docjl_move_block` (requires validation)
- ✅ `mcp_docjl_delete_block` (requires confirmation)
- ✅ `mcp_docjl_list_headings` (read-only)
- ✅ `mcp_docjl_search_blocks` (read-only)
- ✅ `mcp_docjl_validate_references` (read-only)
- ✅ `mcp_docjl_validate_schema` (read-only)
- ✅ `mcp_docjl_get_audit_log` (read-only)
- ❌ Direct database access (blocked)
- ❌ Schema modification (blocked)
- ❌ Collection operations (blocked)

### Authentication

```rust
pub struct McpAuth {
    api_key: String,
    allowed_documents: Vec<DocumentId>,
    rate_limit: RateLimit,
}
```

### Rate Limiting

- 100 requests per minute per API key
- 10 write operations per minute
- 1 MB payload size limit

## Error Handling

### Error Codes

```rust
pub enum McpError {
    InvalidCommand,           // 400
    Unauthorized,            // 401
    DocumentNotFound,        // 404
    BlockNotFound,           // 404
    SchemaValidationFailed,  // 422
    ReferenceError,          // 422
    RateLimitExceeded,       // 429
    InternalError,           // 500
}
```

### Error Response Format

```json
{
  "error": {
    "code": "SCHEMA_VALIDATION_FAILED",
    "message": "Block does not match DOCJL schema",
    "details": {
      "block_label": "para:4.2",
      "validation_errors": [
        "content.type must be one of: text, bold, italic"
      ]
    }
  }
}
```

## Implementation Phases

### Phase 1: Core Infrastructure (Week 1-2)
- [ ] MCP JSON-RPC server skeleton
- [ ] IronBase adapter with schema validation
- [ ] Basic authentication and rate limiting
- [ ] Audit log storage

### Phase 2: Read Operations (Week 2-3)
- [ ] `mcp_docjl_list_documents`
- [ ] `mcp_docjl_get_document`
- [ ] `mcp_docjl_list_headings`
- [ ] `mcp_docjl_search_blocks`

### Phase 3: Write Operations (Week 3-4)
- [ ] `mcp_docjl_insert_block`
- [ ] `mcp_docjl_update_block`
- [ ] Label auto-generation
- [ ] Schema validation on write

### Phase 4: Advanced Features (Week 4-5)
- [ ] `mcp_docjl_move_block` with label renumbering
- [ ] `mcp_docjl_delete_block` with cascade
- [ ] Cross-reference validation and updates
- [ ] Broken reference detection

### Phase 5: Production Hardening (Week 5-6)
- [ ] Comprehensive error handling
- [ ] Transaction rollback on validation failure
- [ ] Performance optimization (80k block stress test)
- [ ] Documentation and examples

## Testing Strategy

### Unit Tests
- Schema validation logic
- Label generation and renumbering
- Cross-reference tracking

### Integration Tests
- Full MCP command flow (JSON-RPC → Domain → IronBase)
- Transaction rollback scenarios
- Rate limiting and authentication

### Stress Tests
- 80,000 block document
- Concurrent modifications
- Large move operations (1000+ blocks)

## Performance Targets

- Read operations: < 100ms for documents up to 10k blocks
- Write operations: < 500ms including validation
- Move operations: < 2s for moving 100 blocks
- Schema validation: < 50ms per block

## Configuration

### Server Config (TOML)

```toml
[mcp_server]
host = "127.0.0.1"
port = 8080
max_connections = 100

[mcp_server.auth]
api_keys_file = "api_keys.json"
require_auth = true

[mcp_server.rate_limit]
requests_per_minute = 100
writes_per_minute = 10

[mcp_server.storage]
ironbase_path = "./docjl_storage.mlite"
audit_log_path = "./docjl_audit.log"

[mcp_server.validation]
schema_file = "docjl_schema.json"
strict_mode = true
```

## Example Usage (Python Client)

```python
import requests

# Connect to MCP server
mcp = requests.Session()
mcp.headers.update({"Authorization": "Bearer your_api_key"})

# List documents
response = mcp.post("http://localhost:8080/mcp", json={
    "method": "mcp_docjl_list_documents",
    "params": {}
})
docs = response.json()["documents"]

# Insert a new paragraph
response = mcp.post("http://localhost:8080/mcp", json={
    "method": "mcp_docjl_insert_block",
    "params": {
        "document_id": "doc_123",
        "parent_label": "sec:4",
        "position": "end",
        "block": {
            "type": "paragraph",
            "content": [{"type": "text", "content": "New compliance requirement"}]
        }
    }
})

print(f"Inserted block: {response.json()['block_label']}")
```

## Next Steps

1. Create Rust project structure in `/mcp-server`
2. Implement IronBase adapter with schema support
3. Build MCP JSON-RPC handler
4. Implement Domain API for DOCJL operations
5. Add comprehensive tests
6. Deploy and test with Claude AI
