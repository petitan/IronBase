//! Detail pane - right panel showing full document JSON

use crate::app::App;
use crate::base64_detect::sanitize_base64_values;
use crate::i18n::t;
use crate::theme::Theme;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

/// Render the detail pane (right panel)
pub fn render(frame: &mut Frame, area: Rect, app: &App, theme: &Theme, focused: bool) {
    let border_color = if focused {
        theme.accent
    } else {
        theme.secondary
    };
    let title_style = if focused {
        Style::default().fg(theme.accent).bold()
    } else {
        Style::default().fg(theme.secondary)
    };

    // Show match info in title if searching
    let matches = (!app.search.doc_matches.is_empty())
        .then(|| (app.search.current_match + 1, app.search.doc_matches.len()));
    let title = t().detail_title(matches);

    let block = Block::default()
        .title(title)
        .title_style(title_style)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Get selected document - BUG FIX: use if-let instead of is_none + unwrap
    let Some(doc) = app.get_selected_document() else {
        let empty_msg = Paragraph::new(t().select_document())
            .style(Style::default().fg(theme.muted))
            .alignment(Alignment::Center);
        frame.render_widget(empty_msg, inner);
        return;
    };

    // Get search query for highlighting (if any)
    let highlight_query = if !app.search.doc_matches.is_empty() {
        Some(app.search.query_input.value().trim().to_lowercase())
    } else {
        None
    };

    // Pretty print JSON with syntax highlighting and search highlighting
    let formatted = format_json_pretty(doc, theme, highlight_query.as_deref());

    let paragraph = Paragraph::new(formatted)
        .scroll((app.detail_scroll as u16, 0))
        .wrap(Wrap { trim: false });

    frame.render_widget(paragraph, inner);
}

/// Format JSON with basic syntax highlighting and optional search match highlighting
fn format_json_pretty(
    value: &serde_json::Value,
    theme: &Theme,
    highlight: Option<&str>,
) -> Vec<Line<'static>> {
    // Base64 tartalmak helyettesítése előnézettel
    let sanitized = sanitize_base64_values(value);
    let pretty = serde_json::to_string_pretty(&sanitized).unwrap_or_else(|_| "{}".to_string());

    let mut lines = Vec::new();

    for line in pretty.lines() {
        if let Some(query) = highlight {
            // Check if this line contains the search query
            let line_lower = line.to_lowercase();
            if line_lower.contains(query) {
                // Highlight matches in this line
                lines.push(highlight_line(line, query, theme));
                continue;
            }
        }

        // Normal syntax highlighting
        lines.push(syntax_highlight_line(line, theme));
    }

    lines
}

/// Highlight search matches in a line (UTF-8 safe)
fn highlight_line(line: &str, query: &str, theme: &Theme) -> Line<'static> {
    let line_lower = line.to_lowercase();
    let mut spans = Vec::new();

    // Build char index to byte index mappings for UTF-8 safe slicing
    let char_indices: Vec<usize> = line.char_indices().map(|(i, _)| i).collect();
    let char_indices_lower: Vec<usize> = line_lower.char_indices().map(|(i, _)| i).collect();

    // If char counts differ due to case mapping, fall back to simple display
    if char_indices.len() != char_indices_lower.len() {
        return syntax_highlight_line(line, theme);
    }

    let mut last_char_end = 0;

    // Find all matches using char indices
    let query_char_len = query.chars().count();
    let mut search_char_start = 0;

    while search_char_start + query_char_len <= char_indices_lower.len() {
        // Get byte slice for search
        let search_byte_start = char_indices_lower[search_char_start];
        let search_byte_end = if search_char_start + query_char_len < char_indices_lower.len() {
            char_indices_lower[search_char_start + query_char_len]
        } else {
            line_lower.len()
        };

        if line_lower[search_byte_start..search_byte_end].starts_with(query) {
            // Found a match at search_char_start
            let match_char_start = search_char_start;
            let match_char_end = search_char_start + query_char_len;

            // Add text before match
            if match_char_start > last_char_end {
                let byte_start = char_indices[last_char_end];
                let byte_end = char_indices[match_char_start];
                spans.push(Span::styled(
                    line[byte_start..byte_end].to_string(),
                    Style::default().fg(theme.fg),
                ));
            }

            // Add highlighted match
            let match_byte_start = char_indices[match_char_start];
            let match_byte_end = if match_char_end < char_indices.len() {
                char_indices[match_char_end]
            } else {
                line.len()
            };
            spans.push(Span::styled(
                line[match_byte_start..match_byte_end].to_string(),
                Style::default()
                    .fg(theme.bg)
                    .bg(theme.warning)
                    .add_modifier(Modifier::BOLD),
            ));

            last_char_end = match_char_end;
            search_char_start = match_char_end;
        } else {
            search_char_start += 1;
        }
    }

    // Add remaining text
    if last_char_end < char_indices.len() {
        let byte_start = char_indices[last_char_end];
        spans.push(Span::styled(
            line[byte_start..].to_string(),
            Style::default().fg(theme.fg),
        ));
    } else if last_char_end == 0 {
        // No matches found, return whole line
        spans.push(Span::styled(
            line.to_string(),
            Style::default().fg(theme.fg),
        ));
    }

    Line::from(spans)
}

