//! Vector search modal - create indexes and perform similarity search

use super::render_modal_frame;
use crate::i18n::t;
use crate::state::vector::{DistanceMetric, VectorSearchState, VectorTab};
use crate::theme::Theme;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Tabs};

/// Render the vector search modal
pub fn render(frame: &mut Frame, area: Rect, state: &VectorSearchState, theme: &Theme) {
    let inner = render_modal_frame(frame, area, t().vector_search_title(), theme, 80, 85);

    // Split into sections
    let chunks = Layout::vertical([
        Constraint::Length(3), // Tabs
        Constraint::Min(10),   // Tab content
        Constraint::Length(1), // Status/error
    ])
    .split(inner);

    // Render tabs
    render_tabs(frame, chunks[0], state, theme);

    // Render tab content
    match state.active_tab {
        VectorTab::Search => render_search_tab(frame, chunks[1], state, theme),
        VectorTab::CreateIndex => render_create_tab(frame, chunks[1], state, theme),
        VectorTab::ListIndexes => render_list_tab(frame, chunks[1], state, theme),
    }

    // Status/error line
    render_status(frame, chunks[2], state, theme);
}

fn render_tabs(frame: &mut Frame, area: Rect, state: &VectorSearchState, theme: &Theme) {
    let titles = vec![
        t().key_search(),
        t().tab_create_index(),
        t().tab_list_indexes(),
    ];
    let selected = match state.active_tab {
        VectorTab::Search => 0,
        VectorTab::CreateIndex => 1,
        VectorTab::ListIndexes => 2,
    };

    let tabs = Tabs::new(titles)
        .block(Block::default().borders(Borders::BOTTOM))
        .select(selected)
        .style(Style::default().fg(theme.muted))
        .highlight_style(
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        );

    frame.render_widget(tabs, area);
}

fn render_search_tab(frame: &mut Frame, area: Rect, state: &VectorSearchState, theme: &Theme) {
    let chunks = Layout::vertical([
        Constraint::Length(2), // Help
        Constraint::Length(3), // Field + K
        Constraint::Length(3), // Vector input
        Constraint::Length(3), // Filter input
        Constraint::Min(4),    // Results
    ])
    .split(area);

    // Help text
    let help = Paragraph::new(Line::from(vec![
        Span::styled("[Tab]", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}  ", t().key_tabs())),
        Span::styled("[Enter]", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}  ", t().key_search())),
        Span::styled("[j/k]", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}  ", t().key_navigate())),
        Span::styled("[Esc]", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}", t().key_close())),
    ]))
    .style(Style::default().fg(theme.muted));
    frame.render_widget(help, chunks[0]);

    // Field and K
    let field_k_chunks =
        Layout::horizontal([Constraint::Percentage(70), Constraint::Percentage(30)])
            .split(chunks[1]);

    let field_block = Block::default()
        .title(format!(" {} ", t().label_field()))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.muted));
    let field_text = Paragraph::new(&*state.search_field)
        .block(field_block)
        .style(Style::default().fg(theme.fg));
    frame.render_widget(field_text, field_k_chunks[0]);

    let k_block = Block::default()
        .title(format!(" {} ", t().label_k_limit()))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.muted));
    let k_text = Paragraph::new(state.k.to_string())
        .block(k_block)
        .style(Style::default().fg(theme.accent));
    frame.render_widget(k_text, field_k_chunks[1]);

    // Vector input
    let vector_block = Block::default()
        .title(format!(" {} ", t().label_vector_json()))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.accent));
    let vector_inner = vector_block.inner(chunks[2]);
    frame.render_widget(vector_block, chunks[2]);
    if state.vector_input.is_empty() {
        frame.render_widget(
            Paragraph::new("[1.0, 2.0, 3.0, ...]").style(Style::default().fg(theme.muted)),
            vector_inner,
        );
        frame.set_cursor_position(Position::new(vector_inner.x, vector_inner.y));
    } else {
        // A long vector scrolls: the cursor stays in view
        state
            .vector_input
            .render(frame, vector_inner, Style::default().fg(theme.fg), true);
    }

    // Filter input
    let filter_block = Block::default()
        .title(format!(" {} ", t().label_filter_optional()))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.muted));
    let filter_preview = if state.filter_input.is_empty() {
        r#"{"category": "science"}"#.to_string()
    } else {
        state.filter_input.clone()
    };
    let filter_text = Paragraph::new(filter_preview)
        .block(filter_block)
        .style(Style::default().fg(theme.muted));
    frame.render_widget(filter_text, chunks[3]);

    // Results
    render_results(frame, chunks[4], state, theme);
}

fn render_results(frame: &mut Frame, area: Rect, state: &VectorSearchState, theme: &Theme) {
    let title = t().vector_results_title(state.results.len(), state.is_loading);

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.muted));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if state.results.is_empty() {
        let msg = if state.is_loading {
            t().searching()
        } else {
            t().no_vector_results()
        };
        let empty = Paragraph::new(msg).style(Style::default().fg(theme.muted));
        frame.render_widget(empty, inner);
        return;
    }

    let items: Vec<ListItem> = state
        .results
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let style = if i == state.selected_result {
                Style::default().fg(theme.bg).bg(theme.accent)
            } else {
                Style::default().fg(theme.fg)
            };

            // Format: similarity score + document preview
            let id = r
                .document
                .get("_id")
                .map(|v| format!("{}", v))
                .unwrap_or_else(|| "?".to_string());
            let doc_preview = format!("id:{}", id);

            // No usable score: NaN / infinite, or the document's own `_score`
            let score = r
                .score
                .map_or_else(|| format!("{:>6}", "n/a"), |s| format!("{s:.4}"));
            let text = format!("  {}  {}", score, doc_preview);
            ListItem::new(text).style(style)
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, inner);
}

