//! Theme system for IronBase TUI
//!
//! Provides 4 color themes including Claude-inspired warm colors.

use ratatui::style::Color;
use serde::{Deserialize, Serialize};

/// Available themes
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ThemeName {
    #[default]
    Claude,
    Dark,
    Light,
    Ocean,
}

impl ThemeName {
    pub fn next(self) -> Self {
        match self {
            ThemeName::Claude => ThemeName::Dark,
            ThemeName::Dark => ThemeName::Light,
            ThemeName::Light => ThemeName::Ocean,
            ThemeName::Ocean => ThemeName::Claude,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            ThemeName::Claude => "Claude",
            ThemeName::Dark => "Dark",
            ThemeName::Light => "Light",
            ThemeName::Ocean => "Ocean",
        }
    }
}

/// Color theme configuration
#[derive(Debug, Clone)]
pub struct Theme {
    /// Background color
    pub bg: Color,
    /// Foreground (text) color
    pub fg: Color,
    /// Primary accent color (highlights, selected items)
    pub accent: Color,
    /// Secondary accent color (borders, less important elements)
    pub secondary: Color,
    /// Success color (green)
    pub success: Color,
    /// Warning color (yellow/orange)
    pub warning: Color,
    /// Error color (red)
    pub error: Color,
    /// Muted text color (hints, disabled)
    pub muted: Color,
    /// Selection background
    pub selection_bg: Color,
    /// Header background
    pub header_bg: Color,
}

impl Theme {
    /// Claude-inspired warm theme (default)
    /// Based on claude.ai color palette: warm orange/brown tones
    pub fn claude() -> Self {
        Self {
            bg: Color::Rgb(25, 23, 22),           // Dark warm gray
            fg: Color::Rgb(236, 230, 223),        // Warm white
            accent: Color::Rgb(217, 119, 87),     // Claude orange
            secondary: Color::Rgb(140, 120, 100), // Warm gray
            success: Color::Rgb(134, 179, 134),   // Soft green
            warning: Color::Rgb(217, 168, 87),    // Warm yellow
            error: Color::Rgb(204, 102, 102),     // Soft red
            muted: Color::Rgb(120, 110, 100),     // Muted warm gray
            selection_bg: Color::Rgb(60, 50, 45), // Selection
            header_bg: Color::Rgb(45, 40, 38),    // Header
        }
    }

    /// Dark theme
    pub fn dark() -> Self {
        Self {
            bg: Color::Rgb(20, 20, 25),
            fg: Color::Rgb(230, 230, 235),
            accent: Color::Rgb(100, 149, 237), // Cornflower blue
            secondary: Color::Rgb(100, 100, 110),
            success: Color::Rgb(80, 200, 120),
            warning: Color::Rgb(255, 193, 7),
            error: Color::Rgb(239, 83, 80),
            muted: Color::Rgb(100, 100, 100),
            selection_bg: Color::Rgb(40, 44, 52),
            header_bg: Color::Rgb(30, 32, 38),
        }
    }

    /// Light theme
    pub fn light() -> Self {
        Self {
            bg: Color::Rgb(250, 250, 250),
            fg: Color::Rgb(30, 30, 30),
            accent: Color::Rgb(0, 102, 204),
            secondary: Color::Rgb(150, 150, 160),
            success: Color::Rgb(40, 167, 69),
            warning: Color::Rgb(255, 152, 0),
            error: Color::Rgb(220, 53, 69),
            muted: Color::Rgb(140, 140, 140),
            selection_bg: Color::Rgb(230, 240, 250),
            header_bg: Color::Rgb(240, 240, 245),
        }
    }

    /// Ocean theme (blue/teal)
    pub fn ocean() -> Self {
        Self {
            bg: Color::Rgb(15, 30, 40),
            fg: Color::Rgb(200, 220, 230),
            accent: Color::Rgb(64, 224, 208), // Turquoise
            secondary: Color::Rgb(70, 100, 120),
            success: Color::Rgb(46, 204, 113),
            warning: Color::Rgb(241, 196, 15),
            error: Color::Rgb(231, 76, 60),
            muted: Color::Rgb(80, 100, 120),
            selection_bg: Color::Rgb(30, 50, 65),
            header_bg: Color::Rgb(20, 40, 55),
        }
    }

    /// Get theme by name
    pub fn from_name(name: ThemeName) -> Self {
        match name {
            ThemeName::Claude => Self::claude(),
            ThemeName::Dark => Self::dark(),
            ThemeName::Light => Self::light(),
            ThemeName::Ocean => Self::ocean(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_theme_cycle() {
        let theme = ThemeName::Claude;
        assert_eq!(theme.next(), ThemeName::Dark);
        assert_eq!(theme.next().next(), ThemeName::Light);
        assert_eq!(theme.next().next().next(), ThemeName::Ocean);
        assert_eq!(theme.next().next().next().next(), ThemeName::Claude);
    }

    #[test]
    fn test_theme_names() {
        assert_eq!(ThemeName::Claude.name(), "Claude");
        assert_eq!(ThemeName::Dark.name(), "Dark");
    }
}
