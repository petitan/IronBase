//! Query builder modal - build and execute queries

use super::render_modal_frame;
use crate::app::QueryState;
use crate::theme::Theme;
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

/// Render the query builder modal
pub fn render(frame: &mut Frame, area: Rect, state: &QueryState, theme: &Theme) {
    let inner = render_modal_frame(frame, area, "Query Builder", theme, 75, 70);

    // Split into sections
    let chunks = Layout::vertical([
        Constraint::Length(2), // Help text
        Constraint::Min(8),    // Query editor
        Constraint::Length(6), // Results preview
        Constraint::Length(1), // Status/error
    ])
    .split(inner);

    // Help text
    let help = Paragraph::new(Line::from(vec![
        Span::styled("[Ctrl+S]", Style::default().fg(theme.accent)),
        Span::raw(" / "),
        Span::styled("[F5]", Style::default().fg(theme.accent)),
        Span::raw(" Run  "),
        Span::styled("[Tab]", Style::default().fg(theme.accent)),
        Span::raw(" Templates  "),
        Span::styled("[Esc]", Style::default().fg(theme.accent)),
        Span::raw(" Close"),
    ]))
    .style(Style::default().fg(theme.muted));
    frame.render_widget(help, chunks[0]);

    // Query editor area
    render_editor(frame, chunks[1], state, theme);

    // Results preview
    render_results(frame, chunks[2], state, theme);

    // Status/error line
    render_status(frame, chunks[3], state, theme);
}

fn render_editor(frame: &mut Frame, area: Rect, state: &QueryState, theme: &Theme) {
    let visible_height = area.height as usize;

    // Build display lines with cursor
    let mut lines: Vec<Line> = Vec::new();

    for (line_idx, line) in state.lines.iter().enumerate() {
        let is_cursor_line = line_idx == state.cursor_line;

        if is_cursor_line {
            // Line with cursor - UTF-8 safe slicing
            let char_count = line.chars().count();
            let before: String = line.chars().take(state.cursor_col).collect();
            let after: String = if state.cursor_col < char_count {
                line.chars().skip(state.cursor_col).collect()
            } else {
                String::new()
            };

            let line_num = format!("{:2} ", line_idx + 1);
            lines.push(Line::from(vec![
                Span::styled(line_num, Style::default().fg(theme.muted)),
                Span::styled(before, Style::default().fg(theme.fg)),
                Span::styled("|", Style::default().fg(theme.accent)), // Cursor
                Span::styled(after, Style::default().fg(theme.fg)),
            ]));
        } else {
            let line_num = format!("{:2} ", line_idx + 1);
            lines.push(Line::from(vec![
                Span::styled(line_num, Style::default().fg(theme.muted)),
                Span::styled(line.clone(), Style::default().fg(theme.fg)),
            ]));
        }
    }

    // Handle scroll
    let start_line = if state.cursor_line >= visible_height {
        state.cursor_line - visible_height + 1
    } else {
        0
    };

    let visible_lines: Vec<Line> = lines
        .into_iter()
        .skip(start_line)
        .take(visible_height)
        .collect();

    let editor = Paragraph::new(visible_lines).style(Style::default().bg(theme.bg));

    frame.render_widget(editor, area);
}

fn render_results(frame: &mut Frame, area: Rect, state: &QueryState, theme: &Theme) {
    let result_text = if let Some(ref results) = state.results {
        let count = results.len();
        let preview = if count > 0 {
            let first = serde_json::to_string(&results[0]).unwrap_or_default();
            // UTF-8 safe truncation using character count
            let truncated = if first.chars().count() > 80 {
                let chars: String = first.chars().take(80).collect();
                format!("{}...", chars)
            } else {
                first
            };
            format!("{} talalat. Elso: {}", count, truncated)
        } else {
            "Nincs talalat".to_string()
        };
        preview
    } else {
        "Irj query-t es nyomd meg Ctrl+S vagy F5".to_string()
    };

    let results = Paragraph::new(vec![
        Line::from(Span::styled("Eredmeny:", Style::default().fg(theme.accent))),
        Line::from(Span::styled(result_text, Style::default().fg(theme.muted))),
    ]);

    frame.render_widget(results, area);
}

fn render_status(frame: &mut Frame, area: Rect, state: &QueryState, theme: &Theme) {
    let status_text = if let Some(ref err) = state.error {
        Line::from(vec![
            Span::styled("Hiba: ", Style::default().fg(theme.error)),
            Span::styled(err.clone(), Style::default().fg(theme.error)),
        ])
    } else {
        let (line, col) = (state.cursor_line + 1, state.cursor_col + 1);
        Line::from(vec![Span::styled(
            format!(
                "Sor: {} Oszlop: {} | Kollekció: {}",
                line, col, state.collection
            ),
            Style::default().fg(theme.muted),
        )])
    };

    let status = Paragraph::new(status_text);
    frame.render_widget(status, area);
}
