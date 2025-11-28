//! Logging FFI functions
//!
//! Global logging configuration for IronBase

use std::os::raw::c_char;

use crate::error::{
    c_str_to_string, clear_last_error, set_last_error, string_to_c_str, IronBaseErrorCode,
};

/// Set the global log level
///
/// # Parameters
/// - `level`: Log level string (one of: "ERROR", "WARN", "INFO", "DEBUG", "TRACE")
///
/// # Returns
/// - `IronBaseErrorCode::Success` (0) on success
/// - Error code on failure
///
/// # Log Levels (from least to most verbose)
/// - ERROR: Only critical errors
/// - WARN: Warnings and errors (default)
/// - INFO: Informational messages
/// - DEBUG: Debug information
/// - TRACE: Very verbose tracing
///
/// # Example
/// ```c
/// ironbase_set_log_level("DEBUG");  // Enable debug logging
/// ironbase_set_log_level("WARN");   // Default level
/// ```
#[no_mangle]
pub extern "C" fn ironbase_set_log_level(level: *const c_char) -> i32 {
    clear_last_error();

    let level_str = match c_str_to_string(level) {
        Some(s) => s,
        None => {
            set_last_error("Level is null or invalid UTF-8");
            return IronBaseErrorCode::NullPointer as i32;
        }
    };

    let log_level = match ironbase_core::LogLevel::from_str(&level_str) {
        Some(l) => l,
        None => {
            set_last_error(&format!(
                "Invalid log level '{}'. Must be one of: ERROR, WARN, INFO, DEBUG, TRACE",
                level_str
            ));
            return IronBaseErrorCode::InvalidQuery as i32;
        }
    };

    ironbase_core::set_log_level(log_level);
    IronBaseErrorCode::Success as i32
}

/// Get the current global log level
///
/// # Returns
/// - Log level string (caller must free with `ironbase_free_string()`)
/// - Null on error
#[no_mangle]
pub extern "C" fn ironbase_get_log_level() -> *mut c_char {
    clear_last_error();

    let level = ironbase_core::get_log_level();
    string_to_c_str(level.as_str())
}
