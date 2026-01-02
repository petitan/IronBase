//! Cross-platform system memory detection
//!
//! Provides functions to query available and total system RAM.
//! Uses `/proc/meminfo` on Linux, `libc::sysconf` on other Unix systems,
//! and falls back to safe defaults on unsupported platforms.

/// Get available system memory in bytes
///
/// Returns `Some(bytes)` on supported platforms, `None` if detection fails.
/// When detection fails, callers should use conservative fallback limits.
///
/// # Platform support
/// - **Linux**: Reads `MemAvailable` from `/proc/meminfo` (most accurate)
/// - **macOS/BSD**: Uses `sysconf(_SC_AVPHYS_PAGES)`
/// - **Windows**: Returns `None` (use fallback limits)
///
/// # Example
/// ```rust,ignore
/// use ironbase_core::aggregation::memory_info::get_available_memory_bytes;
///
/// if let Some(bytes) = get_available_memory_bytes() {
///     println!("Available RAM: {} MB", bytes / 1024 / 1024);
/// }
/// ```
pub fn get_available_memory_bytes() -> Option<u64> {
    #[cfg(unix)]
    {
        get_available_memory_unix()
    }

    #[cfg(not(unix))]
    {
        None // Use fallback defaults on unsupported platforms
    }
}

/// Get total system memory in bytes
///
/// Returns `Some(bytes)` on supported platforms, `None` if detection fails.
///
/// # Platform support
/// - **Unix/Linux/macOS**: Uses `sysconf(_SC_PHYS_PAGES)`
/// - **Windows**: Returns `None`
#[allow(dead_code)]
pub fn get_total_memory_bytes() -> Option<u64> {
    #[cfg(unix)]
    {
        get_total_memory_unix()
    }

    #[cfg(not(unix))]
    {
        None
    }
}

#[cfg(unix)]
fn get_available_memory_unix() -> Option<u64> {
    // Linux: parse /proc/meminfo (most accurate, includes buffers/cache as available)
    #[cfg(target_os = "linux")]
    {
        if let Some(mem) = parse_proc_meminfo_available() {
            return Some(mem);
        }
    }

    // Fallback: sysconf (POSIX, works on macOS and BSD too)
    // Note: _SC_AVPHYS_PAGES may not be available on all systems
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "freebsd"))]
    unsafe {
        let page_size = libc::sysconf(libc::_SC_PAGESIZE);

        // _SC_AVPHYS_PAGES is not defined on macOS, use _SC_PHYS_PAGES * 0.5 as estimate
        #[cfg(target_os = "macos")]
        {
            let total_pages = libc::sysconf(libc::_SC_PHYS_PAGES);
            if page_size > 0 && total_pages > 0 {
                // Estimate 50% of total as available (conservative)
                return Some((page_size as u64) * (total_pages as u64) / 2);
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            let avail_pages = libc::sysconf(libc::_SC_AVPHYS_PAGES);
            if page_size > 0 && avail_pages > 0 {
                return Some((page_size as u64) * (avail_pages as u64));
            }
        }
    }

    None
}

#[cfg(target_os = "linux")]
fn parse_proc_meminfo_available() -> Option<u64> {
    use std::fs::File;
    use std::io::{BufRead, BufReader};

    let file = File::open("/proc/meminfo").ok()?;
    let reader = BufReader::new(file);

    for line in reader.lines().map_while(Result::ok) {
        if line.starts_with("MemAvailable:") {
            // Format: "MemAvailable:    1234567 kB"
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                if let Ok(kb) = parts[1].parse::<u64>() {
                    return Some(kb * 1024); // Convert kB to bytes
                }
            }
        }
    }

    None
}

#[cfg(unix)]
#[allow(dead_code)]
fn get_total_memory_unix() -> Option<u64> {
    unsafe {
        let page_size = libc::sysconf(libc::_SC_PAGESIZE);
        let total_pages = libc::sysconf(libc::_SC_PHYS_PAGES);

        if page_size > 0 && total_pages > 0 {
            return Some((page_size as u64) * (total_pages as u64));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(unix)]
    fn test_get_available_memory_returns_some() {
        let mem = get_available_memory_bytes();
        assert!(mem.is_some(), "Memory detection should work on Unix");

        let bytes = mem.unwrap();
        assert!(bytes > 0, "Available memory should be > 0");

        // Sanity check: at least 100MB, less than 1TB
        let mb = bytes / 1024 / 1024;
        assert!(mb >= 100, "Available memory should be at least 100MB");
        assert!(
            mb < 1_000_000,
            "Available memory should be less than 1TB (got {}MB)",
            mb
        );
    }

    #[test]
    #[cfg(unix)]
    fn test_get_total_memory_returns_some() {
        let mem = get_total_memory_bytes();
        assert!(mem.is_some(), "Total memory detection should work on Unix");

        let bytes = mem.unwrap();
        assert!(bytes > 0, "Total memory should be > 0");

        // Total should be >= available
        if let Some(avail) = get_available_memory_bytes() {
            assert!(
                bytes >= avail,
                "Total memory ({}) should be >= available ({})",
                bytes,
                avail
            );
        }
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn test_proc_meminfo_parsing() {
        let mem = parse_proc_meminfo_available();
        assert!(mem.is_some(), "/proc/meminfo should be readable on Linux");
        assert!(mem.unwrap() > 0);
    }
}
