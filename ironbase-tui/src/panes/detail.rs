//! Detail pane - right panel showing full document JSON

use crate::app::App;
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

    let block = Block::default()
        .title(" Detail ")
        .title_style(title_style)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Get selected document
    let doc = app.get_selected_document();

    if doc.is_none() {
        let empty_msg = Paragraph::new("Valassz dokumentumot")
            .style(Style::default().fg(theme.muted))
            .alignment(Alignment::Center);
        frame.render_widget(empty_msg, inner);
        return;
    }

    let doc = doc.unwrap();

    // Pretty print JSON with syntax highlighting
    let formatted = format_json_pretty(&doc, theme);

    let paragraph = Paragraph::new(formatted)
        .scroll((app.detail_scroll as u16, 0))
        .wrap(Wrap { trim: false });

    frame.render_widget(paragraph, inner);
}

/// Format JSON with basic syntax highlighting
fn format_json_pretty(value: &serde_json::Value, theme: &Theme) -> Vec<Line<'static>> {
    let pretty = serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string());

    let mut lines = Vec::new();

    for line in pretty.lines() {
        let mut spans = Vec::new();
        let mut chars = line.chars().peekable();
        let mut current = String::new();
        let mut in_string = false;
        let mut is_key = false;

        // Simple state machine for JSON syntax highlighting
        while let Some(c) = chars.next() {
            match c {
                '"' => {
                    if in_string {
                        current.push(c);
                        let color = if is_key { theme.accent } else { theme.success };
                        spans.push(Span::styled(current.clone(), Style::default().fg(color)));
                        current.clear();
                        in_string = false;
                        is_key = false;
                    } else {
                        if !current.is_empty() {
                            spans
                                .push(Span::styled(current.clone(), Style::default().fg(theme.fg)));
                            current.clear();
                        }
                        current.push(c);
                        in_string = true;
                        // Check if this is a key (followed by : somewhere after)
                        let rest: String = chars.clone().collect();
                        is_key = rest.contains(':');
                    }
                }
                ':' | ',' | '{' | '}' | '[' | ']' => {
                    if in_string {
                        current.push(c);
                    } else {
                        if !current.is_empty() {
                            spans
                                .push(Span::styled(current.clone(), Style::default().fg(theme.fg)));
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
                        spans.push(Span::styled("null", Style::default().fg(theme.muted)));
                    } else {
                        current.push(c);
                    }
                }
                _ if !in_string && (c.is_ascii_digit() || c == '-') => {
                    current.push(c);
                    // Consume full number
                    while chars
                        .peek()
                        .map(|&nc| {
                            nc.is_ascii_digit()
                                || nc == '.'
                                || nc == 'e'
                                || nc == 'E'
                                || nc == '+'
                                || nc == '-'
                        })
                        .unwrap_or(false)
                    {
                        current.push(chars.next().unwrap());
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

        lines.push(Line::from(spans));
    }

    lines
}
