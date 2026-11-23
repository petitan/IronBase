//! New collection modal - create a new collection

use super::render_modal_frame;
use crate::app::NewCollectionState;
use crate::i18n::t;
use crate::theme::Theme;
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

/// Render the new collection modal
pub fn render(frame: &mut Frame, area: Rect, state: &NewCollectionState, theme: &Theme) {
    let inner = render_modal_frame(frame, area, t().new_collection_title(), theme, 50, 30);

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
        Span::raw(format!(" {}  ", t().key_create())),
        Span::styled("[Esc]", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}", t().key_cancel())),
    ]))
    .style(Style::default().fg(theme.muted));
    frame.render_widget(help, chunks[0]);

    // Name input
    state.name.render_labeled(
        frame,
        chunks[1],
        Span::styled(
            format!(" {}: ", t().label_name()),
            Style::default().fg(theme.muted),
        ),
        Style::default().fg(theme.fg),
        true,
    );

    // Status/error
    let status_text = if let Some(ref err) = state.error {
        Line::from(vec![
            Span::styled(
                format!(" {}: ", t().label_error()),
                Style::default().fg(theme.error),
            ),
            Span::styled(err.clone(), Style::default().fg(theme.error)),
        ])
    } else {
        Line::from(Span::styled(
            format!(" {}", t().collection_name_hint()),
            Style::default().fg(theme.muted),
        ))
    };
    let status = Paragraph::new(status_text);
    frame.render_widget(status, chunks[2]);
}