/// Apply syntax highlighting to a JSON line (no search highlight)
fn syntax_highlight_line(line: &str, theme: &Theme) -> Line<'static> {
    let mut spans = Vec::new();
    let mut chars = line.chars().peekable();
    let mut current = String::new();
    let mut in_string = false;

    // Simple state machine for JSON syntax highlighting
    while let Some(c) = chars.next() {
        match c {
            // An escaped character (`\"`, `\\`) is part of the string
            '\\' if in_string => {
                current.push(c);
                if let Some(escaped) = chars.next() {
                    current.push(escaped);
                }
            }
            '"' => {
                if in_string {
                    current.push(c);
                    // A key is a string followed by `:` (whitespace aside)
                    let is_key = chars.clone().find(|c| !c.is_whitespace()) == Some(':');
                    let color = if is_key { theme.accent } else { theme.success };
                    spans.push(Span::styled(current.clone(), Style::default().fg(color)));
                    current.clear();
                    in_string = false;
                } else {
                    if !current.is_empty() {
                        spans.push(Span::styled(current.clone(), Style::default().fg(theme.fg)));
                        current.clear();
                    }
                    current.push(c);
                    in_string = true;
                }
            }
            ':' | ',' | '{' | '}' | '[' | ']' => {
                if in_string {
                    current.push(c);
                } else {
                    if !current.is_empty() {
                        spans.push(Span::styled(current.clone(), Style::default().fg(theme.fg)));
                        current.clear();
                    }
                    spans.push(Span::styled(
                        c.to_string(),
                        Style::default().fg(theme.secondary),
                    ));
                }
            }
            't' if !in_string => {
                // Check for "true"
                let rest: String = chars.clone().take(3).collect();
                if rest == "rue" {
                    for _ in 0..3 {
                        chars.next();
                    }
                    flush_plain(&mut spans, &mut current, theme);
                    spans.push(Span::styled("true", Style::default().fg(theme.success)));
                } else {
                    current.push(c);
                }
            }
            'f' if !in_string => {
                // Check for "false"
                let rest: String = chars.clone().take(4).collect();
                if rest == "alse" {
                    for _ in 0..4 {
                        chars.next();
                    }
                    flush_plain(&mut spans, &mut current, theme);
                    spans.push(Span::styled("false", Style::default().fg(theme.error)));
                } else {
                    current.push(c);
                }
            }
            'n' if !in_string => {
                // Check for "null"
                let rest: String = chars.clone().take(3).collect();
                if rest == "ull" {
                    for _ in 0..3 {
                        chars.next();
                    }
                    flush_plain(&mut spans, &mut current, theme);
                    spans.push(Span::styled("null", Style::default().fg(theme.muted)));
                } else {
                    current.push(c);
                }
            }
            _ if !in_string && (c.is_ascii_digit() || c == '-') => {
                flush_plain(&mut spans, &mut current, theme);
                current.push(c);
                // Consume full number - BUG FIX: Use while-let instead of peek + unwrap
                while let Some(&nc) = chars.peek() {
                    if nc.is_ascii_digit()
                        || nc == '.'
                        || nc == 'e'
                        || nc == 'E'
                        || nc == '+'
                        || nc == '-'
                    {
                        current.push(chars.next().unwrap_or(nc)); // Safe: peek returned Some
                    } else {
                        break;
                    }
                }
                spans.push(Span::styled(
                    current.clone(),
                    Style::default().fg(theme.warning),
                ));
                current.clear();
            }
            _ => {
                current.push(c);
            }
        }
    }

    if !current.is_empty() {
        let color = if in_string { theme.success } else { theme.fg };
        spans.push(Span::styled(current, Style::default().fg(color)));
    }

    Line::from(spans)
}

/// Push the plain text gathered so far (whitespace between tokens) before a
/// token, so the spans keep the line's order
fn flush_plain(spans: &mut Vec<Span<'static>>, current: &mut String, theme: &Theme) {
    if !current.is_empty() {
        spans.push(Span::styled(
            std::mem::take(current),
            Style::default().fg(theme.fg),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    /// The highlighted line has the same text as the plain line
    #[test]
    fn test_highlight_keeps_the_text() {
        let theme = Theme::dark();
        for line in [
            r#"  "elérhető": true,"#,
            r#"  "a": false,"#,
            r#"  "b": null"#,
            r#"  "év": 1906,"#,
            r#"  "url": "http://x:1","#,
        ] {
            assert_eq!(text(&syntax_highlight_line(line, &theme)), line);
        }
    }

    /// An escaped quote does not end a string: the rest of it keeps the
    /// string's color, and `true` inside it is not a literal
    #[test]
    fn test_escaped_quote_stays_in_the_string() {
        let theme = Theme::dark();
        let line = r#"  "a\"b": "x \"true\" -1","#;
        let highlighted = syntax_highlight_line(line, &theme);
        assert_eq!(text(&highlighted), line);
        let color = |s: &str| {
            highlighted
                .spans
                .iter()
                .find(|span| span.content == s)
                .and_then(|span| span.style.fg)
        };
        assert_eq!(color(r#""a\"b""#), Some(theme.accent));
        assert_eq!(color(r#""x \"true\" -1""#), Some(theme.success));
    }

    /// Only a string followed by `:` is a key (a value with a colon is not)
    #[test]
    fn test_a_value_with_a_colon_is_not_a_key() {
        let theme = Theme::dark();
        let line = syntax_highlight_line(r#"  "url": "http://x","#, &theme);
        let color = |s: &str| {
            line.spans
                .iter()
                .find(|span| span.content == s)
                .and_then(|span| span.style.fg)
        };
        assert_eq!(color(r#""url""#), Some(theme.accent));
        assert_eq!(color(r#""http://x""#), Some(theme.success));
    }
}
