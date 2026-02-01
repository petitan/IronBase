//! Working set lock for Windows - prevents memory paging under pressure
//!
//! Uses `SetProcessWorkingSetSizeEx` with `QUOTA_LIMITS_HARDWS_MIN_ENABLE`
//! to set a hard minimum working set floor. Windows will NOT trim below this.
//!
//! On non-Windows platforms, all functions are no-ops.

/// Result of a working set lock attempt
#[derive(Debug, Clone)]
pub struct WorkingSetLockResult {
    /// Whether the lock was successfully applied
    pub success: bool,
    /// Minimum working set size in bytes (the hard floor)
    pub min_bytes: u64,
    /// Maximum working set size in bytes
    pub max_bytes: u64,
    /// Human-readable message
    pub message: String,
}

/// Current working set limits as read back from the OS via GetProcessWorkingSetSizeEx.
/// Used by /health endpoint to verify the lock is actually applied.
#[derive(Debug, Clone, serde::Serialize)]
pub struct WorkingSetLimits {
    /// Minimum working set size in MB
    pub min_mb: u64,
    /// Maximum working set size in MB
    pub max_mb: u64,
    /// Whether hard minimum is enabled (QUOTA_LIMITS_HARDWS_MIN_ENABLE)
    pub hard_min_enabled: bool,
    /// Whether hard maximum is enabled (QUOTA_LIMITS_HARDWS_MAX_ENABLE)
    pub hard_max_enabled: bool,
    /// Raw flags value from GetProcessWorkingSetSizeEx
    pub flags: u32,
}

// =============================================================================
// Windows implementation
// =============================================================================

#[cfg(windows)]
mod platform {
    use super::WorkingSetLockResult;

    // SetProcessWorkingSetSizeEx flags
    const QUOTA_LIMITS_HARDWS_MIN_ENABLE: u32 = 0x00000001;
    const QUOTA_LIMITS_HARDWS_MAX_ENABLE: u32 = 0x00000004;
    const QUOTA_LIMITS_HARDWS_MAX_DISABLE: u32 = 0x00000008;

    /// Lock the current process working set to prevent paging.
    ///
    /// Measures the current working set size and sets it as the hard minimum.
    /// Windows will not trim below this value even under memory pressure.
    ///
    /// - `QUOTA_LIMITS_HARDWS_MIN_ENABLE`: hard floor, Windows won't trim below
    /// - `QUOTA_LIMITS_HARDWS_MAX_DISABLE`: no upper cap (process can grow)
    ///
    /// LocalSystem service accounts have `SeIncreaseQuotaPrivilege` by default,
    /// so no privilege elevation is needed.
    ///
    /// If the call fails, returns a result with `success: false` and a warning message.
    /// The server continues running normally (graceful fallback).
    pub fn lock_working_set() -> WorkingSetLockResult {
        use windows_sys::Win32::System::ProcessStatus::{
            GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
        };
        use windows_sys::Win32::System::Threading::GetCurrentProcess;

        let process = unsafe { GetCurrentProcess() };

        // Step 1: Get current working set size
        let mut pmc: PROCESS_MEMORY_COUNTERS = unsafe { std::mem::zeroed() };
        pmc.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;

        let ok = unsafe {
            GetProcessMemoryInfo(process, &mut pmc as *mut PROCESS_MEMORY_COUNTERS, pmc.cb)
        };

        if ok == 0 {
            let err = std::io::Error::last_os_error();
            return WorkingSetLockResult {
                success: false,
                min_bytes: 0,
                max_bytes: 0,
                message: format!("GetProcessMemoryInfo failed: {}", err),
            };
        }

        let current_ws = pmc.WorkingSetSize as u64;
        let min_ws = current_ws;
        let max_ws = current_ws * 2;

        // Step 2: Lock the working set with hard minimum
        let flags = QUOTA_LIMITS_HARDWS_MIN_ENABLE | QUOTA_LIMITS_HARDWS_MAX_DISABLE;

        // SetProcessWorkingSetSizeEx is in Win32_System_Memory
        let result = unsafe {
            windows_sys::Win32::System::Memory::SetProcessWorkingSetSizeEx(
                process,
                min_ws as usize,
                max_ws as usize,
                flags,
            )
        };

        if result == 0 {
            let err = std::io::Error::last_os_error();
            WorkingSetLockResult {
                success: false,
                min_bytes: min_ws,
                max_bytes: max_ws,
                message: format!(
                    "SetProcessWorkingSetSizeEx failed (min={} MB, max={} MB): {}",
                    min_ws / (1024 * 1024),
                    max_ws / (1024 * 1024),
                    err
                ),
            }
        } else {
            WorkingSetLockResult {
                success: true,
                min_bytes: min_ws,
                max_bytes: max_ws,
                message: format!(
                    "Working set locked: min={} MB, max={} MB (hard floor enabled)",
                    min_ws / (1024 * 1024),
                    max_ws / (1024 * 1024),
                ),
            }
        }
    }

