//! Visual filter modal - easy query builder with field/operator/value

use super::render_modal_frame;
use crate::app::{FilterFocus, FilterOperator, FilterState};
use crate::theme::Theme;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

/// Render the visual filter modal
pub fn render(frame: &mut Frame, area: Rect, state: &FilterState, theme: &Theme) {
    let inner = render_modal_frame(frame, area, "Szuro", theme, 70, 75);

    // Calculate suggestion height (max 5 visible)
    let suggestion_height = if state.show_suggestions && state.focus == FilterFocus::Field {
        state.filtered_suggestions.len().min(5) as u16 + 2 // +2 for border
    } else {
        0
    };

    // Layout: help, field/op/value row, suggestions, add button, filters list, sort, status
    let chunks = Layout::vertical([
        Constraint::Length(2),                 // Help
        Constraint::Length(3),                 // Field input
        Constraint::Length(suggestion_height), // Suggestions dropdown (dynamic)
        Constraint::Length(3),                 // Operator selector
        Constraint::Length(3),                 // Value input
        Constraint::Length(1),                 // Add button hint
        Constraint::Min(3),                    // Active filters
        Constraint::Length(3),                 // Sort selector
        Constraint::Length(2),                 // Status/result count
    ])
    .split(inner);

    // Help text
    render_help(frame, chunks[0], theme);

    // Field input
    render_field_input(frame, chunks[1], state, theme);

    // Suggestions dropdown (if visible)
    if suggestion_height > 0 {
        render_suggestions(frame, chunks[2], state, theme);
    }

    // Operator selector
    render_operator(frame, chunks[3], state, theme);

    // Value input
    render_value_input(frame, chunks[4], state, theme);

    // Add hint
    render_add_hint(frame, chunks[5], state, theme);

    // Active filters list
    render_filters(frame, chunks[6], state, theme);

    // Sort selector
    render_sort(frame, chunks[7], state, theme);

    // Status
    render_status(frame, chunks[8], state, theme);
}

fn render_help(frame: &mut Frame, area: Rect, theme: &Theme) {
    let help = Paragraph::new(Line::from(vec![
        Span::styled("[↑↓]", Style::default().fg(theme.accent)),
        Span::raw(" Valassz mezot  "),
        Span::styled("[Tab/Enter]", Style::default().fg(theme.accent)),
        Span::raw(" Tovabb  "),
        Span::styled("[F5]", Style::default().fg(theme.accent)),
        Span::raw(" Kereses  "),
        Span::styled("[Esc]", Style::default().fg(theme.accent)),
        Span::raw(" Bezar"),
    ]))
    .style(Style::default().fg(theme.muted));
    frame.render_widget(help, area);
}