fn render_create_tab(frame: &mut Frame, area: Rect, state: &VectorSearchState, theme: &Theme) {
    let chunks = Layout::vertical([
        Constraint::Length(2), // Help
        Constraint::Length(3), // Field
        Constraint::Length(3), // Dimension
        Constraint::Length(3), // Metric
        Constraint::Min(2),    // Hint
    ])
    .split(area);

    // Help text
    let help = Paragraph::new(Line::from(vec![
        Span::styled("[Tab]", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}  ", t().key_next_field())),
        Span::styled("[Space]", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}  ", t().key_toggle_metric())),
        Span::styled("[Enter]", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}  ", t().key_create())),
        Span::styled("[Esc]", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}", t().key_cancel())),
    ]))
    .style(Style::default().fg(theme.muted));
    frame.render_widget(help, chunks[0]);

    // Field input
    let field_style = if state.create_form_field == 0 {
        Style::default().fg(theme.accent)
    } else {
        Style::default().fg(theme.muted)
    };
    let field_block = Block::default()
        .title(format!(" {} ", t().label_field_name()))
        .borders(Borders::ALL)
        .border_style(field_style);
    let field_inner = field_block.inner(chunks[1]);
    frame.render_widget(field_block, chunks[1]);
    state.new_field.render(
        frame,
        field_inner,
        Style::default().fg(theme.fg),
        state.create_form_field == 0,
    );

    // Dimension input
    let dim_style = if state.create_form_field == 1 {
        Style::default().fg(theme.accent)
    } else {
        Style::default().fg(theme.muted)
    };
    let dim_block = Block::default()
        .title(format!(" {} ", t().label_dimension()))
        .borders(Borders::ALL)
        .border_style(dim_style);
    let dim_text = Paragraph::new(state.dimension.to_string())
        .block(dim_block)
        .style(Style::default().fg(theme.fg));
    frame.render_widget(dim_text, chunks[2]);

    // Metric selector
    let metric_style = if state.create_form_field == 2 {
        Style::default().fg(theme.accent)
    } else {
        Style::default().fg(theme.muted)
    };
    let metric_block = Block::default()
        .title(format!(" {} ", t().label_distance_metric()))
        .borders(Borders::ALL)
        .border_style(metric_style);

    let metric_display = match state.metric {
        DistanceMetric::Cosine => t().metric_cosine(),
        DistanceMetric::Euclidean => t().metric_euclidean(),
        DistanceMetric::DotProduct => t().metric_dot_product(),
    };
    let metric_text = Paragraph::new(metric_display)
        .block(metric_block)
        .style(Style::default().fg(theme.fg));
    frame.render_widget(metric_text, chunks[3]);

    // Hint
    let hint = Paragraph::new(t().dimension_tip()).style(Style::default().fg(theme.muted));
    frame.render_widget(hint, chunks[4]);
}

fn render_list_tab(frame: &mut Frame, area: Rect, state: &VectorSearchState, theme: &Theme) {
    let chunks = Layout::vertical([
        Constraint::Length(2), // Help
        Constraint::Min(6),    // Index list
    ])
    .split(area);

    // Help text
    let help = Paragraph::new(Line::from(vec![
        Span::styled("[j/k]", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}  ", t().key_navigate())),
        Span::styled("[d]", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}  ", t().cmd_delete())),
        Span::styled("[Tab]", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}  ", t().key_tabs())),
        Span::styled("[Esc]", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}", t().key_close())),
    ]))
    .style(Style::default().fg(theme.muted));
    frame.render_widget(help, chunks[0]);

    // Index list
    let title = t().vector_indexes_title(state.indexes.len());
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.accent));

    let inner = block.inner(area);
    frame.render_widget(block, chunks[1]);

    if state.indexes.is_empty() {
        let empty = Paragraph::new(t().no_vector_indexes()).style(Style::default().fg(theme.muted));
        frame.render_widget(empty, inner);
        return;
    }

    let items: Vec<ListItem> = state
        .indexes
        .iter()
        .enumerate()
        .map(|(i, idx)| {
            let style = if i == state.selected_index {
                Style::default().fg(theme.bg).bg(theme.accent)
            } else {
                Style::default().fg(theme.fg)
            };

            // Format: name | field | dimension | metric | count
            let text = format!(
                "  {}",
                t().vector_index_row(
                    &idx.name,
                    &idx.field,
                    idx.dimension,
                    &idx.metric.to_string(),
                    idx.vector_count
                )
            );
            ListItem::new(text).style(style)
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, inner);
}

fn render_status(frame: &mut Frame, area: Rect, state: &VectorSearchState, theme: &Theme) {
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
        Line::from(vec![Span::styled(
            t().collection_label(&state.collection),
            Style::default().fg(theme.muted),
        )])
    };

    let status = Paragraph::new(status_text);
    frame.render_widget(status, area);
}
