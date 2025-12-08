//! Backup chain discovery and management
//!
//! Automatically discovers and orders backup files by following parent hash references.

use crate::error::{BackupError, Result};
use crate::format::{hash_to_short_hex, BackupFooter, BackupHeader, BackupType};
use std::fs::{self, File};
use std::io::{BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// Information about a single backup in the chain
#[derive(Debug, Clone)]
pub struct BackupInfo {
    /// Path to the backup file
    pub path: PathBuf,
    /// Backup header
    pub header: BackupHeader,
    /// SHA256 hash of this backup's content (from footer)
    pub hash: [u8; 32],
}

impl BackupInfo {
    /// Get the filename without path
    pub fn filename(&self) -> &str {
        self.path.file_name().and_then(|s| s.to_str()).unwrap_or("")
    }

    /// Check if this is a full backup
    pub fn is_full(&self) -> bool {
        self.header.backup_type == BackupType::Full
    }
}

/// A chain of backups for a single database
#[derive(Debug)]
pub struct Chain {
    /// Database name
    pub db_name: String,
    /// Ordered list of backups (full backup first, then incrementals in order)
    pub backups: Vec<BackupInfo>,
}

impl Chain {
    /// Discover and build a backup chain from a directory
    ///
    /// Scans the directory for .ibak files, reads their headers and footers,
    /// and builds an ordered chain by following parent hash references.
    pub fn discover(dir: &Path, db_name: &str) -> Result<Self> {
        if !dir.exists() {
            return Err(BackupError::OutputDirNotFound {
                path: dir.to_path_buf(),
            });
        }

        // Find all .ibak files
        let entries = fs::read_dir(dir)?;
        let mut backups = Vec::new();

        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("ibak") {
                match read_backup_info(&path) {
                    Ok(info) => {
                        // Filter by database name
                        if info.header.db_name_str() == db_name {
                            backups.push(info);
                        }
                    }
                    Err(e) => {
                        // Log warning but continue scanning
                        eprintln!("Warning: skipping invalid backup {}: {}", path.display(), e);
                    }
                }
            }
        }

        if backups.is_empty() {
            return Ok(Chain {
                db_name: db_name.to_string(),
                backups: vec![],
            });
        }

        // Build ordered chain
        let chain = build_chain(backups)?;

        Ok(Chain {
            db_name: db_name.to_string(),
            backups: chain,
        })
    }

    /// Discover all database chains in a directory
    pub fn discover_all(dir: &Path) -> Result<Vec<Chain>> {
        if !dir.exists() {
            return Err(BackupError::OutputDirNotFound {
                path: dir.to_path_buf(),
            });
        }

        // Find all unique database names
        let entries = fs::read_dir(dir)?;
        let mut db_names = std::collections::HashSet::new();

        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("ibak") {
                if let Ok(info) = read_backup_info(&path) {
                    db_names.insert(info.header.db_name_str().to_string());
                }
            }
        }

        // Build chain for each database
        let mut chains = Vec::new();
        for db_name in db_names {
            let chain = Chain::discover(dir, &db_name)?;
            if !chain.backups.is_empty() {
                chains.push(chain);
            }
        }

        Ok(chains)
    }

    /// Check if the chain is empty
    pub fn is_empty(&self) -> bool {
        self.backups.is_empty()
    }

    /// Get the last backup in the chain
    pub fn last(&self) -> Option<&BackupInfo> {
        self.backups.last()
    }

    /// Get total compressed size of all backups
    pub fn total_size(&self) -> u64 {
        self.backups
            .iter()
            .map(|b| b.header.compressed_length)
            .sum()
    }

    /// Find a backup by filename
    pub fn find_by_name(&self, name: &str) -> Result<usize> {
        self.backups
            .iter()
            .position(|b| b.filename() == name)
            .ok_or_else(|| BackupError::BackupNotFound {
                name: name.to_string(),
            })
    }

    /// Print chain summary
    pub fn print_summary(&self) {
        use crate::compression::format_size;

        println!(
            "Chain: {} ({} backups, {} total)",
            self.db_name,
            self.backups.len(),
            format_size(self.total_size())
        );

        for backup in &self.backups {
            let type_str = if backup.is_full() { "FULL" } else { "INCR" };
            let size_str = if backup.is_full() {
                format_size(backup.header.compressed_length)
            } else {
                format!("+{}", format_size(backup.header.compressed_length))
            };

            let timestamp = chrono::DateTime::from_timestamp(backup.header.timestamp, 0)
                .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                .unwrap_or_else(|| "unknown".to_string());

            println!(
                "  [{}] {} ({}) - {} [{}]",
                type_str,
                backup.filename(),
                size_str,
                timestamp,
                hash_to_short_hex(&backup.hash)
            );
        }
    }
}

