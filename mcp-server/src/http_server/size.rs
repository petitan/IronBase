//! Size parsing and formatting utilities
//!
//! Human-readable size strings like "1GB", "500MB", "10KB"

/// Parse human-readable size strings like "1GB", "500MB", "10KB"
///
/// Supported suffixes (case-insensitive):
/// - B (bytes)
/// - KB (kilobytes, 1024 bytes)
/// - MB (megabytes, 1024^2 bytes)
/// - GB (gigabytes, 1024^3 bytes)
///
/// Examples: "1GB", "500MB", "10KB", "1024B", "1 GB" (spaces allowed)
pub(crate) fn parse_size(s: &str) -> Result<usize, String> {
    let s = s.trim().to_uppercase();

    // Find where the number ends and the suffix begins
    let (num_part, suffix) = if let Some(pos) = s.find(|c: char| c.is_alphabetic()) {
        let (n, s) = s.split_at(pos);
        (n.trim(), s.trim())
    } else {
        // No suffix, assume bytes
        (s.as_str(), "B")
    };

    let number: f64 = num_part
        .parse()
        .map_err(|_| format!("Invalid number: '{}'", num_part))?;

    let multiplier: usize = match suffix {
        "B" | "" => 1,
        "KB" | "K" => 1024,
        "MB" | "M" => 1024 * 1024,
        "GB" | "G" => 1024 * 1024 * 1024,
        _ => {
            return Err(format!(
                "Unknown size suffix: '{}'. Use B, KB, MB, or GB",
                suffix
            ))
        }
    };

    Ok((number * multiplier as f64) as usize)
}

/// Format bytes as human-readable size string
pub(crate) fn format_size(bytes: usize) -> String {
    const GB: usize = 1024 * 1024 * 1024;
    const MB: usize = 1024 * 1024;
    const KB: usize = 1024;

    if bytes >= GB && bytes.is_multiple_of(GB) {
        format!("{}GB", bytes / GB)
    } else if bytes >= MB && bytes.is_multiple_of(MB) {
        format!("{}MB", bytes / MB)
    } else if bytes >= KB && bytes.is_multiple_of(KB) {
        format!("{}KB", bytes / KB)
    } else if bytes >= GB {
        format!("{:.1}GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1}MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1}KB", bytes as f64 / KB as f64)
    } else {
        format!("{}B", bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_size_bytes() {
        assert_eq!(parse_size("100").unwrap(), 100);
        assert_eq!(parse_size("100B").unwrap(), 100);
        assert_eq!(parse_size("100b").unwrap(), 100);
    }

    #[test]
    fn test_parse_size_kilobytes() {
        assert_eq!(parse_size("1KB").unwrap(), 1024);
        assert_eq!(parse_size("1kb").unwrap(), 1024);
        assert_eq!(parse_size("1K").unwrap(), 1024);
        assert_eq!(parse_size("10KB").unwrap(), 10 * 1024);
    }

    #[test]
    fn test_parse_size_megabytes() {
        assert_eq!(parse_size("1MB").unwrap(), 1024 * 1024);
        assert_eq!(parse_size("1mb").unwrap(), 1024 * 1024);
        assert_eq!(parse_size("1M").unwrap(), 1024 * 1024);
        assert_eq!(parse_size("500MB").unwrap(), 500 * 1024 * 1024);
    }

    #[test]
    fn test_parse_size_gigabytes() {
        assert_eq!(parse_size("1GB").unwrap(), 1024 * 1024 * 1024);
        assert_eq!(parse_size("1gb").unwrap(), 1024 * 1024 * 1024);
        assert_eq!(parse_size("1G").unwrap(), 1024 * 1024 * 1024);
        assert_eq!(parse_size("2GB").unwrap(), 2 * 1024 * 1024 * 1024);
    }

    #[test]
    fn test_parse_size_with_spaces() {
        assert_eq!(parse_size("  1GB  ").unwrap(), 1024 * 1024 * 1024);
        assert_eq!(parse_size("1 GB").unwrap(), 1024 * 1024 * 1024);
    }

    #[test]
    fn test_parse_size_fractional() {
        assert_eq!(
            parse_size("1.5GB").unwrap(),
            (1.5 * 1024.0 * 1024.0 * 1024.0) as usize
        );
        assert_eq!(
            parse_size("0.5MB").unwrap(),
            (0.5 * 1024.0 * 1024.0) as usize
        );
    }

    #[test]
    fn test_parse_size_invalid() {
        assert!(parse_size("abc").is_err());
        assert!(parse_size("1TB").is_err()); // TB not supported
        assert!(parse_size("").is_err());
    }

    #[test]
    fn test_format_size() {
        assert_eq!(format_size(1024), "1KB");
        assert_eq!(format_size(1024 * 1024), "1MB");
        assert_eq!(format_size(1024 * 1024 * 1024), "1GB");
        assert_eq!(format_size(500 * 1024 * 1024), "500MB");
        assert_eq!(format_size(100), "100B");
    }

    #[test]
    fn test_format_size_fractional() {
        // 1.5 GB = 1536 MB (exact), so it shows as MB
        let size = (1.5 * 1024.0 * 1024.0 * 1024.0) as usize;
        assert_eq!(format_size(size), "1536MB");

        // Non-exact values show decimals
        let size2 = 1024 * 1024 * 1024 + 512 * 1024 * 1024; // 1.5 GB
        assert_eq!(format_size(size2), "1536MB");
    }
}
