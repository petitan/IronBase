//! Search modal - collection or document search

use super::render_modal_frame;
use crate::app::{App, SearchMode};
use crate::theme::Theme;
use ratatui::prelude::*;
use ratatui::widgets::{List, ListItem, Paragraph};

/// Render the search modal
pub fn render(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let title = match app.search.mode {
        SearchMode::Collections => "Kollekciok keresese",
        SearchMode::Document => "Kereses a dokumentumban",
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
        SearchMode::Collections => "Irj be kollekcio nevet...",
        SearchMode::Document => "Irj be keresett szoveget...",
    };

    let input_text = if app.search.query_input.is_empty() {
        placeholder
    } else {
        &app.search.query_input
    };

    let style = if app.search.query_input.is_empty() {
        Style::default().fg(theme.muted)
    } else {
        Style::default().fg(theme.fg)
    };

    // Add cursor if active - UTF-8 safe slicing
    let display = if app.search.input_active && !app.search.query_input.is_empty() {
        let before: String = app
            .search
            .query_input
            .chars()
            .take(app.search.cursor_pos)
            .collect();
        let after: String = app
            .search
            .query_input
            .chars()
            .skip(app.search.cursor_pos)
            .collect();
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

fn render_help(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let line = match app.search.mode {
        SearchMode::Collections => Line::from(vec![
            Span::styled("[Enter]", Style::default().fg(theme.accent)),
            Span::styled(" Kereses/Ugras  ", Style::default().fg(theme.muted)),
            Span::styled("[Esc]", Style::default().fg(theme.accent)),
            Span::styled(" Bezar", Style::default().fg(theme.muted)),
        ]),
        SearchMode::Document => Line::from(vec![
            Span::styled("[Enter]", Style::default().fg(theme.accent)),
            Span::styled(" Kereses  ", Style::default().fg(theme.muted)),
            Span::styled("[n/N]", Style::default().fg(theme.accent)),
            Span::styled(" Kov/Eloz  ", Style::default().fg(theme.muted)),
            Span::styled("[Esc]", Style::default().fg(theme.accent)),
            Span::styled(" Bezar", Style::default().fg(theme.muted)),
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

            let text = format!("  {} ({} docs)", result.name, result.doc_count);

            ListItem::new(text).style(style)
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, area);
}

fn render_doc_results(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let text = if app.search.doc_matches.is_empty() {
        if app.search.query_input.is_empty() {
            "Kezdj el gepelni a kereséshez".to_string()
        } else {
            "Nincs talalat".to_string()
        }
    } else {
        format!(
            "Talalat: {}/{}\n\n[n] Kovetkezo  [N] Elozo\n[Esc] Bezar (talaltok kijelolve maradnak)",
            app.search.current_match + 1,
            app.search.doc_matches.len()
        )
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
