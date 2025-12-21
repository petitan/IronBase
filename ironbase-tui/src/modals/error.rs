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
/// BUG FIX: Use char count instead of byte length for UTF-8 safety
fn wrap_text(text: &str, max_width: usize) -> Vec<String> {
    if max_width == 0 {
        return vec![text.to_string()];
    }

    let mut lines = Vec::new();

    for line in text.lines() {
        // Use char count, not byte length
        if line.chars().count() <= max_width {
            lines.push(line.to_string());
        } else {
            // Simple character-based wrap
            let mut current = String::new();
            let mut current_len = 0usize;

            for word in line.split_whitespace() {
                let word_len = word.chars().count();

                if current.is_empty() {
                    if word_len > max_width {
                        // Word too long, split it by chars (UTF-8 safe)
                        let mut chars = word.chars().peekable();
                        while chars.peek().is_some() {
                            let chunk: String = chars.by_ref().take(max_width).collect();
                            if chars.peek().is_some() {
                                // More chars remaining, push this chunk
                                lines.push(chunk);
                            } else {
                                // Last chunk, keep as current
                                current = chunk;
                                current_len = current.chars().count();
                            }
                        }
                    } else {
                        current = word.to_string();
                        current_len = word_len;
                    }
                } else if current_len + 1 + word_len <= max_width {
                    current.push(' ');
                    current.push_str(word);
                    current_len += 1 + word_len;
                } else {
                    lines.push(std::mem::take(&mut current));
                    current_len = 0;
                    if word_len > max_width {
                        // Word too long, split it by chars (UTF-8 safe)
                        let mut chars = word.chars().peekable();
                        while chars.peek().is_some() {
                            let chunk: String = chars.by_ref().take(max_width).collect();
                            if chars.peek().is_some() {
                                lines.push(chunk);
                            } else {
                                current = chunk;
                                current_len = current.chars().count();
                            }
                        }
                    } else {
                        current = word.to_string();
                        current_len = word_len;
                    }
                }
            }
            if !current.is_empty() {
                lines.push(std::mem::take(&mut current));
            }
        }
    }

    if lines.is_empty() {
        lines.push(String::new());
    }

    lines
}
