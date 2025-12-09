//! New collection modal - create a new collection

use super::render_modal_frame;
use crate::app::NewCollectionState;
use crate::theme::Theme;
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

/// Render the new collection modal
pub fn render(frame: &mut Frame, area: Rect, state: &NewCollectionState, theme: &Theme) {
    let inner = render_modal_frame(frame, area, "Uj Kollekcio", theme, 50, 30);

    // Split into help, input, and status
    let chunks = Layout::vertical([
        Constraint::Length(2), // Help
        Constraint::Length(3), // Input
        Constraint::Length(2), // Status/error
    ])
    .split(inner);

    // Help text
    let help = Paragraph::new(Line::from(vec![
        Span::styled("[Enter]", Style::default().fg(theme.accent)),
        Span::raw(" Letrehoz  "),
        Span::styled("[Esc]", Style::default().fg(theme.accent)),
        Span::raw(" Megse"),
    ]))
    .style(Style::default().fg(theme.muted));
    frame.render_widget(help, chunks[0]);

    // Name input with cursor
    let display = if state.name.is_empty() {
        "|".to_string()
    } else {
        let before: String = state.name.chars().take(state.cursor).collect();
        let after: String = state.name.chars().skip(state.cursor).collect();
        format!("{}|{}", before, after)
    };

    let input_line = Line::from(vec![
        Span::styled(" Nev: ", Style::default().fg(theme.muted)),
        Span::styled(display, Style::default().fg(theme.fg)),
    ]);
    let input = Paragraph::new(input_line);
    frame.render_widget(input, chunks[1]);

    // Status/error
    let status_text = if let Some(ref err) = state.error {
        Line::from(vec![
            Span::styled(" Hiba: ", Style::default().fg(theme.error)),
            Span::styled(err.clone(), Style::default().fg(theme.error)),
        ])
    } else {
        Line::from(Span::styled(
            " Adj meg egy nevet (a-z, 0-9, _, -)",
            Style::default().fg(theme.muted),
        ))
    };
    let status = Paragraph::new(status_text);
    frame.render_widget(status, chunks[2]);
}
