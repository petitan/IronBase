//! Index management modal - create/delete indexes with statistics

use super::render_modal_frame;
use crate::i18n::t;
use crate::state::index::{IndexState, IndexStatistics};
use crate::theme::Theme;
use crate::widgets::{marked, selection_mark};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

/// Render the index management modal
pub fn render(frame: &mut Frame, area: Rect, state: &IndexState, theme: &Theme) {
    let inner = render_modal_frame(frame, area, t().index_title(), theme, 70, 80);

    // Split into sections
    let chunks = Layout::vertical([
        Constraint::Length(2), // Help text
        Constraint::Min(6),    // Existing indexes list
        Constraint::Length(4), // Statistics for selected index
        Constraint::Length(7), // Create new index form
        Constraint::Length(1), // Status/error
    ])
    .split(inner);

    // Help text
    let help = Paragraph::new(Line::from(vec![
        Span::styled("[j/k]", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}  ", t().key_navigate())),
        Span::styled("[d]", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}  ", t().cmd_delete())),
        Span::styled("[a]", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}  ", t().key_analyze())),
        Span::styled("[Tab]", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}  ", t().key_new())),
        Span::styled("[Esc]", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}", t().key_close())),
    ]))
    .style(Style::default().fg(theme.muted));
    frame.render_widget(help, chunks[0]);

    // Existing indexes list with staleness indicators
    render_index_list(frame, chunks[1], state, theme);

    // Statistics for selected index
    render_statistics(frame, chunks[2], state, theme);

    // Create new index form
    render_create_form(frame, chunks[3], state, theme);

    // Status/error line
    render_status(frame, chunks[4], state, theme);
}

fn render_index_list(frame: &mut Frame, area: Rect, state: &IndexState, theme: &Theme) {
    let title = t().existing_indexes_title(state.is_analyzing);

    let block = Block::default()
        .title(title)
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
            Paragraph::new(t().no_indexes_only_id()).style(Style::default().fg(theme.muted));
        frame.render_widget(empty, inner);
        return;
    }

    let items: Vec<ListItem> = state
        .indexes
        .iter()
        .enumerate()
        .map(|(i, idx)| {
            let is_selected = !state.is_creating && i == state.selected_index;
            let style = if is_selected {
                Style::default().fg(theme.bg).bg(theme.accent)
            } else {
                Style::default().fg(theme.fg)
            };

            // Find entry with type info
            let entry = state.index_entries.iter().find(|e| &e.name == idx);
            let kind_label = entry
                .map(|e| format!("[{}]", e.kind.label()))
                .unwrap_or_default();
            let detail = entry
                .map(|e| {
                    if e.detail.is_empty() {
                        String::new()
                    } else {
                        format!(" {}", e.detail)
                    }
                })
                .unwrap_or_default();

            // Find statistics for this index
            let stats = state.statistics.iter().find(|s| &s.name == idx);

            // Add staleness indicator
            let staleness_icon = match stats.map(|s| s.staleness_hint()) {
                Some("green") => " ●",  // Good
                Some("yellow") => " ◐", // Partial stats
                Some("red") => " ○",    // Needs refresh
                _ => "",
            };

            // Format: "  [B+] index_name field_detail ●  "
            let text = format!("{} {}{}{}", kind_label, idx, detail, staleness_icon);

            // Color the staleness icon
            if is_selected {
                ListItem::new(marked(is_selected, text)).style(style)
            } else {
                let staleness_color = match stats.map(|s| s.staleness_hint()) {
                    Some("green") => theme.accent,
                    Some("yellow") => Color::Yellow,
                    Some("red") => theme.error,
                    _ => theme.muted,
                };

                // Create spans for proper coloring
                let base_text =
                    format!("{}{} {}{}", selection_mark(false), kind_label, idx, detail);
                ListItem::new(Line::from(vec![
                    Span::styled(base_text, style),
                    Span::styled(staleness_icon, Style::default().fg(staleness_color)),
                ]))
            }
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, inner);
}

/// Render statistics for the selected index
fn render_statistics(frame: &mut Frame, area: Rect, state: &IndexState, theme: &Theme) {
    let block = Block::default()
        .title(format!(" {} ", t().index_statistics_title()))
        .title_style(Style::default().fg(theme.fg))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.muted));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if let Some(stats) = state.selected_statistics() {
        render_stats_content(frame, inner, stats, theme);
    } else if state.indexes.is_empty() {
        let msg = Paragraph::new(t().no_index_selected()).style(Style::default().fg(theme.muted));
        frame.render_widget(msg, inner);
    } else {
        let msg =
            Paragraph::new(t().load_statistics_hint()).style(Style::default().fg(theme.muted));
        frame.render_widget(msg, inner);
    }
}

