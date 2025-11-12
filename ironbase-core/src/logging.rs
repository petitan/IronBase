// logging.rs - Simple, embedded-friendly logging system
// Designed for IronBase (no external dependencies like env_logger)

use std::sync::atomic::{AtomicU8, Ordering};

/// Log levels (ordered by severity)
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

    /// Get emoji/icon for log level
    fn icon(&self) -> &'static str {
        match self {
            LogLevel::Error => "❌",
            LogLevel::Warn => "⚠️",
            LogLevel::Info => "ℹ️",
            LogLevel::Debug => "🔍",
            LogLevel::Trace => "📝",
        }
    }
}

// Global log level (default: WARN for production)
static GLOBAL_LOG_LEVEL: AtomicU8 = AtomicU8::new(LogLevel::Warn as u8);

/// Set the global log level
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
#[inline]
pub fn should_log(level: LogLevel) -> bool {
    level <= get_log_level()
}

/// Internal logging function
#[doc(hidden)]
pub fn log_message(level: LogLevel, module: &str, message: &str) {
    if should_log(level) {
        eprintln!("{} [{}] {}: {}", level.icon(), level.as_str(), module, message);
    }
}

/// Log an error message (always shown unless level = OFF)
#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => {
        $crate::logging::log_message(
            $crate::logging::LogLevel::Error,
            module_path!(),
            &format!($($arg)*)
        )
    };
}

/// Log a warning message
#[macro_export]
macro_rules! log_warn {
    ($($arg:tt)*) => {
        $crate::logging::log_message(
            $crate::logging::LogLevel::Warn,
            module_path!(),
            &format!($($arg)*)
        )
    };
}

/// Log an info message
#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => {
        $crate::logging::log_message(
            $crate::logging::LogLevel::Info,
            module_path!(),
            &format!($($arg)*)
        )
    };
}

/// Log a debug message
#[macro_export]
macro_rules! log_debug {
    ($($arg:tt)*) => {
        $crate::logging::log_message(
            $crate::logging::LogLevel::Debug,
            module_path!(),
            &format!($($arg)*)
        )
    };
}

/// Log a trace message
#[macro_export]
macro_rules! log_trace {
    ($($arg:tt)*) => {
        $crate::logging::log_message(
            $crate::logging::LogLevel::Trace,
            module_path!(),
            &format!($($arg)*)
        )
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
    fn test_log_level_parsing() {
        assert_eq!(LogLevel::from_str("ERROR"), Some(LogLevel::Error));
        assert_eq!(LogLevel::from_str("warn"), Some(LogLevel::Warn));
        assert_eq!(LogLevel::from_str("DeBuG"), Some(LogLevel::Debug));
        assert_eq!(LogLevel::from_str("invalid"), None);
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

    #[test]
    fn test_set_get_log_level() {
        set_log_level(LogLevel::Debug);
        assert_eq!(get_log_level(), LogLevel::Debug);

        set_log_level(LogLevel::Error);
        assert_eq!(get_log_level(), LogLevel::Error);
    }
}
