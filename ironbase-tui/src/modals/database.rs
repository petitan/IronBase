//! Database open/create modal

use super::render_modal_frame;
use crate::app::{DatabaseMode, DatabaseState};
use crate::i18n::t;
use crate::theme::Theme;
use crate::widgets::choice;
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

/// Render the database open/create modal
pub fn render(frame: &mut Frame, area: Rect, state: &DatabaseState, theme: &Theme) {
    let title = match state.mode {
        DatabaseMode::Open => t().open_database_title(),
        DatabaseMode::Create => t().create_database_title(),
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
        Span::raw(format!(" {}  ", t().key_switch_mode())),
        Span::styled("[Enter]", Style::default().fg(theme.accent)),
        Span::raw(" OK  "),
        Span::styled("[Esc]", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}", t().key_close())),
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
        Span::raw(format!(" {}: ", t().label_mode())),
        Span::styled(
            choice(t().mode_open(), state.mode == DatabaseMode::Open),
            open_style,
        ),
        Span::raw("  "),
        Span::styled(
            choice(t().mode_create(), state.mode == DatabaseMode::Create),
            create_style,
        ),
    ]);
    frame.render_widget(Paragraph::new(mode_line), chunks[1]);

    // Info text
    let info_text = match state.mode {
        DatabaseMode::Open => t().open_database_info().to_string(),
        DatabaseMode::Create => t().create_database_info().to_string(),
    };
    let info = Paragraph::new(info_text).style(Style::default().fg(theme.fg));
    frame.render_widget(info, chunks[2]);

    // Path input
    state.path.render_labeled(
        frame,
        chunks[3],
        Span::styled(
            format!(" {}: ", t().label_path()),
            Style::default().fg(theme.accent),
        ),
        Style::default().fg(theme.fg),
        true,
    );

    // Status/error line
    let status_text = if let Some(ref err) = state.error {
        Line::from(vec![
            Span::styled(
                format!(" {}: ", t().label_error()),
                Style::default().fg(theme.error),
            ),
            Span::styled(err.clone(), Style::default().fg(theme.error)),
        ])
    } else if let Some(ref msg) = state.message {
        Line::from(vec![Span::styled(
            format!(" {}", msg),
            Style::default().fg(theme.success),
        )])
    } else if state.loading {
        Line::from(vec![Span::styled(
            format!(" {}", t().connecting()),
            Style::default().fg(theme.accent),
        )])
    } else {
        let tip = match state.mode {
            DatabaseMode::Open => t().open_database_tip(),
            DatabaseMode::Create => t().create_database_tip(),
        };
        Line::from(vec![Span::styled(
            format!(" {tip}"),
            Style::default().fg(theme.muted),
        )])
    };
    frame.render_widget(Paragraph::new(status_text), chunks[4]);
}
