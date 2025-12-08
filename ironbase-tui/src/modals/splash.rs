//! Splash screen modal - shown during startup

use crate::theme::Theme;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

/// ASCII art logo for IronBase
const LOGO: &str = r#"
  ___                 ____
 |_ _|_ __ ___  _ __ | __ )  __ _ ___  ___
  | || '__/ _ \| '_ \|  _ \ / _` / __|/ _ \
  | || | | (_) | | | | |_) | (_| \__ \  __/
 |___|_|  \___/|_| |_|____/ \__,_|___/\___|
"#;

/// Render the splash screen
pub fn render(frame: &mut Frame, area: Rect, message: &str, spinner_frame: usize, theme: &Theme) {
    // Full screen background
    let block = Block::default().style(Style::default().bg(theme.bg));
    frame.render_widget(block, area);

    // Calculate center position
    let logo_height = 6;
    let content_height = logo_height + 4;
    let vertical_margin = (area.height.saturating_sub(content_height as u16)) / 2;

    // Spinner character
    const SPINNERS: [char; 8] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧'];
    let spinner = SPINNERS[spinner_frame % SPINNERS.len()];

    // Build content
    let mut lines: Vec<Line> = Vec::new();

    // Logo lines
    for line in LOGO.lines().skip(1) {
        lines.push(Line::from(Span::styled(
            line,
            Style::default().fg(theme.accent).bold(),
        )));
    }

    // Empty line
    lines.push(Line::from(""));

    // Status message with spinner
    lines.push(Line::from(vec![
        Span::styled(format!("{} ", spinner), Style::default().fg(theme.accent)),
        Span::styled(message, Style::default().fg(theme.muted)),
    ]));

    // Version
    lines.push(Line::from(Span::styled(
        format!("v{}", env!("CARGO_PKG_VERSION")),
        Style::default().fg(theme.muted),
    )));

    let paragraph = Paragraph::new(lines).alignment(Alignment::Center).block(
        Block::default()
            .borders(Borders::NONE)
            .style(Style::default().bg(theme.bg)),
    );

    // Create centered area
    let centered_area = Rect {
        x: 0,
        y: vertical_margin,
        width: area.width,
        height: content_height as u16,
    };

    frame.render_widget(paragraph, centered_area);
}
