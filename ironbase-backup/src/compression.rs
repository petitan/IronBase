//! Compression utilities using zstd

use crate::error::{BackupError, Result};

/// Default zstd compression level (3 is a good balance of speed and ratio)
pub const DEFAULT_COMPRESSION_LEVEL: i32 = 3;

/// Compress data using zstd
pub fn compress(data: &[u8]) -> Result<Vec<u8>> {
    compress_with_level(data, DEFAULT_COMPRESSION_LEVEL)
}

/// Compress data using zstd with custom level (1-22)
pub fn compress_with_level(data: &[u8], level: i32) -> Result<Vec<u8>> {
    zstd::encode_all(data, level).map_err(|e| BackupError::Compression(e.to_string()))
}

/// Decompress zstd compressed data
pub fn decompress(data: &[u8]) -> Result<Vec<u8>> {
    zstd::decode_all(data).map_err(|e| BackupError::Decompression(e.to_string()))
}

/// Calculate compression ratio
pub fn compression_ratio(original: usize, compressed: usize) -> f64 {
    if compressed == 0 {
        return 0.0;
    }
    original as f64 / compressed as f64
}

/// Format size as human-readable string
pub fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compress_decompress_roundtrip() {
        let original = b"Hello, World! This is a test of compression.".repeat(100);

        let compressed = compress(&original).unwrap();
        let decompressed = decompress(&compressed).unwrap();

        assert_eq!(original.as_slice(), decompressed.as_slice());
        assert!(compressed.len() < original.len());
    }

    #[test]
    fn test_compression_ratio() {
        let ratio = compression_ratio(1000, 200);
        assert!((ratio - 5.0).abs() < 0.001);
    }

    #[test]
    fn test_format_size() {
        assert_eq!(format_size(500), "500 B");
        assert_eq!(format_size(1024), "1.00 KB");
        assert_eq!(format_size(1024 * 1024), "1.00 MB");
        assert_eq!(format_size(1024 * 1024 * 1024), "1.00 GB");
        assert_eq!(format_size(1536 * 1024), "1.50 MB");
    }
}
