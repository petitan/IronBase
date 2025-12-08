//! Index management modal - create/delete indexes

use super::render_modal_frame;
use crate::app::IndexState;
use crate::theme::Theme;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

/// Render the index management modal
pub fn render(frame: &mut Frame, area: Rect, state: &IndexState, theme: &Theme) {
    let inner = render_modal_frame(frame, area, "Index kezeles", theme, 60, 70);

    // Split into sections
    let chunks = Layout::vertical([
        Constraint::Length(2), // Help text
        Constraint::Min(8),    // Existing indexes list
        Constraint::Length(5), // Create new index form
        Constraint::Length(1), // Status/error
    ])
    .split(inner);

    // Help text
    let help = Paragraph::new(Line::from(vec![
        Span::styled("[j/k]", Style::default().fg(theme.accent)),
        Span::raw(" Navigate  "),
        Span::styled("[d]", Style::default().fg(theme.accent)),
        Span::raw(" Delete  "),
        Span::styled("[Tab]", Style::default().fg(theme.accent)),
        Span::raw(" New index  "),
        Span::styled("[Esc]", Style::default().fg(theme.accent)),
        Span::raw(" Close"),
    ]))
    .style(Style::default().fg(theme.muted));
    frame.render_widget(help, chunks[0]);

    // Existing indexes list
    render_index_list(frame, chunks[1], state, theme);

    // Create new index form
    render_create_form(frame, chunks[2], state, theme);

    // Status/error line
    render_status(frame, chunks[3], state, theme);
}

fn render_index_list(frame: &mut Frame, area: Rect, state: &IndexState, theme: &Theme) {
    let block = Block::default()
        .title(" Meglevo indexek ")
        .title_style(Style::default().fg(theme.fg))
        .borders(Borders::ALL)
        .border_style(if !state.is_creating {
            Style::default().fg(theme.accent)
        } else {
            Style::default().fg(theme.muted)
        });

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if state.indexes.is_empty() {
        let empty =
            Paragraph::new("Nincs index (csak _id)").style(Style::default().fg(theme.muted));
        frame.render_widget(empty, inner);
        return;
    }

    let items: Vec<ListItem> = state
        .indexes
        .iter()
        .enumerate()
        .map(|(i, idx)| {
            let style = if !state.is_creating && i == state.selected_index {
                Style::default().fg(theme.bg).bg(theme.accent)
            } else {
                Style::default().fg(theme.fg)
            };

            let unique_marker = if idx.contains("unique") { " [U]" } else { "" };
            ListItem::new(format!("  {}{}  ", idx, unique_marker)).style(style)
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, inner);
}

fn render_create_form(frame: &mut Frame, area: Rect, state: &IndexState, theme: &Theme) {
    let block = Block::default()
        .title(" Uj index letrehozasa ")
        .title_style(Style::default().fg(theme.fg))
        .borders(Borders::ALL)
        .border_style(if state.is_creating {
            Style::default().fg(theme.accent)
        } else {
            Style::default().fg(theme.muted)
        });

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let form_chunks = Layout::vertical([
        Constraint::Length(1), // Field name input
        Constraint::Length(1), // Unique checkbox
        Constraint::Length(1), // Create button hint
    ])
    .split(inner);

    // Field name input
    let field_style = if state.is_creating && state.form_field == 0 {
        Style::default().fg(theme.accent)
    } else {
        Style::default().fg(theme.muted)
    };

    let cursor = if state.is_creating && state.form_field == 0 {
        "|"
    } else {
        ""
    };
    let field_line = Line::from(vec![
        Span::raw(" Mezo neve: "),
        Span::styled(&state.field_input, field_style),
        Span::styled(cursor, Style::default().fg(theme.accent)),
    ]);
    frame.render_widget(Paragraph::new(field_line), form_chunks[0]);

    // Unique checkbox
    let unique_style = if state.is_creating && state.form_field == 1 {
        Style::default().fg(theme.accent)
    } else {
        Style::default().fg(theme.muted)
    };

    let checkbox = if state.unique { "[X]" } else { "[ ]" };
    let unique_line = Line::from(vec![
        Span::raw(" Unique: "),
        Span::styled(checkbox, unique_style),
        Span::raw(" (Space valt)"),
    ]);
    frame.render_widget(Paragraph::new(unique_line), form_chunks[1]);

    // Create hint
    if state.is_creating {
        let hint = Paragraph::new(" [Enter] Letrehoz  [Esc] Megse")
            .style(Style::default().fg(theme.muted));
        frame.render_widget(hint, form_chunks[2]);
    }
}

fn render_status(frame: &mut Frame, area: Rect, state: &IndexState, theme: &Theme) {
    let status_text = if let Some(ref err) = state.error {
        Line::from(vec![
            Span::styled("Hiba: ", Style::default().fg(theme.error)),
            Span::styled(err.clone(), Style::default().fg(theme.error)),
        ])
    } else if let Some(ref msg) = state.message {
        Line::from(vec![Span::styled(
            msg.clone(),
            Style::default().fg(theme.accent),
        )])
    } else {
        Line::from(vec![Span::styled(
            format!("Kollekció: {}", state.collection),
            Style::default().fg(theme.muted),
        )])
    };

    let status = Paragraph::new(status_text);
    frame.render_widget(status, area);
}
