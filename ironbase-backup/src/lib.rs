//! IronBase Backup Library
//!
//! Incremental backup and restore tool for IronBase databases.
//!
//! # Features
//!
//! - **Hot backup**: Database continues operating during backup
//! - **Incremental**: Only backs up new data since last backup
//! - **Compressed**: Uses zstd compression for efficient storage
//! - **Verified**: SHA256 checksums ensure data integrity
//! - **Chain management**: Automatic discovery and ordering of backup files
//!
//! # Example
//!
//! ```no_run
//! use ironbase_backup::{backup, restore, Chain};
//! use std::path::Path;
//!
//! // Create a backup
//! let result = backup::create_backup(
//!     Path::new("/data/mydb.mlite"),
//!     Path::new("/backups/"),
//!     false, // auto-detect full vs incremental
//!     None,  // no split (single file backup)
//! ).unwrap();
//! println!("Backup created: {}", result.path.display());
//!
//! // Create a split backup (2 GiB parts)
//! let result = backup::create_backup(
//!     Path::new("/data/mydb.mlite"),
//!     Path::new("/backups/"),
//!     false,
//!     Some(2 * 1024 * 1024 * 1024), // 2 GiB per part
//! ).unwrap();
//! println!("Backup created in {} parts", result.part_count);
//!
//! // Restore from backup
//! let result = restore::restore(
//!     Path::new("/backups/"),
//!     Path::new("/restored/mydb.mlite"),
//!     None, // restore to latest
//!     None, // auto-detect database name
//! ).unwrap();
//! println!("Restored {} bytes", result.size);
//! ```

pub mod backup;
pub mod chain;
pub mod color;
pub mod compression;
pub mod error;
pub mod format;
pub mod restore;
pub mod verify;

// Re-export commonly used types
pub use backup::{create_backup, BackupResult};
pub use chain::{BackupInfo, Chain};
pub use error::{BackupError, Result};
pub use format::{BackupHeader, BackupType};
pub use restore::{restore, RestoreResult};
pub use verify::{verify_backup, verify_chain};
