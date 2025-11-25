# MCP DOCJL Server

AI-assisted DOCJL document editing server implementing the Model Context Protocol (MCP).

## Features

### Core Features
- **Secure API**: Authentication, authorization, and rate limiting
- **Audit Logging**: Complete audit trail of all operations
- **Schema Validation**: Enforce DOCJL document structure
- **Label Management**: Automatic label generation and renumbering
- **Cross-Reference Tracking**: Validate and update document references
- **Transaction Support**: Rollback on validation failure

### MCP Protocol Support (NEW!)
- **Full MCP Implementation**: All MCP protocol logic in Rust server
- **11 Tools**: Complete DOCJL document operations (9 core + 2 chunking tools)
- **Resources**: Documents exposed as `docjl://document/{id}` resources
- **15 Prompts**: 10 Balanced MVP + 5 ISO 17025 Calibration prompts
- **Claude Desktop Integration**: STDIO bridge for seamless desktop integration
- **HTTP API**: Full MCP functionality over HTTP for all clients
- **Chunking Support**: Handle large documents with section-based retrieval and token estimation

## Quick Start

### Build

```bash
cargo build --release
```

### Configuration

Copy the example configuration:

```bash
cp config.example.toml config.toml
```

Edit `config.toml` with your settings.

### Run

```bash
cargo run --release
```

Or use the compiled binary:

```bash
./target/release/mcp-docjl-server
```

### Test

```bash
# Health check
curl http://localhost:8080/health

# List documents
curl -X POST http://localhost:8080/mcp \
  -H "Authorization: Bearer test_key_12345" \
  -H "Content-Type: application/json" \
  -d '{"method": "mcp_docjl_list_documents", "params": {}}'
```

## API Commands

See [docs/MCP_DOCJL_SPEC.md](../docs/MCP_DOCJL_SPEC.md) for complete API documentation.

### Read Operations

- `mcp_docjl_list_documents` - List all documents
- `mcp_docjl_get_document` - Retrieve document
- `mcp_docjl_list_headings` - Get document outline
- `mcp_docjl_search_blocks` - Search for blocks
- `mcp_docjl_search_content` - Full-text content search
- `mcp_docjl_get_section` - Get specific section with controlled depth (Phase 3: Chunking Support)
- `mcp_docjl_estimate_tokens` - Estimate token count for documents/sections (Phase 3: Chunking Support)

### Write Operations

- `mcp_docjl_insert_block` - Insert new block
- `mcp_docjl_update_block` - Update existing block
- `mcp_docjl_move_block` - Move block to new location
- `mcp_docjl_delete_block` - Delete block (with cascade)

### Validation

- `mcp_docjl_validate_references` - Check cross-references
- `mcp_docjl_validate_schema` - Validate document schema

### Audit

- `mcp_docjl_get_audit_log` - Retrieve audit log entries

## Architecture

```
src/
├── main.rs          # HTTP server and JSON-RPC handler
├── lib.rs           # Library entry point
├── domain/          # DOCJL domain logic
│   ├── block.rs     # Block types and structures
│   ├── document.rs  # Document operations
│   ├── label.rs     # Label generation and renumbering
│   ├── reference.rs # Cross-reference tracking
│   └── validation.rs # Schema validation
└── host/            # MCP host layer
    ├── security.rs  # Authentication and authorization
    └── audit.rs     # Audit logging
```

## Security

### Authentication

All requests require an API key in the `Authorization` header:

```
Authorization: Bearer your_api_key_here
```

### Command Whitelist

API keys can be restricted to specific commands (see `config.toml`).

### Rate Limiting

- Default: 100 requests/minute
- Write operations: 10 requests/minute (configurable per key)

### Document Access Control

API keys can be restricted to specific documents or use wildcard `*` for all.

## Development

### Run Tests

```bash
cargo test
```

### Run with Debug Logging

```bash
RUST_LOG=debug cargo run
```

### Generate API Documentation

```bash
cargo doc --open
```

## Integration with IronBase

The server will integrate with IronBase for document storage:

```rust
// TODO: Implement IronBase adapter
use ironbase::IronBase;

let db = IronBase::open("docjl_storage.mlite")?;
let collection = db.collection("documents")?;
```

## Roadmap

- [x] Phase 1: Core infrastructure, auth, audit
- [x] Phase 2: Read operations (list, get, search)
- [x] Phase 3: Chunking support (get_section, estimate_tokens)
- [x] Phase 4: Write operations (insert, update, delete)
- [x] MCP Integration: Full protocol support with 15 prompts
- [ ] Phase 5: Performance optimization
- [ ] Phase 6: Production hardening

## License

MIT
