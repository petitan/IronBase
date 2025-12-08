//! Search modal - command palette style search

use super::render_modal_frame;
use crate::app::{App, SearchMode};
use crate::theme::Theme;
use ratatui::prelude::*;
use ratatui::widgets::{List, ListItem, Paragraph};

/// Render the search modal
pub fn render(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let inner = render_modal_frame(frame, area, "Kereses", theme, 60, 50);

    // Split into input, mode selector, and results
    let chunks = Layout::vertical([
        Constraint::Length(3), // Input
        Constraint::Length(2), // Mode hints
        Constraint::Min(3),    // Results
    ])
    .split(inner);

    // Search input
    render_search_input(frame, chunks[0], app, theme);

    // Mode selector hints
    render_mode_hints(frame, chunks[1], app, theme);

    // Results
    render_results(frame, chunks[2], app, theme);
}

fn render_search_input(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let input_text = if app.search.query_input.is_empty() {
        "Irj be keresokifejezest..."
    } else {
        &app.search.query_input
    };

    let style = if app.search.query_input.is_empty() {
        Style::default().fg(theme.muted)
    } else {
        Style::default().fg(theme.fg)
    };

    // Add cursor if active
    let display = if app.search.input_active && !app.search.query_input.is_empty() {
        let before = &app.search.query_input[..app.search.cursor_pos];
        let after = &app.search.query_input[app.search.cursor_pos..];
        format!("{}|{}", before, after)
    } else if app.search.input_active {
        "|".to_string()
    } else {
        input_text.to_string()
    };

    let input = Paragraph::new(display)
        .style(style)
        .alignment(Alignment::Left);

    frame.render_widget(input, area);
}

fn render_mode_hints(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let (coll_style, doc_style) = match app.search.mode {
        SearchMode::Collections => (
            Style::default().fg(theme.bg).bg(theme.accent),
            Style::default().fg(theme.muted),
        ),
        SearchMode::Documents => (
            Style::default().fg(theme.muted),
            Style::default().fg(theme.bg).bg(theme.accent),
        ),
    };

    let line = Line::from(vec![
        Span::styled("[c]", Style::default().fg(theme.accent)),
        Span::styled(" Kollekcio ", coll_style),
        Span::raw("  "),
        Span::styled("[d]", Style::default().fg(theme.accent)),
        Span::styled(" Dokumentum ", doc_style),
        Span::raw("  "),
        Span::styled("[Tab]", Style::default().fg(theme.muted)),
        Span::styled(" valtas", Style::default().fg(theme.muted)),
    ]);

    let hints = Paragraph::new(line);
    frame.render_widget(hints, area);
}

fn render_results(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    if app.search.results.is_empty() {
        let msg = if app.search.query_input.is_empty() {
            ""
        } else {
            "Nincs talalat"
        };
        let empty = Paragraph::new(msg)
            .style(Style::default().fg(theme.muted))
            .alignment(Alignment::Center);
        frame.render_widget(empty, area);
        return;
    }

    let visible_height = area.height as usize;
    let items: Vec<ListItem> = app
        .search
        .results
        .iter()
        .enumerate()
        .skip(app.search.result_offset)
        .take(visible_height)
        .map(|(idx, result)| {
            let is_selected = idx == app.search.selected_result;
            let style = if is_selected {
                Style::default().fg(theme.bg).bg(theme.accent)
            } else {
                Style::default().fg(theme.fg)
            };

            let text = match result {
                crate::app::SearchResult::Collection {
                    name, doc_count, ..
                } => {
                    format!("  {} ({} docs)", name, doc_count)
                }
                crate::app::SearchResult::Document {
                    collection,
                    doc_id,
                    preview,
                    ..
                } => {
                    format!("  [{}] #{} {}", collection, doc_id, truncate(preview, 30))
                }
            };

            ListItem::new(text).style(style)
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, area);
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        &s[..max]
    }
}
