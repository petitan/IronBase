# Security Policy

## Supported Versions

| Version | Supported          |
| ------- | ------------------ |
| 0.3.x   | :white_check_mark: |
| < 0.3   | :x:                |

## Reporting a Vulnerability

We take security vulnerabilities seriously. If you discover a security issue, please report it responsibly.

### How to Report

**DO NOT** open a public GitHub issue for security vulnerabilities.

Instead, please report security issues via:

1. **GitHub Security Advisories**: Use the "Report a vulnerability" button in the Security tab of this repository
2. **Email**: Contact the maintainer directly (if GitHub advisories are not available)

### What to Include

When reporting a vulnerability, please include:

- Description of the vulnerability
- Steps to reproduce the issue
- Affected versions
- Potential impact
- Any suggested fixes (optional)

### Response Timeline

- **Acknowledgment**: Within 48 hours
- **Initial Assessment**: Within 7 days
- **Resolution Timeline**: Depends on severity
  - Critical: 7 days
  - High: 14 days
  - Medium: 30 days
  - Low: 60 days

### Disclosure Policy

- We follow [responsible disclosure](https://en.wikipedia.org/wiki/Responsible_disclosure)
- We will coordinate with you on disclosure timing
- Credit will be given to reporters (unless anonymity is requested)

## Security Considerations

### What IronBase Provides

- **Data Integrity**: CRC32 checksums for WAL entries
- **Crash Recovery**: Write-Ahead Logging (WAL) with automatic replay
- **Transaction Support**: ACD (Atomicity, Consistency, Durability) guarantees

### Known Limitations

IronBase is designed as a lightweight embedded database. Please be aware of these architectural limitations:

#### Not ACID Compliant

IronBase provides **ACD** (Atomicity, Consistency, Durability) but **NOT full ACID**:

- **No Isolation**: No MVCC (Multi-Version Concurrency Control)
- **Single-writer model**: Concurrent writes are serialized
- Suitable for: Single-process applications, local data storage, development/testing

#### Not Designed For

- Multi-user concurrent access
- Networked database scenarios
- Mission-critical financial systems
- Medical or safety-critical applications

### Best Practices

1. **File Permissions**: Ensure database files have appropriate filesystem permissions
2. **Backup Strategy**: Implement regular backups for important data
3. **Input Validation**: Validate data before storing (use JSON schema validation feature)
4. **Error Handling**: Always handle errors from database operations
5. **Resource Cleanup**: Properly close database connections to ensure data persistence

### Dependency Security

We use the following practices:

- Regular dependency updates via Dependabot
- `cargo audit` for Rust dependency vulnerabilities
- Minimal dependency footprint

## Security Features

### JSON Schema Validation

IronBase supports JSON schema validation to ensure data integrity:

```python
db.set_collection_schema("users", {
    "type": "object",
    "required": ["email"],
    "properties": {
        "email": {"type": "string", "format": "email"},
        "age": {"type": "integer", "minimum": 0}
    }
})
```

### Durability Modes

Choose appropriate durability based on your needs:

- **Safe Mode**: Every operation commits immediately (recommended for important data)
- **Batch Mode**: Periodic commits (balance between performance and safety)
- **Unsafe Mode**: Manual commits only (highest performance, use with caution)

## Contact

For security-related questions that are not vulnerabilities, please open a [GitHub Discussion](https://github.com/petitan/IronBase/discussions).
