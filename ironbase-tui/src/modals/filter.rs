//! Visual filter modal - easy query builder with field/operator/value

use super::render_modal_frame;
use crate::app::{FilterFocus, FilterOperator, FilterState};
use crate::i18n::t;
use crate::theme::Theme;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use unicode_width::UnicodeWidthStr;

/// Render the visual filter modal
pub fn render(frame: &mut Frame, area: Rect, state: &FilterState, theme: &Theme) {
    let inner = render_modal_frame(frame, area, t().filter_title(), theme, 70, 75);

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
        Span::raw(format!(" {}  ", t().key_choose_field())),
        Span::styled("[Tab/Enter]", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}  ", t().key_next())),
        Span::styled("[^S]", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}  ", t().key_search())),
        Span::styled("[Esc]", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}", t().key_close())),
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
    let title = t().field_title(state.all_fields.len());

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let placeholder = if state.all_fields.is_empty() {
        t().field_placeholder_examples()
    } else {
        t().field_placeholder_pick()
    };
    state.field_input.render_with_placeholder(
        frame,
        inner,
        Style::default().fg(theme.fg),
        Span::styled(placeholder, Style::default().fg(theme.muted)),
        is_focused,
    );
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
        .title(format!(" {} [</>] ", t().label_operator()))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Show the operators with the current one highlighted; a row wider than
    // the field (longer translated labels, a narrow terminal) starts later, so
    // the selected operator is always visible
    let ops = FilterOperator::all();
    let widths: Vec<usize> = ops
        .iter()
        .map(|op| op.label().width() + 3) // " label " and the separator
        .collect();
    let first = first_visible(&widths, state.operator_idx, inner.width as usize);
    let spans: Vec<Span> = ops
        .iter()
        .enumerate()
        .skip(first)
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

/// The first item to draw so that item `selected` fits in `available`
/// columns, given each item's width: 0 when everything up to it fits
fn first_visible(widths: &[usize], selected: usize, available: usize) -> usize {
    let selected = selected.min(widths.len().saturating_sub(1));
    let mut first = 0;
    while first < selected && widths[first..=selected].iter().sum::<usize>() > available {
        first += 1;
    }
    first
}

fn render_value_input(frame: &mut Frame, area: Rect, state: &FilterState, theme: &Theme) {
    let is_focused = state.focus == FilterFocus::Value;
    let border_color = if is_focused {
        theme.accent
    } else {
        theme.secondary
    };

    let block = Block::default()
        .title(format!(" {} ", t().label_value()))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    state.value_input.render_with_placeholder(
        frame,
        inner,
        Style::default().fg(theme.fg),
        Span::styled(t().value_placeholder(), Style::default().fg(theme.muted)),
        is_focused,
    );
}

fn render_add_hint(frame: &mut Frame, area: Rect, state: &FilterState, theme: &Theme) {
    use crate::app::{FilterFocus, FilterOperator};

    let hint = match state.focus {
        FilterFocus::Field if state.field_input.is_empty() => (t().hint_type_field(), theme.muted),
        FilterFocus::Field => (t().hint_field_next(), theme.accent),
        FilterFocus::Operator => (t().hint_operator(), theme.accent),
        FilterFocus::Value if state.operator == FilterOperator::Exists => {
            (t().hint_add_exists(), theme.accent)
        }
        FilterFocus::Value if state.value_input.is_empty() => (t().hint_type_value(), theme.muted),
        FilterFocus::Value => (t().hint_add_condition(), theme.accent),
        FilterFocus::Filters => (t().hint_filters(), theme.accent),
        FilterFocus::SortField => (t().hint_sort(), theme.accent),
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

    let title = t().active_filters_title(state.filters.len());
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if state.filters.is_empty() {
        let msg = Paragraph::new(t().no_active_filters())
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
        .title(format!(" {} [←/→ Space] ", t().label_sort()))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let sort_text = match &state.sort_field {
        Some(field) => format!("{} {}", field, state.sort_direction.label()),
        None => t().no_sort().to_string(),
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
            Span::styled(
                format!("{}: ", t().label_error()),
                Style::default().fg(theme.error),
            ),
            Span::styled(err.clone(), Style::default().fg(theme.error)),
        ])
    } else if state.result_count > 0 {
        Line::from(vec![Span::styled(
            t().match_count(state.result_count),
            Style::default().fg(theme.accent),
        )])
    } else if !state.filters.is_empty() {
        Line::from(Span::styled(
            t().press_ctrl_s_to_search(),
            Style::default().fg(theme.muted),
        ))
    } else {
        Line::from(Span::styled(
            t().add_filters(),
            Style::default().fg(theme.muted),
        ))
    };

    let p = Paragraph::new(text);
    frame.render_widget(p, area);
}

#[cfg(test)]
mod tests {
    use super::first_visible;

    #[test]
    fn test_operator_row_keeps_the_selected_operator_visible() {
        let widths = [4, 4, 14, 17, 10, 11];
        // Everything up to the selected one fits: from the start
        assert_eq!(first_visible(&widths, 2, 40), 0);
        // The last one does not fit from the start: start later, as little as needed
        let first = first_visible(&widths, 5, 40);
        assert!(widths[first..=5].iter().sum::<usize>() <= 40);
        assert!(widths[first - 1..=5].iter().sum::<usize>() > 40);
        // Wider than the field by itself: it is drawn first (and clipped)
        assert_eq!(first_visible(&widths, 3, 5), 3);
        assert_eq!(first_visible(&[], 0, 10), 0);
    }
}
