//! Insert/Edit document modal - multi-line JSON editor

use super::render_modal_frame;
use crate::app::InsertState;
use crate::theme::Theme;
use crate::widgets::text_area::plain;
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
        Span::styled("[^S]", Style::default().fg(theme.accent)),
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
    state.editor.render(
        frame,
        area,
        Style::default().bg(theme.bg),
        Some(Style::default().fg(theme.muted)),
        &plain(Style::default().fg(theme.fg)),
        true,
    );
}

fn render_status(frame: &mut Frame, area: Rect, state: &InsertState, theme: &Theme) {
    let status_text = if let Some(ref err) = state.error {
        Line::from(vec![
            Span::styled("Hiba: ", Style::default().fg(theme.error)),
            Span::styled(err.clone(), Style::default().fg(theme.error)),
        ])
    } else {
        let (line, col) = state.editor.cursor();
        let (line, col) = (line + 1, col + 1);
        Line::from(vec![Span::styled(
            format!("Sor: {} Oszlop: {}", line, col),
            Style::default().fg(theme.muted),
        )])
    };

    let status = Paragraph::new(status_text);
    frame.render_widget(status, area);
}
