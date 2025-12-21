//! Fulltext search modal

use super::render_modal_frame;
use crate::app::FulltextState;
use crate::theme::Theme;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

/// Render the fulltext search modal
pub fn render(frame: &mut Frame, area: Rect, state: &FulltextState, theme: &Theme) {
    let inner = render_modal_frame(frame, area, "Fulltext kereses", theme, 70, 80);

    // Split into sections
    let chunks = Layout::vertical([
        Constraint::Length(2), // Help text
        Constraint::Length(3), // Field selector
        Constraint::Length(3), // Search input
        Constraint::Min(5),    // Results
        Constraint::Length(1), // Status
    ])
    .split(inner);

    // Help text
    render_help(frame, chunks[0], theme);

    // Field selector
    render_field_selector(frame, chunks[1], state, theme);

    // Search input
    render_search_input(frame, chunks[2], state, theme);

    // Results
    render_results(frame, chunks[3], state, theme);

    // Status
    render_status(frame, chunks[4], state, theme);
}

fn render_help(frame: &mut Frame, area: Rect, theme: &Theme) {
    let help = Paragraph::new(Line::from(vec![
        Span::styled("[Enter]", Style::default().fg(theme.accent)),
        Span::raw(" Kereses  "),
        Span::styled("[Tab]", Style::default().fg(theme.accent)),
        Span::raw(" Mezo valtas  "),
        Span::styled("[j/k]", Style::default().fg(theme.accent)),
        Span::raw(" Navigacio  "),
        Span::styled("[Esc]", Style::default().fg(theme.accent)),
        Span::raw(" Bezar"),
    ]))
    .style(Style::default().fg(theme.muted));
    frame.render_widget(help, area);
}

fn render_field_selector(frame: &mut Frame, area: Rect, state: &FulltextState, theme: &Theme) {
    let block = Block::default()
        .title(" Mezo ")
        .title_style(Style::default().fg(theme.fg))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.muted));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if state.indexed_fields.is_empty() {
        let msg = Paragraph::new("Nincs fulltext index! Hozz letre egyet elobb.")
            .style(Style::default().fg(theme.error));
        frame.render_widget(msg, inner);
    } else {
        let fields: Vec<Span> = state
            .indexed_fields
            .iter()
            .enumerate()
            .map(|(i, f)| {
                if i == state.selected_field_idx {
                    Span::styled(
                        format!(" [{}] ", f),
                        Style::default().fg(theme.bg).bg(theme.accent),
                    )
                } else {
                    Span::styled(format!(" {} ", f), Style::default().fg(theme.fg))
                }
            })
            .collect();

        let line = Line::from(fields);
        let p = Paragraph::new(line);
        frame.render_widget(p, inner);
    }
}

fn render_search_input(frame: &mut Frame, area: Rect, state: &FulltextState, theme: &Theme) {
    let block = Block::default()
        .title(" Kereses ")
        .title_style(Style::default().fg(theme.fg))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.accent));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let display = if state.query.is_empty() {
        "Irj be keresett szoveget...".to_string()
    } else {
        // Show cursor
        let before: String = state.query.chars().take(state.cursor_pos).collect();
        let after: String = state.query.chars().skip(state.cursor_pos).collect();
        format!("{}|{}", before, after)
    };

    let style = if state.query.is_empty() {
        Style::default().fg(theme.muted)
    } else {
        Style::default().fg(theme.fg)
    };

    let input = Paragraph::new(display).style(style);
    frame.render_widget(input, inner);
}

fn render_results(frame: &mut Frame, area: Rect, state: &FulltextState, theme: &Theme) {
    let block = Block::default()
        .title(format!(" Talalatok ({}) ", state.results.len()))
        .title_style(Style::default().fg(theme.fg))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.muted));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if state.loading {
        let loading = Paragraph::new("Kereses...")
            .style(Style::default().fg(theme.accent))
            .alignment(Alignment::Center);
        frame.render_widget(loading, inner);
        return;
    }

    if state.results.is_empty() {
        let msg = if state.query.is_empty() {
            ""
        } else {
            "Nincs talalat"
        };
        let empty = Paragraph::new(msg)
            .style(Style::default().fg(theme.muted))
            .alignment(Alignment::Center);
        frame.render_widget(empty, inner);
        return;
    }

    let visible_height = inner.height as usize;
    let items: Vec<ListItem> = state
        .results
        .iter()
        .enumerate()
        .take(visible_height)
        .map(|(idx, result)| {
            let is_selected = idx == state.selected_result;
            let style = if is_selected {
                Style::default().fg(theme.bg).bg(theme.accent)
            } else {
                Style::default().fg(theme.fg)
            };

            // Extract score and document preview
            let score = result
                .get("score")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);

            let doc = result.get("document").cloned().unwrap_or_default();
            let preview = format_doc_preview(&doc, 60);

            let text = format!(" {:.2}  {}", score, preview);
            ListItem::new(text).style(style)
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, inner);
}

/// BUG FIX: Use char count instead of byte length for UTF-8 safety
fn format_doc_preview(doc: &serde_json::Value, max_len: usize) -> String {
    // Try to get a meaningful preview
    let preview = if let Some(obj) = doc.as_object() {
        // Skip _id, show first few fields
        let fields: Vec<String> = obj
            .iter()
            .filter(|(k, _)| *k != "_id")
            .take(3)
            .map(|(k, v)| {
                let val_str = match v {
                    serde_json::Value::String(s) => {
                        // UTF-8 safe truncation
                        if s.chars().count() > 30 {
                            let truncated: String = s.chars().take(30).collect();
                            format!("\"{}...\"", truncated)
                        } else {
                            format!("\"{}\"", s)
                        }
                    }
                    _ => v.to_string(),
                };
                format!("{}: {}", k, val_str)
            })
            .collect();
        fields.join(", ")
    } else {
        doc.to_string()
    };

    // UTF-8 safe truncation
    if preview.chars().count() > max_len {
        let truncated: String = preview.chars().take(max_len).collect();
        format!("{}...", truncated)
    } else {
        preview
    }
}

fn render_status(frame: &mut Frame, area: Rect, state: &FulltextState, theme: &Theme) {
    let text = if let Some(ref err) = state.error {
        Line::from(vec![
            Span::styled("Hiba: ", Style::default().fg(theme.error)),
            Span::styled(err.clone(), Style::default().fg(theme.error)),
        ])
    } else {
        Line::from(vec![Span::styled(
            format!("Kollekcio: {}", state.collection),
            Style::default().fg(theme.muted),
        )])
    };

    let status = Paragraph::new(text);
    frame.render_widget(status, area);
}
