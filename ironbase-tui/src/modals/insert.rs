//! Insert/Edit document modal - multi-line JSON editor

use super::render_modal_frame;
use crate::app::InsertState;
use crate::theme::Theme;
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

/// Render the insert/edit document modal
pub fn render(frame: &mut Frame, area: Rect, state: &InsertState, theme: &Theme) {
    let title = if state.is_edit_mode() {
        "Dokumentum szerkesztese"
    } else {
        "Uj dokumentum"
    };
    let inner = render_modal_frame(frame, area, title, theme, 70, 60);

    // Split into help text, editor, and status
    let chunks = Layout::vertical([
        Constraint::Length(2), // Help text
        Constraint::Min(5),    // Editor area
        Constraint::Length(2), // Status/error + buttons
    ])
    .split(inner);

    // Help text
    let help = Paragraph::new(Line::from(vec![
        Span::styled("[Ctrl+S]", Style::default().fg(theme.accent)),
        Span::raw(" / "),
        Span::styled("[F5]", Style::default().fg(theme.accent)),
        Span::raw(" Mentes  "),
        Span::styled("[Esc]", Style::default().fg(theme.accent)),
        Span::raw(" Megse  "),
        Span::styled("[Tab]", Style::default().fg(theme.muted)),
        Span::raw(" 2 space"),
    ]))
    .style(Style::default().fg(theme.muted));
    frame.render_widget(help, chunks[0]);

    // Editor area
    render_editor(frame, chunks[1], state, theme);

    // Status/error line
    render_status(frame, chunks[2], state, theme);
}

fn render_editor(frame: &mut Frame, area: Rect, state: &InsertState, theme: &Theme) {
    let visible_height = area.height as usize;

    // Build display lines with cursor
    let mut lines: Vec<Line> = Vec::new();

    for (line_idx, line) in state.lines.iter().enumerate() {
        let is_cursor_line = line_idx == state.cursor_line;

        if is_cursor_line {
            // Line with cursor - use character-based slicing for UTF-8 safety
            let char_count = line.chars().count();
            let before: String = line.chars().take(state.cursor_col).collect();
            let after: String = if state.cursor_col < char_count {
                line.chars().skip(state.cursor_col).collect()
            } else {
                String::new()
            };

            let line_num = format!("{:3} ", line_idx + 1);
            lines.push(Line::from(vec![
                Span::styled(line_num, Style::default().fg(theme.muted)),
                Span::styled(before, Style::default().fg(theme.fg)),
                Span::styled("│", Style::default().fg(theme.accent)), // Cursor
                Span::styled(after, Style::default().fg(theme.fg)),
            ]));
        } else {
            let line_num = format!("{:3} ", line_idx + 1);
            lines.push(Line::from(vec![
                Span::styled(line_num, Style::default().fg(theme.muted)),
                Span::styled(line.clone(), Style::default().fg(theme.fg)),
            ]));
        }
    }

    // Handle scroll - show lines around cursor
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

fn render_status(frame: &mut Frame, area: Rect, state: &InsertState, theme: &Theme) {
    let status_text = if let Some(ref err) = state.error {
        Line::from(vec![
            Span::styled("Hiba: ", Style::default().fg(theme.error)),
            Span::styled(err.clone(), Style::default().fg(theme.error)),
        ])
    } else {
        let (line, col) = (state.cursor_line + 1, state.cursor_col + 1);
        Line::from(vec![Span::styled(
            format!("Sor: {} Oszlop: {}", line, col),
            Style::default().fg(theme.muted),
        )])
    };

    let status = Paragraph::new(status_text);
    frame.render_widget(status, area);
}
