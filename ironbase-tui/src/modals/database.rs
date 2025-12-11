//! Database open/create modal

use super::render_modal_frame;
use crate::app::DatabaseState;
use crate::theme::Theme;
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

/// Render the database open/create modal
pub fn render(frame: &mut Frame, area: Rect, state: &DatabaseState, theme: &Theme) {
    let inner = render_modal_frame(frame, area, "Adatbazis Megnyitas", theme, 70, 40);

    // Split into sections
    let chunks = Layout::vertical([
        Constraint::Length(2), // Help text
        Constraint::Length(3), // Info
        Constraint::Length(3), // Path input
        Constraint::Min(1),    // Status/error
    ])
    .split(inner);

    // Help text
    let help = Paragraph::new(Line::from(vec![
        Span::styled("[Enter]", Style::default().fg(theme.accent)),
        Span::raw(" Megnyitas  "),
        Span::styled("[Esc]", Style::default().fg(theme.accent)),
        Span::raw(" Bezar"),
    ]))
    .style(Style::default().fg(theme.muted));
    frame.render_widget(help, chunks[0]);

    // Info text
    let info_text = if state.is_http_mode {
        "HTTP modban nem valthatod az adatbazist."
    } else {
        "Add meg az adatbazis fajl utvonalat.\nHa nem letezik, letrehozzuk."
    };
    let info = Paragraph::new(info_text)
        .style(Style::default().fg(if state.is_http_mode { theme.warning } else { theme.fg }));
    frame.render_widget(info, chunks[1]);

    // Path input (only if not HTTP mode)
    if !state.is_http_mode {
        let before: String = state.path.chars().take(state.cursor).collect();
        let after: String = state.path.chars().skip(state.cursor).collect();

        let path_line = Line::from(vec![
            Span::styled(" Utvonal: ", Style::default().fg(theme.accent)),
            Span::styled(before, Style::default().fg(theme.fg)),
            Span::styled("|", Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)),
            Span::styled(after, Style::default().fg(theme.fg)),
        ]);
        frame.render_widget(Paragraph::new(path_line), chunks[2]);
    }

    // Status/error line
    let status_text = if let Some(ref err) = state.error {
        Line::from(vec![
            Span::styled(" Hiba: ", Style::default().fg(theme.error)),
            Span::styled(err.clone(), Style::default().fg(theme.error)),
        ])
    } else if let Some(ref msg) = state.message {
        Line::from(vec![Span::styled(
            format!(" {}", msg),
            Style::default().fg(theme.success),
        )])
    } else if state.loading {
        Line::from(vec![Span::styled(
            " Csatlakozas...",
            Style::default().fg(theme.accent),
        )])
    } else {
        Line::from(vec![Span::styled(
            " Tipp: Hasznalj abszolut utvonalat (pl. /home/user/data.mlite)",
            Style::default().fg(theme.muted),
        )])
    };
    frame.render_widget(Paragraph::new(status_text), chunks[3]);
}
