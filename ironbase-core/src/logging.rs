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

/// Maximum length for log messages (will be truncated with "...")
/// 1 KB limit to prevent massive log spam
const MAX_LOG_LENGTH: usize = 1024;

/// Internal logging function
#[doc(hidden)]
pub fn log_message(level: LogLevel, module: &str, message: &str) {
    if should_log(level) {
        // Truncate long messages
        let truncated = if message.len() > MAX_LOG_LENGTH {
            format!("{}...", &message[..MAX_LOG_LENGTH])
        } else {
            message.to_string()
        };

        eprintln!(
            "{} [{}] {}: {}",
            level.icon(),
            level.as_str(),
            module,
            truncated
        );
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
    fn test_log_level_parsing_lowercase() {
        assert_eq!(LogLevel::from_str("error"), Some(LogLevel::Error));
        assert_eq!(LogLevel::from_str("warn"), Some(LogLevel::Warn));
        assert_eq!(LogLevel::from_str("info"), Some(LogLevel::Info));
        assert_eq!(LogLevel::from_str("debug"), Some(LogLevel::Debug));
        assert_eq!(LogLevel::from_str("trace"), Some(LogLevel::Trace));
    }

    #[test]
    fn test_log_level_parsing_mixed_case() {
        assert_eq!(LogLevel::from_str("ErRoR"), Some(LogLevel::Error));
        assert_eq!(LogLevel::from_str("wArN"), Some(LogLevel::Warn));
        assert_eq!(LogLevel::from_str("iNfO"), Some(LogLevel::Info));
        assert_eq!(LogLevel::from_str("DeBuG"), Some(LogLevel::Debug));
        assert_eq!(LogLevel::from_str("TrAcE"), Some(LogLevel::Trace));
    }

    #[test]
    fn test_log_level_parsing_invalid() {
        assert_eq!(LogLevel::from_str("invalid"), None);
        assert_eq!(LogLevel::from_str(""), None);
        assert_eq!(LogLevel::from_str("ERR"), None);
        assert_eq!(LogLevel::from_str("WARNING"), None);
        assert_eq!(LogLevel::from_str("INFORMATION"), None);
        assert_eq!(LogLevel::from_str("DBG"), None);
        assert_eq!(LogLevel::from_str("TRC"), None);
        assert_eq!(LogLevel::from_str(" ERROR"), None);
        assert_eq!(LogLevel::from_str("ERROR "), None);
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
    fn test_log_level_icon() {
        // Test that each level has a unique icon
        let error_icon = LogLevel::Error.icon();
        let warn_icon = LogLevel::Warn.icon();
        let info_icon = LogLevel::Info.icon();
        let debug_icon = LogLevel::Debug.icon();
        let trace_icon = LogLevel::Trace.icon();

        // All icons should be non-empty
        assert!(!error_icon.is_empty());
        assert!(!warn_icon.is_empty());
        assert!(!info_icon.is_empty());
        assert!(!debug_icon.is_empty());
        assert!(!trace_icon.is_empty());

        // Icons should be different from each other
        assert_ne!(error_icon, warn_icon);
        assert_ne!(warn_icon, info_icon);
        assert_ne!(info_icon, debug_icon);
        assert_ne!(debug_icon, trace_icon);
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
    fn test_log_level_filtering_error_only() {
        set_log_level(LogLevel::Error);
        assert!(should_log(LogLevel::Error));
        assert!(!should_log(LogLevel::Warn));
        assert!(!should_log(LogLevel::Info));
        assert!(!should_log(LogLevel::Debug));
        assert!(!should_log(LogLevel::Trace));
    }

    #[test]
    fn test_log_level_filtering_trace_all() {
        set_log_level(LogLevel::Trace);
        assert!(should_log(LogLevel::Error));
        assert!(should_log(LogLevel::Warn));
        assert!(should_log(LogLevel::Info));
        assert!(should_log(LogLevel::Debug));
        assert!(should_log(LogLevel::Trace));
    }

    #[test]
    fn test_set_get_log_level() {
        set_log_level(LogLevel::Debug);
        assert_eq!(get_log_level(), LogLevel::Debug);

        set_log_level(LogLevel::Error);
        assert_eq!(get_log_level(), LogLevel::Error);
    }

    #[test]
    fn test_set_get_log_level_all() {
        for level in [
            LogLevel::Error,
            LogLevel::Warn,
            LogLevel::Info,
            LogLevel::Debug,
            LogLevel::Trace,
        ] {
            set_log_level(level);
            assert_eq!(get_log_level(), level);
        }
    }

    #[test]
    fn test_log_level_clone() {
        let level = LogLevel::Debug;
        let cloned = level.clone();
        assert_eq!(level, cloned);
    }

    #[test]
    fn test_log_level_copy() {
        let level = LogLevel::Info;
        let copied = level;
        assert_eq!(level, copied);
    }

    #[test]
    fn test_log_level_debug_format() {
        // Test Debug trait implementation
        let debug_str = format!("{:?}", LogLevel::Error);
        assert!(debug_str.contains("Error"));

        let debug_str = format!("{:?}", LogLevel::Warn);
        assert!(debug_str.contains("Warn"));

        let debug_str = format!("{:?}", LogLevel::Info);
        assert!(debug_str.contains("Info"));

        let debug_str = format!("{:?}", LogLevel::Debug);
        assert!(debug_str.contains("Debug"));

        let debug_str = format!("{:?}", LogLevel::Trace);
        assert!(debug_str.contains("Trace"));
    }

    #[test]
    fn test_log_level_repr() {
        // Test that repr(u8) values are correct
        assert_eq!(LogLevel::Error as u8, 0);
        assert_eq!(LogLevel::Warn as u8, 1);
        assert_eq!(LogLevel::Info as u8, 2);
        assert_eq!(LogLevel::Debug as u8, 3);
        assert_eq!(LogLevel::Trace as u8, 4);
    }

    #[test]
    fn test_log_level_ord_comparison() {
        // Test various ordering comparisons
        assert!(LogLevel::Error <= LogLevel::Error);
        assert!(LogLevel::Error <= LogLevel::Warn);
        assert!(LogLevel::Warn >= LogLevel::Error);
        assert!(LogLevel::Trace >= LogLevel::Error);
    }

    #[test]
    fn test_should_log_at_boundary() {
        set_log_level(LogLevel::Warn);

        // At boundary: should log
        assert!(should_log(LogLevel::Warn));

        // Below boundary: should log
        assert!(should_log(LogLevel::Error));

        // Above boundary: should not log
        assert!(!should_log(LogLevel::Info));
    }

    #[test]
    fn test_max_log_length_constant() {
        // Verify the constant exists and has a reasonable value
        assert_eq!(MAX_LOG_LENGTH, 1024);
    }

    #[test]
    fn test_log_message_short() {
        // Short message should be logged without truncation
        set_log_level(LogLevel::Trace);
        // This should not panic
        log_message(LogLevel::Info, "test_module", "Short message");
    }

    #[test]
    fn test_log_message_at_limit() {
        set_log_level(LogLevel::Trace);
        let message = "x".repeat(MAX_LOG_LENGTH);
        // This should not panic
        log_message(LogLevel::Info, "test_module", &message);
    }

    #[test]
    fn test_log_message_over_limit() {
        set_log_level(LogLevel::Trace);
        let message = "x".repeat(MAX_LOG_LENGTH + 100);
        // This should not panic and should truncate
        log_message(LogLevel::Info, "test_module", &message);
    }

    #[test]
    fn test_log_message_empty() {
        set_log_level(LogLevel::Trace);
        // Empty message should be handled gracefully
        log_message(LogLevel::Info, "test_module", "");
    }

    #[test]
    fn test_log_message_unicode() {
        set_log_level(LogLevel::Trace);
        // Unicode message should be handled gracefully
        log_message(LogLevel::Info, "test_module", "Hello, világ! 你好世界 🎉");
    }

    #[test]
    fn test_log_message_filtered_out() {
        set_log_level(LogLevel::Error);
        // Debug message should be filtered out (no panic)
        log_message(LogLevel::Debug, "test_module", "This should be filtered");
    }

    #[test]
    fn test_log_message_multiline() {
        set_log_level(LogLevel::Trace);
        // Multiline message should be handled
        log_message(LogLevel::Info, "test_module", "Line 1\nLine 2\nLine 3");
    }

    #[test]
    fn test_log_message_special_chars() {
        set_log_level(LogLevel::Trace);
        // Special characters should be handled
        log_message(
            LogLevel::Info,
            "test_module",
            "Tab\there, quote\"here, backslash\\here",
        );
    }

    #[test]
    fn test_from_str_roundtrip() {
        // Test that as_str and from_str are inverses
        for level in [
            LogLevel::Error,
            LogLevel::Warn,
            LogLevel::Info,
            LogLevel::Debug,
            LogLevel::Trace,
        ] {
            let as_str = level.as_str();
            let parsed = LogLevel::from_str(as_str);
            assert_eq!(parsed, Some(level));
        }
    }
}
