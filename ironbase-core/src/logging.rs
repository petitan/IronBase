// logging.rs - Logging system using tracing
// Provides log_error!, log_warn!, log_info!, log_debug!, log_trace! macros
// that forward to tracing for unified logging with MCP server

use std::sync::atomic::{AtomicU8, Ordering};

/// Log levels (ordered by severity)
/// Kept for backwards compatibility - actual filtering is done by tracing subscriber
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum LogLevel {
    /// Errors - critical failures that prevent operations
    Error = 0,
    /// Warnings - potential issues that don't stop execution
    Warn = 1,
    /// Info - high-level operational information
    Info = 2,
    /// Debug - detailed diagnostic information
    Debug = 3,
    /// Trace - extremely verbose, every operation logged
    Trace = 4,
}

impl LogLevel {
    /// Parse log level from string (case-insensitive)
    pub fn from_str(s: &str) -> Option<LogLevel> {
        match s.to_uppercase().as_str() {
            "ERROR" => Some(LogLevel::Error),
            "WARN" => Some(LogLevel::Warn),
            "INFO" => Some(LogLevel::Info),
            "DEBUG" => Some(LogLevel::Debug),
            "TRACE" => Some(LogLevel::Trace),
            _ => None,
        }
    }

    /// Convert to string
    pub fn as_str(&self) -> &'static str {
        match self {
            LogLevel::Error => "ERROR",
            LogLevel::Warn => "WARN",
            LogLevel::Info => "INFO",
            LogLevel::Debug => "DEBUG",
            LogLevel::Trace => "TRACE",
        }
    }
}

// Global log level (kept for backwards compatibility)
static GLOBAL_LOG_LEVEL: AtomicU8 = AtomicU8::new(LogLevel::Warn as u8);

/// Set the global log level
/// Note: Actual filtering is done by tracing subscriber (RUST_LOG env var)
/// This is kept for backwards compatibility
pub fn set_log_level(level: LogLevel) {
    GLOBAL_LOG_LEVEL.store(level as u8, Ordering::Relaxed);
}

/// Get the current global log level
pub fn get_log_level() -> LogLevel {
    let level = GLOBAL_LOG_LEVEL.load(Ordering::Relaxed);
    match level {
        0 => LogLevel::Error,
        1 => LogLevel::Warn,
        2 => LogLevel::Info,
        3 => LogLevel::Debug,
        4 => LogLevel::Trace,
        _ => LogLevel::Warn, // Fallback
    }
}

/// Check if a message at the given level should be logged
/// Note: This checks the internal level, but tracing subscriber does actual filtering
/// Kept for backwards compatibility - actual filtering is done by tracing subscriber
#[inline]
#[allow(dead_code)]
pub fn should_log(level: LogLevel) -> bool {
    level <= get_log_level()
}

/// Log an error message
#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => {
        tracing::error!($($arg)*)
    };
}

/// Log a warning message
#[macro_export]
macro_rules! log_warn {
    ($($arg:tt)*) => {
        tracing::warn!($($arg)*)
    };
}

/// Log an info message
#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => {
        tracing::info!($($arg)*)
    };
}

/// Log a debug message
#[macro_export]
macro_rules! log_debug {
    ($($arg:tt)*) => {
        tracing::debug!($($arg)*)
    };
}

/// Log a trace message
#[macro_export]
macro_rules! log_trace {
    ($($arg:tt)*) => {
        tracing::trace!($($arg)*)
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_level_ordering() {
        assert!(LogLevel::Error < LogLevel::Warn);
        assert!(LogLevel::Warn < LogLevel::Info);
        assert!(LogLevel::Info < LogLevel::Debug);
        assert!(LogLevel::Debug < LogLevel::Trace);
    }

    #[test]
    fn test_log_level_equality() {
        assert_eq!(LogLevel::Error, LogLevel::Error);
        assert_eq!(LogLevel::Warn, LogLevel::Warn);
        assert_eq!(LogLevel::Info, LogLevel::Info);
        assert_eq!(LogLevel::Debug, LogLevel::Debug);
        assert_eq!(LogLevel::Trace, LogLevel::Trace);
    }

    #[test]
    fn test_log_level_parsing() {
        assert_eq!(LogLevel::from_str("ERROR"), Some(LogLevel::Error));
        assert_eq!(LogLevel::from_str("warn"), Some(LogLevel::Warn));
        assert_eq!(LogLevel::from_str("DeBuG"), Some(LogLevel::Debug));
        assert_eq!(LogLevel::from_str("invalid"), None);
    }

    #[test]
    fn test_log_level_parsing_all_levels() {
        assert_eq!(LogLevel::from_str("ERROR"), Some(LogLevel::Error));
        assert_eq!(LogLevel::from_str("WARN"), Some(LogLevel::Warn));
        assert_eq!(LogLevel::from_str("INFO"), Some(LogLevel::Info));
        assert_eq!(LogLevel::from_str("DEBUG"), Some(LogLevel::Debug));
        assert_eq!(LogLevel::from_str("TRACE"), Some(LogLevel::Trace));
    }

    #[test]
    fn test_log_level_as_str() {
        assert_eq!(LogLevel::Error.as_str(), "ERROR");
        assert_eq!(LogLevel::Warn.as_str(), "WARN");
        assert_eq!(LogLevel::Info.as_str(), "INFO");
        assert_eq!(LogLevel::Debug.as_str(), "DEBUG");
        assert_eq!(LogLevel::Trace.as_str(), "TRACE");
    }

    #[test]
    fn test_log_level_repr() {
        assert_eq!(LogLevel::Error as u8, 0);
        assert_eq!(LogLevel::Warn as u8, 1);
        assert_eq!(LogLevel::Info as u8, 2);
        assert_eq!(LogLevel::Debug as u8, 3);
        assert_eq!(LogLevel::Trace as u8, 4);
    }

    #[test]
    fn test_set_get_log_level() {
        set_log_level(LogLevel::Debug);
        assert_eq!(get_log_level(), LogLevel::Debug);

        set_log_level(LogLevel::Error);
        assert_eq!(get_log_level(), LogLevel::Error);
    }

    #[test]
    fn test_log_level_filtering() {
        set_log_level(LogLevel::Info);
        assert!(should_log(LogLevel::Error));
        assert!(should_log(LogLevel::Warn));
        assert!(should_log(LogLevel::Info));
        assert!(!should_log(LogLevel::Debug));
        assert!(!should_log(LogLevel::Trace));
    }
}
