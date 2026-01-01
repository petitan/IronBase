//! Backup verification and integrity checking

use crate::chain::{read_backup_info, Chain};
use crate::color::{green, red};
use crate::compression::format_size;
use crate::error::Result;
use crate::format::{hash_to_hex, hash_to_short_hex, FOOTER_SIZE, HEADER_SIZE};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

/// Result of verifying a single backup
#[derive(Debug)]
pub struct VerifyResult {
    /// Backup filename
    pub filename: String,
    /// Whether the backup is valid
    pub valid: bool,
    /// Expected hash (from footer)
    pub expected_hash: [u8; 32],
    /// Actual computed hash
    pub actual_hash: [u8; 32],
    /// Error message if invalid
    pub error: Option<String>,
}

/// Verify a single backup file's integrity
pub fn verify_backup(path: &Path) -> Result<VerifyResult> {
    let filename = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();

    // Read backup info (header + footer hash)
    let info = read_backup_info(path)?;
    let expected_hash = info.hash;

    // Calculate actual hash
    let actual_hash = calculate_backup_hash(path)?;

    let valid = expected_hash == actual_hash;
    let error = if valid {
        None
    } else {
        Some(format!(
            "Hash mismatch: expected {}, got {}",
            hash_to_hex(&expected_hash),
            hash_to_hex(&actual_hash)
        ))
    };

    Ok(VerifyResult {
        filename,
        valid,
        expected_hash,
        actual_hash,
        error,
    })
}

/// Calculate SHA256 hash of backup content (header + payload)
fn calculate_backup_hash(path: &Path) -> Result<[u8; 32]> {
    let file = File::open(path)?;
    let file_size = file.metadata()?.len();
    let mut reader = BufReader::new(file);

    let mut hasher = Sha256::new();

    // Hash header
    let mut header_buf = vec![0u8; HEADER_SIZE];
    reader.read_exact(&mut header_buf)?;
    hasher.update(&header_buf);

    // Hash payload (everything between header and footer)
    let payload_size = file_size - HEADER_SIZE as u64 - FOOTER_SIZE as u64;
    let mut payload_buf = vec![0u8; 8192]; // Read in chunks

    let mut remaining = payload_size as usize;
    while remaining > 0 {
        let to_read = remaining.min(payload_buf.len());
        reader.read_exact(&mut payload_buf[..to_read])?;
        hasher.update(&payload_buf[..to_read]);
        remaining -= to_read;
    }

    Ok(hasher.finalize().into())
}

/// Verify an entire backup chain
pub fn verify_chain(chain: &Chain) -> Vec<VerifyResult> {
    let mut results = Vec::new();

    for backup in &chain.backups {
        match verify_backup(&backup.path) {
            Ok(result) => results.push(result),
            Err(e) => results.push(VerifyResult {
                filename: backup.filename().to_string(),
                valid: false,
                expected_hash: backup.hash,
                actual_hash: [0u8; 32],
                error: Some(e.to_string()),
            }),
        }
    }

    results
}

/// Print verification results
pub fn print_verify_results(results: &[VerifyResult]) {
    println!("Verifying backup chain integrity...\n");

    let mut valid_count = 0;
    let total = results.len();

    for result in results {
        let status = if result.valid {
            valid_count += 1;
            green("OK")
        } else {
            red("FAIL")
        };

        println!(
            "  {}: {} [{}]",
            result.filename,
            status,
            hash_to_short_hex(&result.expected_hash)
        );

        if let Some(ref error) = result.error {
            println!("    Error: {}", error);
        }
    }

    println!();
    if valid_count == total {
        println!(
            "{}",
            green(&format!(
                "Chain valid: {}/{} backups verified",
                valid_count, total
            ))
        );
    } else {
        println!(
            "{}",
            red(&format!(
                "Chain invalid: {}/{} backups verified",
                valid_count, total
            ))
        );
    }
}

/// Get backup info and print it
pub fn print_backup_info(path: &Path) -> Result<()> {
    let info = read_backup_info(path)?;

    let type_str = if info.is_full() {
        "Full"
    } else {
        "Incremental"
    };

    let timestamp = chrono::DateTime::from_timestamp(info.header.timestamp, 0)
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let compression_ratio = if info.header.compressed_length > 0 {
        info.header.data_length as f64 / info.header.compressed_length as f64
    } else {
        0.0
    };

    println!("Backup Information");
    println!("==================");
    println!("File:        {}", path.display());
    println!("Database:    {}", info.header.db_name_str());
    println!("Type:        {}", type_str);
    println!("Created:     {}", timestamp);
    println!();
    println!("Data:");
    println!(
        "  Original size:   {}",
        format_size(info.header.data_length)
    );
    println!(
        "  Compressed size: {}",
        format_size(info.header.compressed_length)
    );
    println!("  Compression:     {:.1}x", compression_ratio);
    println!(
        "  DB size at backup: {}",
        format_size(info.header.original_db_size)
    );
    println!("  Start offset:    {}", info.header.start_offset);
    println!();
    println!("Integrity:");
    println!("  Content hash:    {}", hash_to_hex(&info.hash));

    if !info.is_full() {
        println!(
            "  Parent hash:     {}",
            hash_to_hex(&info.header.parent_hash)
        );
    }

    // Verify
    println!();
    print!("Verifying... ");
    match verify_backup(path) {
        Ok(result) if result.valid => println!("{}", green("OK")),
        Ok(result) => println!(
            "{}",
            red(&format!("FAILED: {}", result.error.unwrap_or_default()))
        ),
        Err(e) => println!("{}", red(&format!("ERROR: {}", e))),
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::create_backup;
    use tempfile::TempDir;

    #[test]
    fn test_verify_backup() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");
        let backup_dir = temp_dir.path().join("backups");

        // Create test database
        std::fs::write(&db_path, b"test content".repeat(100)).unwrap();

        // Create backup (no split)
        let result = create_backup(&db_path, &backup_dir, false, None).unwrap();

        // Verify
        let verify_result = verify_backup(&result.path).unwrap();
        assert!(verify_result.valid);
    }
}
