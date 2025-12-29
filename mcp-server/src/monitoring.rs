//! Memory monitoring and system health checks for IronBase MCP Server
//!
//! Provides jemalloc memory statistics on Unix platforms and system health endpoints.

use serde::Serialize;

/// Memory statistics from jemalloc (Unix only)
#[derive(Debug, Clone, Serialize, Default)]
pub struct MemoryStats {
    /// Allocated memory in bytes
    pub allocated_bytes: u64,
    /// Resident memory in bytes
    pub resident_bytes: u64,
    /// Allocated memory in MB (for easier reading)
    pub allocated_mb: f64,
    /// Resident memory in MB (for easier reading)
    pub resident_mb: f64,
    /// Whether jemalloc stats are available
    pub available: bool,
}

/// Get current memory statistics from jemalloc
///
/// On Unix platforms, returns accurate jemalloc statistics.
/// On Windows or if jemalloc is not available, returns default (unavailable) stats.
#[cfg(all(unix, not(target_env = "msvc")))]
pub fn get_memory_stats() -> MemoryStats {
    use tikv_jemalloc_ctl::{epoch, stats};

    // Advance the epoch to get fresh statistics
    if epoch::advance().is_err() {
        return MemoryStats::default();
    }

    let allocated = stats::allocated::read().unwrap_or(0);
    let resident = stats::resident::read().unwrap_or(0);

    MemoryStats {
        allocated_bytes: allocated as u64,
        resident_bytes: resident as u64,
        allocated_mb: allocated as f64 / 1024.0 / 1024.0,
        resident_mb: resident as f64 / 1024.0 / 1024.0,
        available: true,
    }
}

/// Get current memory statistics (Windows/non-jemalloc fallback)
#[cfg(not(all(unix, not(target_env = "msvc"))))]
pub fn get_memory_stats() -> MemoryStats {
    MemoryStats::default()
}

/// Log current memory statistics at INFO level
///
/// This can be called periodically to track memory usage over time.
pub fn log_memory_stats() {
    let stats = get_memory_stats();
    if stats.available {
        tracing::info!(
            allocated_mb = format!("{:.2}", stats.allocated_mb),
            resident_mb = format!("{:.2}", stats.resident_mb),
            "Memory stats"
        );
    }
}

/// Check if memory usage is above a warning threshold
///
/// Returns true if allocated memory exceeds the given threshold in MB.
pub fn is_memory_pressure(threshold_mb: f64) -> bool {
    let stats = get_memory_stats();
    if stats.available {
        stats.allocated_mb > threshold_mb
    } else {
        false // Can't determine on Windows
    }
}

/// Health check result
#[derive(Debug, Clone, Serialize)]
pub struct HealthCheck {
    /// Server status
    pub status: &'static str,
    /// Server version
    pub version: &'static str,
    /// Memory statistics
    pub memory: MemoryStats,
    /// Unix timestamp of check
    pub timestamp: i64,
}

/// Perform a health check
pub fn health_check() -> HealthCheck {
    HealthCheck {
        status: "ok",
        version: crate::VERSION,
        memory: get_memory_stats(),
        timestamp: chrono::Utc::now().timestamp(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_stats() {
        let stats = get_memory_stats();
        // On Unix, should be available
        #[cfg(all(unix, not(target_env = "msvc")))]
        assert!(stats.available);
        // On Windows, should not be available
        #[cfg(not(all(unix, not(target_env = "msvc"))))]
        assert!(!stats.available);
    }

    #[test]
    fn test_health_check() {
        let health = health_check();
        assert_eq!(health.status, "ok");
        assert!(!health.version.is_empty());
    }
}
