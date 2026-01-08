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
        Constraint::Length(7), // Create new index form (field + compound + unique + sparse + hint)
        Constraint::Length(1), // Status/error
    ])
    .split(inner);

    // Help text
    let help = Paragraph::new(Line::from(vec![
        Span::styled("[j/k]", Style::default().fg(theme.accent)),
        Span::raw(" Navigacio  "),
        Span::styled("[d]", Style::default().fg(theme.accent)),
        Span::raw(" Torles  "),
        Span::styled("[Tab]", Style::default().fg(theme.accent)),
        Span::raw(" Uj index  "),
        Span::styled("[Esc]", Style::default().fg(theme.accent)),
        Span::raw(" Bezar"),
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

            let mut markers = String::new();
            if idx.contains("unique") { markers.push_str(" [U]"); }
            if idx.contains("sparse") { markers.push_str(" [S]"); }
            ListItem::new(format!("  {}{}  ", idx, markers)).style(style)
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
        Constraint::Length(1), // Compound checkbox
        Constraint::Length(1), // Unique checkbox
        Constraint::Length(1), // Sparse checkbox
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

    let field_label = if state.is_compound {
        " Mezok (vesszovel): "
    } else {
        " Mezo neve: "
    };

    let field_line = Line::from(vec![
        Span::raw(field_label),
        Span::styled(&state.field_input, field_style),
        Span::styled(cursor, Style::default().fg(theme.accent)),
    ]);
    frame.render_widget(Paragraph::new(field_line), form_chunks[0]);

    // Compound checkbox (form_field == 1)
    let compound_style = if state.is_creating && state.form_field == 1 {
        Style::default().fg(theme.accent)
    } else {
        Style::default().fg(theme.muted)
    };

    let compound_checkbox = if state.is_compound { "[X]" } else { "[ ]" };
    let compound_line = Line::from(vec![
        Span::raw(" Compound: "),
        Span::styled(compound_checkbox, compound_style),
        Span::styled(
            if state.is_compound {
                format!(" [{} mezo]", state.compound_fields.len())
            } else {
                String::new()
            },
            Style::default().fg(theme.muted),
        ),
    ]);
    frame.render_widget(Paragraph::new(compound_line), form_chunks[1]);

    // Unique checkbox (form_field == 2)
    let unique_style = if state.is_creating && state.form_field == 2 {
        Style::default().fg(theme.accent)
    } else {
        Style::default().fg(theme.muted)
    };

    let unique_checkbox = if state.unique { "[X]" } else { "[ ]" };
    let unique_line = Line::from(vec![
        Span::raw(" Unique: "),
        Span::styled(unique_checkbox, unique_style),
    ]);
    frame.render_widget(Paragraph::new(unique_line), form_chunks[2]);

    // Sparse checkbox (form_field == 3)
    let sparse_style = if state.is_creating && state.form_field == 3 {
        Style::default().fg(theme.accent)
    } else {
        Style::default().fg(theme.muted)
    };

    let sparse_checkbox = if state.sparse { "[X]" } else { "[ ]" };
    let sparse_line = Line::from(vec![
        Span::raw(" Sparse: "),
        Span::styled(sparse_checkbox, sparse_style),
        Span::styled(" (csak letezo mezok)", Style::default().fg(theme.muted)),
    ]);
    frame.render_widget(Paragraph::new(sparse_line), form_chunks[3]);

    // Create hint
    if state.is_creating {
        let hint = Paragraph::new(" [Enter] Letrehoz  [Tab] Kovetkezo  [Esc] Megse")
            .style(Style::default().fg(theme.muted));
        frame.render_widget(hint, form_chunks[4]);
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
