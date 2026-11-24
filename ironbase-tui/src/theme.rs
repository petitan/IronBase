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
            secondary: Color::Rgb(146, 127, 108), // Warm gray
            success: Color::Rgb(134, 179, 134),   // Soft green
            warning: Color::Rgb(217, 168, 87),    // Warm yellow
            error: Color::Rgb(204, 102, 102),     // Soft red
            muted: Color::Rgb(138, 129, 120),     // Muted warm gray
            header_bg: Color::Rgb(45, 40, 38),    // Header
        }
    }

    /// Dark theme
    pub fn dark() -> Self {
        Self {
            bg: Color::Rgb(20, 20, 25),
            fg: Color::Rgb(230, 230, 235),
            accent: Color::Rgb(100, 149, 237), // Cornflower blue
            secondary: Color::Rgb(128, 128, 136),
            success: Color::Rgb(80, 200, 120),
            warning: Color::Rgb(255, 193, 7),
            error: Color::Rgb(239, 83, 80),
            muted: Color::Rgb(128, 128, 128),
            header_bg: Color::Rgb(30, 32, 38),
        }
    }

    /// Light theme
    pub fn light() -> Self {
        Self {
            bg: Color::Rgb(250, 250, 250),
            fg: Color::Rgb(30, 30, 30),
            accent: Color::Rgb(0, 102, 204),
            secondary: Color::Rgb(112, 112, 120),
            success: Color::Rgb(31, 130, 54),
            warning: Color::Rgb(163, 97, 0),
            error: Color::Rgb(211, 51, 66),
            muted: Color::Rgb(113, 113, 113),
            header_bg: Color::Rgb(240, 240, 245),
        }
    }

    /// Ocean theme (blue/teal)
    pub fn ocean() -> Self {
        Self {
            bg: Color::Rgb(15, 30, 40),
            fg: Color::Rgb(200, 220, 230),
            accent: Color::Rgb(64, 224, 208), // Turquoise
            secondary: Color::Rgb(114, 137, 152),
            success: Color::Rgb(46, 204, 113),
            warning: Color::Rgb(241, 196, 15),
            error: Color::Rgb(232, 83, 68),
            muted: Color::Rgb(120, 136, 151),
            header_bg: Color::Rgb(20, 40, 55),
        }
    }

    /// No colors at all: every color is the terminal's own (`Color::Reset`),
    /// for `NO_COLOR` (https://no-color.org); the selection still shows
    /// through its mark (`widgets::selection_mark`, `widgets::choice`)
    pub fn monochrome() -> Self {
        Self {
            bg: Color::Reset,
            fg: Color::Reset,
            accent: Color::Reset,
            secondary: Color::Reset,
            success: Color::Reset,
            warning: Color::Reset,
            error: Color::Reset,
            muted: Color::Reset,
            header_bg: Color::Reset,
        }
    }

    /// The theme to draw with: `name`, or no colors when `NO_COLOR` is set
    /// to a non-empty value
    pub fn for_terminal(name: ThemeName) -> Self {
        let no_color = std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
        Self::resolve(name, no_color)
    }

    fn resolve(name: ThemeName, no_color: bool) -> Self {
        if no_color {
            Self::monochrome()
        } else {
            Self::from_name(name)
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

    /// WCAG relative luminance of an RGB color
    fn luminance(color: Color) -> f64 {
        let Color::Rgb(r, g, b) = color else {
            panic!("theme colors are RGB: {color:?}")
        };
        let channel = |v: u8| {
            let v = f64::from(v) / 255.0;
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
    }

    fn contrast(a: Color, b: Color) -> f64 {
        let (la, lb) = (luminance(a), luminance(b));
        (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
    }

    /// Every text color reads against the background at WCAG AA (4.5:1),
    /// hints and borders (`muted`, `secondary`) included, and so does a
    /// selected row (background-colored text on the accent)
    #[test]
    fn test_text_colors_have_aa_contrast() {
        for name in [
            ThemeName::Claude,
            ThemeName::Dark,
            ThemeName::Light,
            ThemeName::Ocean,
        ] {
            let theme = Theme::from_name(name);
            for (role, color) in [
                ("fg", theme.fg),
                ("accent", theme.accent),
                ("secondary", theme.secondary),
                ("success", theme.success),
                ("warning", theme.warning),
                ("error", theme.error),
                ("muted", theme.muted),
            ] {
                let ratio = contrast(color, theme.bg);
                assert!(ratio >= 4.5, "{name:?} {role}: {ratio:.2}");
            }
            assert!(
                contrast(theme.fg, theme.header_bg) >= 4.5,
                "{name:?} header"
            );
        }
    }

    /// `NO_COLOR` draws without colors, whatever the theme
    #[test]
    fn test_no_color_draws_without_colors() {
        let theme = Theme::resolve(ThemeName::Ocean, true);
        for color in [theme.bg, theme.fg, theme.accent, theme.muted, theme.error] {
            assert_eq!(color, Color::Reset);
        }
        assert_eq!(
            Theme::resolve(ThemeName::Ocean, false).bg,
            Theme::ocean().bg
        );
    }

    #[test]
    fn test_theme_names() {
        assert_eq!(ThemeName::Claude.name(), "Claude");
        assert_eq!(ThemeName::Dark.name(), "Dark");
    }
}
