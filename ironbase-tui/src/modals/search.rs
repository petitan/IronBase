//! Search modal - collection or document search

use super::render_modal_frame;
use crate::app::{App, SearchMode};
use crate::i18n::t;
use crate::theme::Theme;
use ratatui::prelude::*;
use ratatui::widgets::{List, ListItem, Paragraph};

/// Render the search modal
pub fn render(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let title = match app.search.mode {
        SearchMode::Collections => t().search_collections_title(),
        SearchMode::Document => t().search_document_title(),
    };

    let inner = render_modal_frame(frame, area, title, theme, 60, 50);

    // Split into input and results/status
    let chunks = Layout::vertical([
        Constraint::Length(3), // Input
        Constraint::Length(1), // Help hint
        Constraint::Min(3),    // Results
    ])
    .split(inner);

    // Search input
    render_search_input(frame, chunks[0], app, theme);

    // Help hint (mode-specific)
    render_help(frame, chunks[1], app, theme);

    // Results or match info
    match app.search.mode {
        SearchMode::Collections => render_collection_results(frame, chunks[2], app, theme),
        SearchMode::Document => render_doc_results(frame, chunks[2], app, theme),
    }
}

fn render_search_input(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let placeholder = match app.search.mode {
        SearchMode::Collections => t().type_collection_name(),
        SearchMode::Document => t().type_search_text(),
    };

    if app.search.query_input.is_empty() {
        frame.render_widget(
            Paragraph::new(placeholder).style(Style::default().fg(theme.muted)),
            area,
        );
        // An active empty field: the cursor waits at its start
        if app.search.input_active {
            frame.set_cursor_position(Position::new(area.x, area.y));
        }
    } else {
        app.search.query_input.render(
            frame,
            area,
            Style::default().fg(theme.fg),
            app.search.input_active,
        );
    }
}

fn render_help(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let line = match app.search.mode {
        SearchMode::Collections => Line::from(vec![
            Span::styled("[Enter]", Style::default().fg(theme.accent)),
            Span::styled(
                format!(" {}  ", t().key_search_or_jump()),
                Style::default().fg(theme.muted),
            ),
            Span::styled("[Esc]", Style::default().fg(theme.accent)),
            Span::styled(
                format!(" {}", t().key_close()),
                Style::default().fg(theme.muted),
            ),
        ]),
        SearchMode::Document => Line::from(vec![
            Span::styled("[Enter]", Style::default().fg(theme.accent)),
            Span::styled(
                format!(" {}  ", t().key_search()),
                Style::default().fg(theme.muted),
            ),
            Span::styled("[Esc]", Style::default().fg(theme.accent)),
            Span::styled(
                format!(" {}", t().key_close()),
                Style::default().fg(theme.muted),
            ),
        ]),
    };

    let hints = Paragraph::new(line);
    frame.render_widget(hints, area);
}

fn render_collection_results(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    if app.search.results.is_empty() {
        let msg = if app.search.query_input.is_empty() {
            ""
        } else {
            t().no_matches()
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

            let text = format!(
                "  {}",
                t().collection_with_count(&result.name, result.doc_count)
            );

            ListItem::new(text).style(style)
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, area);
}

fn render_doc_results(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let text = if app.search.doc_matches.is_empty() {
        if app.search.query_input.is_empty() {
            t().start_typing_to_search().to_string()
        } else {
            t().no_matches().to_string()
        }
    } else {
        t().document_match_help(app.search.current_match + 1, app.search.doc_matches.len())
    };

    let style = if app.search.doc_matches.is_empty() {
        Style::default().fg(theme.muted)
    } else {
        Style::default().fg(theme.accent)
    };

    let p = Paragraph::new(text)
        .style(style)
        .alignment(Alignment::Center);
    frame.render_widget(p, area);
}
