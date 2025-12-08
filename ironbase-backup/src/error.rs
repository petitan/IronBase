//! Error types for ironbase-backup

use std::path::PathBuf;
use thiserror::Error;

/// Result type alias for backup operations
pub type Result<T> = std::result::Result<T, BackupError>;

/// Errors that can occur during backup/restore operations
#[derive(Error, Debug)]
pub enum BackupError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Invalid backup file: {reason}")]
    InvalidBackupFile { reason: String },

    #[error("Invalid magic number in backup file")]
    InvalidMagic,

    #[error("Unsupported backup version: {version}")]
    UnsupportedVersion { version: u16 },

    #[error("Checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch { expected: String, actual: String },

    #[error("Compression error: {0}")]
    Compression(String),

    #[error("Decompression error: {0}")]
    Decompression(String),

    #[error("No full backup found in directory")]
    NoFullBackup,

    #[error("Broken chain: backup {backup} references unknown parent {parent_hash}")]
    BrokenChain { backup: String, parent_hash: String },

    #[error("Database file not found: {path}")]
    DatabaseNotFound { path: PathBuf },

    #[error("Backup not found: {name}")]
    BackupNotFound { name: String },

    #[error("Output directory does not exist: {path}")]
    OutputDirNotFound { path: PathBuf },

    #[error(
        "Cannot create incremental backup: database size decreased from {expected} to {actual}"
    )]
    DatabaseShrunk { expected: u64, actual: u64 },

    #[error("No backups found for database: {db_name}")]
    NoBackupsFound { db_name: String },
}