fn render_stats_content(frame: &mut Frame, area: Rect, stats: &IndexStatistics, theme: &Theme) {
    let chunks = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).split(area);

    // Row 1: Keys and distinct count
    let staleness_color = match stats.staleness_hint() {
        "green" => theme.accent,
        "yellow" => Color::Yellow,
        "red" => theme.error,
        _ => theme.muted,
    };

    let staleness_text = match stats.staleness_hint() {
        "green" => "OK",
        "yellow" => t().staleness_partial(),
        "red" => t().staleness_stale(),
        _ => "?",
    };

    let row1 = Line::from(vec![
        Span::raw(format!(" {}: ", t().label_keys())),
        Span::styled(
            format!("{}", stats.num_keys),
            Style::default().fg(theme.accent),
        ),
        Span::raw(format!("  {}: ", t().label_distinct())),
        Span::styled(
            format!("{}", stats.distinct_count),
            Style::default().fg(theme.accent),
        ),
        Span::raw(format!("  {}: ", t().label_status())),
        Span::styled(staleness_text, Style::default().fg(staleness_color)),
    ]);
    frame.render_widget(Paragraph::new(row1), chunks[0]);

    // Row 2: Histogram and MCV status
    let histogram_status = if stats.has_histogram {
        t().yes()
    } else {
        t().no()
    };
    let mcv_status = if stats.has_mcv { t().yes() } else { t().no() };

    let histogram_color = if stats.has_histogram {
        theme.accent
    } else {
        theme.muted
    };
    let mcv_color = if stats.has_mcv {
        theme.accent
    } else {
        theme.muted
    };

    let row2 = Line::from(vec![
        Span::raw(format!(" {}: ", t().label_histogram())),
        Span::styled(histogram_status, Style::default().fg(histogram_color)),
        Span::raw("  MCV: "),
        Span::styled(mcv_status, Style::default().fg(mcv_color)),
        Span::raw(format!("  {}: ", t().label_field())),
        Span::styled(&stats.field, Style::default().fg(theme.fg)),
    ]);
    frame.render_widget(Paragraph::new(row2), chunks[1]);
}

fn render_create_form(frame: &mut Frame, area: Rect, state: &IndexState, theme: &Theme) {
    let block = Block::default()
        .title(format!(" {} ", t().create_index_title()))
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

    let field_label = if state.is_compound {
        format!(" {}: ", t().label_fields_comma())
    } else {
        format!(" {}: ", t().label_field_name())
    };
    state.field_input.render_labeled(
        frame,
        form_chunks[0],
        field_label,
        field_style,
        state.is_creating && state.form_field == 0,
    );

    // Compound checkbox (form_field == 1)
    let compound_style = if state.is_creating && state.form_field == 1 {
        Style::default().fg(theme.accent)
    } else {
        Style::default().fg(theme.muted)
    };

    let compound_checkbox = if state.is_compound { "[X]" } else { "[ ]" };
    let compound_line = Line::from(vec![
        Span::raw(format!(" {}: ", t().label_compound())),
        Span::styled(compound_checkbox, compound_style),
        Span::styled(
            if state.is_compound {
                t().compound_field_count(state.compound_fields.len())
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
        Span::raw(format!(" {}: ", t().label_unique())),
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
        Span::raw(format!(" {}: ", t().label_sparse())),
        Span::styled(sparse_checkbox, sparse_style),
        Span::styled(
            format!(" {}", t().sparse_hint()),
            Style::default().fg(theme.muted),
        ),
    ]);
    frame.render_widget(Paragraph::new(sparse_line), form_chunks[3]);

    // Create hint
    if state.is_creating {
        let hint = Paragraph::new(format!(
            " [Enter] {}  [Tab] {}  [Esc] {}",
            t().key_create(),
            t().key_next(),
            t().key_cancel()
        ))
        .style(Style::default().fg(theme.muted));
        frame.render_widget(hint, form_chunks[4]);
    }
}

fn render_status(frame: &mut Frame, area: Rect, state: &IndexState, theme: &Theme) {
    let status_text = if let Some(ref err) = state.error {
        Line::from(vec![
            Span::styled(
                format!("{}: ", t().label_error()),
                Style::default().fg(theme.error),
            ),
            Span::styled(err.clone(), Style::default().fg(theme.error)),
        ])
    } else if let Some(ref msg) = state.message {
        Line::from(vec![Span::styled(
            msg.clone(),
            Style::default().fg(theme.accent),
        )])
    } else {
        // Show stale count if any
        let stale_count = state.stale_index_count();
        if stale_count > 0 {
            Line::from(vec![
                Span::styled(
                    format!("{}  ", t().collection_label(&state.collection)),
                    Style::default().fg(theme.muted),
                ),
                Span::styled(
                    t().stale_indexes(stale_count),
                    Style::default().fg(Color::Yellow),
                ),
            ])
        } else {
            Line::from(vec![Span::styled(
                t().collection_label(&state.collection),
                Style::default().fg(theme.muted),
            )])
        }
    };

    let status = Paragraph::new(status_text);
    frame.render_widget(status, area);
}
