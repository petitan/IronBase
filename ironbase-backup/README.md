# IronBase Backup Tool

Command-line backup and restore utility for IronBase databases.

## Features

- **Hot backup** - Backup running databases without stopping them
- **Incremental backups** - Only backup changed data since last backup
- **Compression** - Zstd compression for efficient storage
- **Integrity verification** - SHA-256 checksums for all backups
- **Cross-platform** - Works on Linux, macOS, and Windows

## Installation

### Pre-built Binaries

Download from [GitHub Releases](https://github.com/petitan/IronBase/releases):

```bash
# Linux
curl -L https://github.com/petitan/IronBase/releases/latest/download/ironbase-backup-linux-x64.tar.gz | tar xz

# Windows (PowerShell)
Invoke-WebRequest -Uri https://github.com/petitan/IronBase/releases/latest/download/ironbase-backup-windows-x64.zip -OutFile backup.zip
Expand-Archive backup.zip -DestinationPath .
```

### Build from Source

```bash
cd ironbase-backup
cargo build --release
# Binary: ./target/release/ironbase-backup
```

## Usage

### Full Backup

```bash
# Backup running database
ironbase-backup backup --db /path/to/database.mlite --output ./backups --full

# Output: ./backups/backup_20241213_143022_full.tar.zst
```

### Incremental Backup

```bash
# First backup is always full
ironbase-backup backup --db /path/to/database.mlite --output ./backups

# Subsequent backups are incremental (only changed data)
ironbase-backup backup --db /path/to/database.mlite --output ./backups

# Force full backup
ironbase-backup backup --db /path/to/database.mlite --output ./backups --full
```

### Restore

```bash
# Restore from backup
ironbase-backup restore --backup ./backups/backup_xxx.tar.zst --output /path/to/restored.mlite

# Restore with verification
ironbase-backup restore --backup ./backups/backup_xxx.tar.zst --output /path/to/restored.mlite --verify
```

### List Backups

```bash
ironbase-backup list --dir ./backups
```

### Verify Backup

```bash
ironbase-backup verify --backup ./backups/backup_xxx.tar.zst
```

## Hot Backup - How It Works

IronBase uses **append-only storage**, which enables safe hot backups without locks:

```
┌─────────────────────────────────────────────────┐
│ Database File (.mlite)                          │
├─────────────────────────────────────────────────┤
│ [Header] [doc1] [doc2] [doc3] | [NEW_DOC]       │
│          ↑                    ↑                 │
│          └── IMMUTABLE ───────┘  (new writes)   │
│              (safe to read)                     │
└─────────────────────────────────────────────────┘
```

**Why it's safe:**
1. Documents are **never modified in place** (update = new doc + tombstone)
2. `data_end_offset` marks the boundary of immutable data
3. Backup reads only immutable data - no locks needed
4. New data written during backup → included in next incremental

**Concurrent writes detection:**
```bash
$ ironbase-backup backup --db /path/to/db.mlite --output ./backups
Creating backup...
Note: Database was modified during backup (5 new documents)
      These will be included in the next incremental backup.
Backup complete: backup_20241213_143022_full.tar.zst
```

## Backup Format

```
backup_YYYYMMDD_HHMMSS_{full|incr}.tar.zst
├── manifest.json       # Backup metadata
├── data.bin           # Document data (compressed)
├── metadata.json      # Collection/index metadata
└── checksum.sha256    # Integrity verification
```

### Manifest Structure

```json
{
  "version": 1,
  "type": "full",
  "created_at": "2024-12-13T14:30:22Z",
  "source_path": "/path/to/database.mlite",
  "data_end_offset": 1048576,
  "document_count": 5000,
  "compressed_size": 524288,
  "checksum": "sha256:abc123..."
}
```

## Windows Compatibility

On Windows, the database uses a **separate lock file** (`.mlite.lock`) to avoid mandatory file lock issues:

```
database.mlite      ← No lock (readable by backup)
database.mlite.lock ← Exclusive lock (writer coordination)
```

This allows hot backups on Windows without any modifications.

## Command Reference

```
USAGE:
    ironbase-backup <COMMAND>

COMMANDS:
    backup   Create a backup of the database
    restore  Restore a database from backup
    list     List available backups
    verify   Verify backup integrity
    help     Print help information

OPTIONS:
    -h, --help       Print help information
    -V, --version    Print version information
```

### backup

```
USAGE:
    ironbase-backup backup [OPTIONS] --db <PATH> --output <DIR>

OPTIONS:
    -d, --db <PATH>       Path to database file
    -o, --output <DIR>    Output directory for backups
    -f, --full            Force full backup (ignore incremental)
    -v, --verbose         Verbose output
```

### restore

```
USAGE:
    ironbase-backup restore [OPTIONS] --backup <FILE> --output <PATH>

OPTIONS:
    -b, --backup <FILE>   Path to backup file
    -o, --output <PATH>   Output path for restored database
        --verify          Verify backup integrity before restore
    -f, --force           Overwrite existing file
```

## Error Handling

| Error | Cause | Solution |
|-------|-------|----------|
| `DatabaseLocked` | Another process has exclusive lock | Wait or use --force |
| `BackupCorrupted` | Checksum mismatch | Re-download or restore from different backup |
| `InsufficientSpace` | Not enough disk space | Free up space or use different output directory |

## Performance

| Operation | Speed | Notes |
|-----------|-------|-------|
| Full backup | ~100-500 MB/s | Depends on disk speed |
| Incremental | ~200-800 MB/s | Only changed data |
| Restore | ~100-300 MB/s | Includes decompression |
| Compression ratio | ~3-5x | Typical JSON documents |

## License

MIT License - see [LICENSE](../LICENSE) for details.
