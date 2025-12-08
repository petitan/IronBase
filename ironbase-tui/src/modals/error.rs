//! Error detail modal - shows full error message with scrolling

use super::render_modal_frame;
use crate::theme::Theme;
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

/// Render the error detail modal
pub fn render(frame: &mut Frame, area: Rect, error_message: &str, scroll: usize, theme: &Theme) {
    let inner = render_modal_frame(frame, area, "Hiba reszletei", theme, 80, 60);

    // Split error message into lines for word-wrap
    let max_width = inner.width.saturating_sub(2) as usize;
    let lines = wrap_text(error_message, max_width);

    let total_lines = lines.len();
    let visible_lines = inner.height.saturating_sub(2) as usize; // -2 for hint line

    // Clamp scroll
    let max_scroll = total_lines.saturating_sub(visible_lines);
    let current_scroll = scroll.min(max_scroll);

    // Build text lines
    let mut text_lines: Vec<Line> = lines
        .into_iter()
        .skip(current_scroll)
        .take(visible_lines)
        .map(|s| Line::from(Span::styled(s, Style::default().fg(theme.error))))
        .collect();

    // Add scroll indicator if needed
    if total_lines > visible_lines {
        text_lines.push(Line::from(""));
        let scroll_hint = format!(
            "({}/{}) - ↑/↓ gorgeteshez, Esc bezaras",
            current_scroll + 1,
            max_scroll + 1
        );
        text_lines.push(Line::from(Span::styled(
            scroll_hint,
            Style::default().fg(theme.muted),
        )));
    } else {
        text_lines.push(Line::from(""));
        text_lines.push(Line::from(Span::styled(
            "Esc bezaras",
            Style::default().fg(theme.muted),
        )));
    }

    let paragraph = Paragraph::new(text_lines).style(Style::default().fg(theme.fg));

    frame.render_widget(paragraph, inner);
}

/// Simple word-wrap function
fn wrap_text(text: &str, max_width: usize) -> Vec<String> {
    if max_width == 0 {
        return vec![text.to_string()];
    }

    let mut lines = Vec::new();

    for line in text.lines() {
        if line.len() <= max_width {
            lines.push(line.to_string());
        } else {
            // Simple character-based wrap
            let mut current = String::new();
            for word in line.split_whitespace() {
                if current.is_empty() {
                    if word.len() > max_width {
                        // Word too long, split it
                        let mut remaining = word;
                        while remaining.len() > max_width {
                            lines.push(remaining[..max_width].to_string());
                            remaining = &remaining[max_width..];
                        }
                        current = remaining.to_string();
                    } else {
                        current = word.to_string();
                    }
                } else if current.len() + 1 + word.len() <= max_width {
                    current.push(' ');
                    current.push_str(word);
                } else {
                    lines.push(current);
                    if word.len() > max_width {
                        let mut remaining = word;
                        while remaining.len() > max_width {
                            lines.push(remaining[..max_width].to_string());
                            remaining = &remaining[max_width..];
                        }
                        current = remaining.to_string();
                    } else {
                        current = word.to_string();
                    }
                }
            }
            if !current.is_empty() {
                lines.push(current);
            }
        }
    }

    if lines.is_empty() {
        lines.push(String::new());
    }

    lines
}