    /// Get current working set size in bytes (Windows).
    pub fn get_working_set_bytes() -> Option<u64> {
        use windows_sys::Win32::System::ProcessStatus::{
            GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
        };
        use windows_sys::Win32::System::Threading::GetCurrentProcess;

        let process = unsafe { GetCurrentProcess() };
        let mut pmc: PROCESS_MEMORY_COUNTERS = unsafe { std::mem::zeroed() };
        pmc.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;

        let ok = unsafe {
            GetProcessMemoryInfo(process, &mut pmc as *mut PROCESS_MEMORY_COUNTERS, pmc.cb)
        };

        if ok != 0 {
            Some(pmc.WorkingSetSize as u64)
        } else {
            None
        }
    }

    /// Read back the current working set limits from the OS.
    ///
    /// Returns the min/max sizes and whether hard limits are enabled.
    /// This verifies that `SetProcessWorkingSetSizeEx` actually applied the lock.
    pub fn get_working_set_limits() -> Option<super::WorkingSetLimits> {
        use windows_sys::Win32::System::Threading::GetCurrentProcess;

        let process = unsafe { GetCurrentProcess() };
        let mut min_ws: usize = 0;
        let mut max_ws: usize = 0;
        let mut flags: u32 = 0;

        let ok = unsafe {
            windows_sys::Win32::System::Memory::GetProcessWorkingSetSizeEx(
                process,
                &mut min_ws,
                &mut max_ws,
                &mut flags,
            )
        };

        if ok != 0 {
            Some(super::WorkingSetLimits {
                min_mb: (min_ws as u64) / (1024 * 1024),
                max_mb: (max_ws as u64) / (1024 * 1024),
                hard_min_enabled: (flags & QUOTA_LIMITS_HARDWS_MIN_ENABLE) != 0,
                hard_max_enabled: (flags & QUOTA_LIMITS_HARDWS_MAX_ENABLE) != 0,
                flags,
            })
        } else {
            None
        }
    }
}

// =============================================================================
// Non-Windows no-op implementation
// =============================================================================

#[cfg(not(windows))]
mod platform {
    use super::WorkingSetLockResult;

    /// No-op on non-Windows platforms.
    pub fn lock_working_set() -> WorkingSetLockResult {
        WorkingSetLockResult {
            success: false,
            min_bytes: 0,
            max_bytes: 0,
            message: "Working set lock is only available on Windows".to_string(),
        }
    }

    /// No-op on non-Windows platforms.
    pub fn get_working_set_bytes() -> Option<u64> {
        None
    }

    /// No-op on non-Windows platforms.
    pub fn get_working_set_limits() -> Option<super::WorkingSetLimits> {
        None
    }
}

// Re-export platform functions
pub use platform::{get_working_set_bytes, get_working_set_limits, lock_working_set};
