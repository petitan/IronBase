//! Database open/create modal

use super::render_modal_frame;
use crate::app::{DatabaseMode, DatabaseState};
use crate::theme::Theme;
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

/// Render the database open/create modal
pub fn render(frame: &mut Frame, area: Rect, state: &DatabaseState, theme: &Theme) {
    let title = match state.mode {
        DatabaseMode::Open => "Adatbazis Megnyitas",
        DatabaseMode::Create => "Uj Adatbazis Letrehozasa",
    };
    let inner = render_modal_frame(frame, area, title, theme, 70, 50);

    // Split into sections
    let chunks = Layout::vertical([
        Constraint::Length(2), // Help text
        Constraint::Length(3), // Mode selector
        Constraint::Length(3), // Info
        Constraint::Length(3), // Path input
        Constraint::Min(1),    // Status/error
    ])
    .split(inner);

    // Help text
    let help = Paragraph::new(Line::from(vec![
        Span::styled("[Tab]", Style::default().fg(theme.accent)),
        Span::raw(" Mod valtas  "),
        Span::styled("[Enter]", Style::default().fg(theme.accent)),
        Span::raw(" OK  "),
        Span::styled("[Esc]", Style::default().fg(theme.accent)),
        Span::raw(" Bezar"),
    ]))
    .style(Style::default().fg(theme.muted));
    frame.render_widget(help, chunks[0]);

    // Mode selector
    let open_style = if state.mode == DatabaseMode::Open {
        Style::default().fg(theme.bg).bg(theme.accent).bold()
    } else {
        Style::default().fg(theme.muted)
    };
    let create_style = if state.mode == DatabaseMode::Create {
        Style::default().fg(theme.bg).bg(theme.accent).bold()
    } else {
        Style::default().fg(theme.muted)
    };

    let mode_line = Line::from(vec![
        Span::raw(" Mod: "),
        Span::styled(" Megnyitas ", open_style),
        Span::raw("  "),
        Span::styled(" Letrehozas ", create_style),
    ]);
    frame.render_widget(Paragraph::new(mode_line), chunks[1]);

    // Info text
    let info_text = match state.mode {
        DatabaseMode::Open => "Add meg egy letezo adatbazis fajl utvonalat.".to_string(),
        DatabaseMode::Create => {
            "Add meg az uj adatbazis fajl utvonalat.\nA fajl nem letezhet meg!".to_string()
        }
    };
    let info = Paragraph::new(info_text).style(Style::default().fg(theme.fg));
    frame.render_widget(info, chunks[2]);

    // Path input
    let label = " Utvonal: ";
    let [label_area, input_area] =
        Layout::horizontal([Constraint::Length(label.len() as u16), Constraint::Min(1)])
            .areas(chunks[3]);
    frame.render_widget(
        Paragraph::new(Span::styled(label, Style::default().fg(theme.accent))),
        label_area,
    );
    state
        .path
        .render(frame, input_area, Style::default().fg(theme.fg), true);

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
        let tip = match state.mode {
            DatabaseMode::Open => " Tipp: Hasznalj abszolut utvonalat (pl. /home/user/data.mlite)",
            DatabaseMode::Create => " Tipp: A .mlite kiterjesztes ajanlott",
        };
        Line::from(vec![Span::styled(tip, Style::default().fg(theme.muted))])
    };
    frame.render_widget(Paragraph::new(status_text), chunks[4]);
}