fn render_field_input(frame: &mut Frame, area: Rect, state: &FilterState, theme: &Theme) {
    let is_focused = state.focus == FilterFocus::Field;
    let border_color = if is_focused {
        theme.accent
    } else {
        theme.secondary
    };

    // Show field count hint if we have suggestions
    let title = if !state.all_fields.is_empty() {
        format!(" Mezo ({} mezo) [↑↓] ", state.all_fields.len())
    } else {
        " Mezo ".to_string()
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let display = if is_focused {
        // UTF-8 safe slicing using character positions
        let before: String = state.field_input.chars().take(state.field_cursor).collect();
        let after: String = state.field_input.chars().skip(state.field_cursor).collect();
        format!("{}|{}", before, after)
    } else if state.field_input.is_empty() {
        if state.all_fields.is_empty() {
            "pl: name, age, email...".to_string()
        } else {
            "gepelj vagy nyilakkal valassz...".to_string()
        }
    } else {
        state.field_input.clone()
    };

    let style = if state.field_input.is_empty() && !is_focused {
        Style::default().fg(theme.muted)
    } else {
        Style::default().fg(theme.fg)
    };

    let p = Paragraph::new(display).style(style);
    frame.render_widget(p, inner);
}

fn render_suggestions(frame: &mut Frame, area: Rect, state: &FilterState, theme: &Theme) {
    let block = Block::default()
        .borders(Borders::LEFT | Borders::RIGHT | Borders::BOTTOM)
        .border_style(Style::default().fg(theme.accent));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Max 5 visible items with scrolling window
    let max_visible = 5;
    let total = state.filtered_suggestions.len();

    // Calculate scroll window to keep selected item visible
    let start = if state.suggestion_idx < max_visible {
        0
    } else {
        // Keep selected item at bottom of visible window
        state.suggestion_idx.saturating_sub(max_visible - 1)
    };

    let items: Vec<ListItem> = state
        .filtered_suggestions
        .iter()
        .enumerate()
        .skip(start)
        .take(max_visible)
        .map(|(i, field)| {
            let is_selected = i == state.suggestion_idx;
            let style = if is_selected {
                Style::default().fg(theme.bg).bg(theme.accent)
            } else {
                Style::default().fg(theme.fg)
            };

            // Show scroll indicators
            let prefix = if is_selected {
                "> "
            } else if i == start && start > 0 {
                "↑ "
            } else if i == start + max_visible - 1 && start + max_visible < total {
                "↓ "
            } else {
                "  "
            };
            ListItem::new(format!("{}{}", prefix, field)).style(style)
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, inner);
}

fn render_operator(frame: &mut Frame, area: Rect, state: &FilterState, theme: &Theme) {
    let is_focused = state.focus == FilterFocus::Operator;
    let border_color = if is_focused {
        theme.accent
    } else {
        theme.secondary
    };

    let block = Block::default()
        .title(" Operator [</>] ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Show all operators with current highlighted
    let ops = FilterOperator::all();
    let spans: Vec<Span> = ops
        .iter()
        .enumerate()
        .flat_map(|(i, op)| {
            let is_selected = i == state.operator_idx;
            let style = if is_selected {
                Style::default().fg(theme.bg).bg(theme.accent)
            } else {
                Style::default().fg(theme.muted)
            };
            vec![
                Span::styled(format!(" {} ", op.label()), style),
                Span::raw(" "),
            ]
        })
        .collect();

    let p = Paragraph::new(Line::from(spans));
    frame.render_widget(p, inner);
}

fn render_value_input(frame: &mut Frame, area: Rect, state: &FilterState, theme: &Theme) {
    let is_focused = state.focus == FilterFocus::Value;
    let border_color = if is_focused {
        theme.accent
    } else {
        theme.secondary
    };

    let block = Block::default()
        .title(" Ertek ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let display = if is_focused {
        // UTF-8 safe slicing using character positions
        let before: String = state.value_input.chars().take(state.value_cursor).collect();
        let after: String = state.value_input.chars().skip(state.value_cursor).collect();
        format!("{}|{}", before, after)
    } else if state.value_input.is_empty() {
        "keresett ertek...".to_string()
    } else {
        state.value_input.clone()
    };

    let style = if state.value_input.is_empty() && !is_focused {
        Style::default().fg(theme.muted)
    } else {
        Style::default().fg(theme.fg)
    };

    let p = Paragraph::new(display).style(style);
    frame.render_widget(p, inner);
}

fn render_add_hint(frame: &mut Frame, area: Rect, state: &FilterState, theme: &Theme) {
    use crate::app::{FilterFocus, FilterOperator};

    let hint = match state.focus {
        FilterFocus::Field if state.field_input.is_empty() => {
            ("Irj be mezonevet (pl: name, age, email)", theme.muted)
        }
        FilterFocus::Field => ("Enter/Tab: tovabb az Operator-ra", theme.accent),
        FilterFocus::Operator => ("<-/-> valassz operatort, Enter/Tab: tovabb", theme.accent),
        FilterFocus::Value if state.operator == FilterOperator::Exists => {
            ("Enter: feltetel hozzaadasa (ertek nem kell)", theme.accent)
        }
        FilterFocus::Value if state.value_input.is_empty() => (
            "Irj be keresett erteket, vagy F5 az azonnali kereseshez",
            theme.muted,
        ),
        FilterFocus::Value => (
            "Enter: feltetel hozzaadasa | F5: azonnali kereses",
            theme.accent,
        ),
        FilterFocus::Filters => (
            "Enter: szerk | Del: torol | Ctrl+↑↓: mozgat | F5: kereses",
            theme.accent,
        ),
        FilterFocus::SortField => (
            "←/→: mezo valasztas | Space: irany | F5: kereses",
            theme.accent,
        ),
    };

    let p = Paragraph::new(hint.0)
        .style(Style::default().fg(hint.1))
        .alignment(Alignment::Center);
    frame.render_widget(p, area);
}

fn render_filters(frame: &mut Frame, area: Rect, state: &FilterState, theme: &Theme) {
    let is_focused = state.focus == FilterFocus::Filters;
    let border_color = if is_focused {
        theme.accent
    } else {
        theme.secondary
    };

    let title = format!(" Aktiv szurok ({}) ", state.filters.len());
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if state.filters.is_empty() {
        let msg = Paragraph::new("Nincs aktiv szuro - adj hozza egyet!")
            .style(Style::default().fg(theme.muted))
            .alignment(Alignment::Center);
        frame.render_widget(msg, inner);
        return;
    }

    let items: Vec<ListItem> = state
        .filters
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let is_selected = i == state.selected_filter && is_focused;
            let style = if is_selected {
                Style::default().fg(theme.bg).bg(theme.accent)
            } else {
                Style::default().fg(theme.fg)
            };

            let prefix = if is_selected { "> " } else { "  " };
            let text = format!("{}{}", prefix, f.display());
            ListItem::new(text).style(style)
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, inner);
}

fn render_sort(frame: &mut Frame, area: Rect, state: &FilterState, theme: &Theme) {
    let is_focused = state.focus == FilterFocus::SortField;
    let border_color = if is_focused {
        theme.accent
    } else {
        theme.secondary
    };

    let block = Block::default()
        .title(" Rendezés [←/→ Space] ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let sort_text = match &state.sort_field {
        Some(field) => format!("{} {}", field, state.sort_direction.label()),
        None => "Nincs rendezés".to_string(),
    };

    let style = if is_focused {
        Style::default().fg(theme.accent)
    } else if state.sort_field.is_some() {
        Style::default().fg(theme.fg)
    } else {
        Style::default().fg(theme.muted)
    };

    let p = Paragraph::new(sort_text).style(style);
    frame.render_widget(p, inner);
}

fn render_status(frame: &mut Frame, area: Rect, state: &FilterState, theme: &Theme) {
    let text = if let Some(ref err) = state.error {
        Line::from(vec![
            Span::styled("Hiba: ", Style::default().fg(theme.error)),
            Span::styled(err.clone(), Style::default().fg(theme.error)),
        ])
    } else if state.result_count > 0 {
        Line::from(vec![Span::styled(
            format!("{} talalat", state.result_count),
            Style::default().fg(theme.accent),
        )])
    } else if !state.filters.is_empty() {
        Line::from(Span::styled(
            "Nyomd meg F5-t a kereséshez",
            Style::default().fg(theme.muted),
        ))
    } else {
        Line::from(Span::styled(
            "Adj hozza szuroket",
            Style::default().fg(theme.muted),
        ))
    };

    let p = Paragraph::new(text);
    frame.render_widget(p, area);
}