/// Read backup info (header + hash) from a file
pub fn read_backup_info(path: &Path) -> Result<BackupInfo> {
    let file = File::open(path)?;
    let file_size = file.metadata()?.len();
    let mut reader = BufReader::new(file);

    // Read header
    let header = BackupHeader::read_from(&mut reader)?;

    // Seek to footer
    let footer_offset = file_size - crate::format::FOOTER_SIZE as u64;
    reader.seek(SeekFrom::Start(footer_offset))?;

    // Read footer
    let footer = BackupFooter::read_from(&mut reader)?;

    Ok(BackupInfo {
        path: path.to_path_buf(),
        header,
        hash: footer.content_hash,
    })
}

/// Build an ordered chain from unordered backup infos
fn build_chain(mut backups: Vec<BackupInfo>) -> Result<Vec<BackupInfo>> {
    // Find full backup(s)
    let full_indices: Vec<_> = backups
        .iter()
        .enumerate()
        .filter(|(_, b)| b.header.backup_type == BackupType::Full)
        .map(|(i, _)| i)
        .collect();

    if full_indices.is_empty() {
        return Err(BackupError::NoFullBackup);
    }

    if full_indices.len() > 1 {
        // Multiple full backups - use the most recent one
        // This can happen after a "new full backup" command
        let latest_idx = full_indices
            .into_iter()
            .max_by_key(|&i| backups[i].header.timestamp)
            .unwrap();

        // Save the timestamp before retain (to avoid borrow issues)
        let latest_timestamp = backups[latest_idx].header.timestamp;

        // Remove older full backups from consideration
        backups.retain(|b| {
            b.header.backup_type != BackupType::Full || b.header.timestamp == latest_timestamp
        });
    }

    // Find the full backup
    let full_idx = backups
        .iter()
        .position(|b| b.header.backup_type == BackupType::Full)
        .ok_or(BackupError::NoFullBackup)?;

    let mut chain = vec![backups.remove(full_idx)];

    // Follow parent chain to build ordered list
    while !backups.is_empty() {
        let last_hash = chain.last().unwrap().hash;

        // Find backup whose parent_hash matches last_hash
        let next_idx = backups
            .iter()
            .position(|b| b.header.parent_hash == last_hash);

        match next_idx {
            Some(idx) => {
                chain.push(backups.remove(idx));
            }
            None => {
                // No more children found - remaining backups are orphans
                if !backups.is_empty() {
                    eprintln!(
                        "Warning: {} orphan backup(s) not part of chain",
                        backups.len()
                    );
                    for orphan in &backups {
                        eprintln!(
                            "  - {} (parent: {})",
                            orphan.filename(),
                            hash_to_short_hex(&orphan.header.parent_hash)
                        );
                    }
                }
                break;
            }
        }
    }

    Ok(chain)
}

/// Extract database name from .mlite path
pub fn db_name_from_path(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("database")
        .to_string()
}

/// Detect database name from backup directory (uses first found backup)
pub fn detect_db_name(dir: &Path) -> Result<String> {
    let entries = fs::read_dir(dir)?;

    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("ibak") {
            if let Ok(info) = read_backup_info(&path) {
                return Ok(info.header.db_name_str().to_string());
            }
        }
    }

    Err(BackupError::NoBackupsFound {
        db_name: "any".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_db_name_from_path() {
        assert_eq!(db_name_from_path(Path::new("/data/emails.mlite")), "emails");
        assert_eq!(db_name_from_path(Path::new("test.mlite")), "test");
    }
}
