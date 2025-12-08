//! Terminal color output utilities
//!
//! Provides ANSI color codes with automatic TTY detection.
//! Colors are disabled when output is not a terminal.

use std::io::IsTerminal;

/// Check if stdout is a TTY
pub fn is_tty() -> bool {
    std::io::stdout().is_terminal()
}

/// Green text (for success)
pub fn green(s: &str) -> String {
    if is_tty() {
        format!("\x1b[32m{}\x1b[0m", s)
    } else {
        s.to_string()
    }
}

/// Red text (for errors/failures)
pub fn red(s: &str) -> String {
    if is_tty() {
        format!("\x1b[31m{}\x1b[0m", s)
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_color_functions_return_string() {
        // These just verify the functions return something
        // Can't test actual colors in unit tests
        assert!(green("OK").contains("OK"));
        assert!(red("FAIL").contains("FAIL"));
    }
}
