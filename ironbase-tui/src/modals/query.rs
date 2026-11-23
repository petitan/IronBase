//! Query builder modal - build and execute queries

use super::render_modal_frame;
use crate::app::{QueryState, QUERY_TEMPLATES};
use crate::i18n::t;
use crate::theme::Theme;
use crate::widgets::text_area::plain;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};

/// Render the query builder modal
pub fn render(frame: &mut Frame, area: Rect, state: &QueryState, theme: &Theme) {
    let inner = render_modal_frame(frame, area, t().query_title(), theme, 75, 70);

    // Split into sections
    let chunks = Layout::vertical([
        Constraint::Length(2), // Help text
        Constraint::Length(8), // Query editor (fixed)
        Constraint::Min(10),   // Results list (grows)
        Constraint::Length(1), // Status/error
    ])
    .split(inner);

    // Help text - include Explain button
    let help = Paragraph::new(Line::from(vec![
        Span::styled("^S", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}  ", t().key_run())),
        Span::styled("^E", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}  ", t().key_plan())),
        Span::styled("^j/k", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}  ", t().key_navigate())),
        Span::styled("^T", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}  ", t().key_templates())),
        Span::styled("Esc", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}", t().key_close())),
    ]))
    .style(Style::default().fg(theme.muted));
    frame.render_widget(help, chunks[0]);

    // Query editor area
    render_editor(frame, chunks[1], state, theme);

    // Results or Explain panel
    if state.show_explain && state.explain_result.is_some() {
        render_explain(frame, chunks[2], state, theme);
    } else {
        render_results(frame, chunks[2], state, theme);
    }

    // Status/error line
    render_status(frame, chunks[3], state, theme);

    // Template selector overlay
    if state.show_templates {
        render_templates(frame, area, state, theme);
    }
}

/// Render template selector overlay
fn render_templates(frame: &mut Frame, area: Rect, state: &QueryState, theme: &Theme) {
    // Calculate popup size
    let popup_width = 50u16;
    let popup_height = (QUERY_TEMPLATES.len() + 2) as u16;

    let popup_area = Rect {
        x: area.x + (area.width.saturating_sub(popup_width)) / 2,
        y: area.y + (area.height.saturating_sub(popup_height)) / 2,
        width: popup_width.min(area.width),
        height: popup_height.min(area.height),
    };

    // Clear background
    frame.render_widget(Clear, popup_area);

    // Build list items
    let items: Vec<ListItem> = QUERY_TEMPLATES
        .iter()
        .enumerate()
        .map(|(idx, (kind, template))| {
            let is_selected = idx == state.template_index;
            let style = if is_selected {
                Style::default().fg(theme.bg).bg(theme.accent)
            } else {
                Style::default().fg(theme.fg)
            };

            let line = Line::from(vec![
                Span::styled(format!("{:<14}", kind.name()), style.bold()),
                Span::styled(" ", style),
                Span::styled(truncate(template, 30), style),
            ]);

            ListItem::new(line)
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .title(format!(" {} [j/k] [Enter] ", t().key_templates()))
            .title_style(Style::default().fg(theme.accent).bold())
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.accent))
            .style(Style::default().bg(theme.bg)),
    );

    frame.render_widget(list, popup_area);
}

/// The top border of the plan box, `title` in it: as wide as the bottom border
fn box_top(title: &str) -> String {
    const WIDTH: usize = 48;
    let used = 4 + title.chars().count();
    format!(
        "┌─ {} {}┐",
        title,
        "─".repeat(WIDTH.saturating_sub(used + 1))
    )
}

/// Truncate string for display
fn truncate(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else {
        let chars: String = s.chars().take(max_len.saturating_sub(1)).collect();
        format!("{}…", chars)
    }
}

fn render_editor(frame: &mut Frame, area: Rect, state: &QueryState, theme: &Theme) {
    state.editor.render(
        frame,
        area,
        Style::default().bg(theme.bg),
        Some(Style::default().fg(theme.muted)),
        &plain(Style::default().fg(theme.fg)),
        // The template list is drawn over the editor
        !state.show_templates,
    );
}

fn render_results(frame: &mut Frame, area: Rect, state: &QueryState, theme: &Theme) {
    let visible_height = area.height.saturating_sub(1) as usize; // -1 for header

    if let Some(ref results) = state.results {
        let count = results.len();
        let total = state.total_count;

        // Header with count
        let header_text = if count > 0 {
            t().query_results_header(count, total)
        } else {
            t().no_matches().to_string()
        };

        let mut lines: Vec<Line> = vec![Line::from(Span::styled(
            header_text,
            Style::default().fg(theme.accent),
        ))];

        if count > 0 {
            // Calculate scroll offset to keep selection visible
            let scroll = if state.result_index >= visible_height {
                state.result_index - visible_height + 1
            } else {
                0
            };

            for (idx, doc) in results.iter().enumerate().skip(scroll).take(visible_height) {
                let is_selected = idx == state.result_index;
                let json_str = serde_json::to_string(doc).unwrap_or_default();
                let max_width = area.width.saturating_sub(4) as usize;
                let display = truncate(&json_str, max_width);

                let style = if is_selected {
                    Style::default().fg(theme.bg).bg(theme.accent)
                } else {
                    Style::default().fg(theme.fg)
                };

                let prefix = if is_selected { "> " } else { "  " };
                lines.push(Line::from(Span::styled(
                    format!("{}{}", prefix, display),
                    style,
                )));
            }
        }

        let results_widget = Paragraph::new(lines);
        frame.render_widget(results_widget, area);
    } else {
        let help = Paragraph::new(vec![
            Line::from(Span::styled(
                format!("{}:", t().label_result()),
                Style::default().fg(theme.accent),
            )),
            Line::from(Span::styled(
                t().query_prompt(),
                Style::default().fg(theme.muted),
            )),
        ]);
        frame.render_widget(help, area);
    }
}

fn render_status(frame: &mut Frame, area: Rect, state: &QueryState, theme: &Theme) {
    let status_text = if let Some(ref err) = state.error {
        Line::from(vec![
            Span::styled(
                format!("{}: ", t().label_error()),
                Style::default().fg(theme.error),
            ),
            Span::styled(err.clone(), Style::default().fg(theme.error)),
        ])
    } else {
        let (line, col) = state.editor.cursor();
        let (line, col) = (line + 1, col + 1);
        Line::from(vec![Span::styled(
            t().query_status(line, col, &state.collection, state.show_explain),
            Style::default().fg(theme.muted),
        )])
    };

    let status = Paragraph::new(status_text);
    frame.render_widget(status, area);
}

/// Render query explain panel
fn render_explain(frame: &mut Frame, area: Rect, state: &QueryState, theme: &Theme) {
    let mut lines: Vec<Line> = vec![Line::from(Span::styled(
        box_top(t().query_plan_title()),
        Style::default().fg(theme.accent).bold(),
    ))];

    if let Some(ref explain) = state.explain_result {
        let plan = &explain.chosen_plan;
        let plan_type = plan.query_plan.as_str();
        let index_name = plan.index_used.as_deref().unwrap_or("-");
        let estimated_rows = explain
            .estimated_rows
            // The planner estimate is a float (e.g. 33.333…); show whole rows
            // like the engine's own selection reason does
            .map(|n| format!("{:.0}", n))
            .unwrap_or_else(|| "-".to_string());
        let stage = plan.stage.as_deref().unwrap_or("-");
        let cost = plan.estimated_cost.as_deref().unwrap_or("-");

        // Plan type line with icon
        let plan_icon = match plan_type {
            "IndexScan" | "SparseIndexScan" | "IndexRangeScan" | "MultiValueScan" => "🔍",
            "CollectionScan" => "📋",
            "CompoundIndexScan" => "🔎",
            _ => "❓",
        };

        let field = |label: &'static str, value: String, style: Style| {
            Line::from(vec![
                Span::styled("│ ", Style::default().fg(theme.accent)),
                Span::styled(format!("{label}: "), Style::default().fg(theme.muted)),
                Span::styled(value, style),
            ])
        };
        let plain = Style::default().fg(theme.fg);

        lines.push(field(
            t().label_plan(),
            format!("{} {}", plan_icon, plan_type),
            plain.bold(),
        ));
        lines.push(field(
            "Index",
            index_name.to_string(),
            if index_name == "-" {
                Style::default().fg(theme.warning)
            } else {
                Style::default().fg(theme.success)
            },
        ));
        lines.push(field(t().label_estimated_rows(), estimated_rows, plain));
        lines.push(field(t().label_stage(), stage.to_string(), plain));
        lines.push(field(t().label_cost(), cost.to_string(), plain));
        if let Some(ref reason) = explain.selection_reason {
            lines.push(field(
                t().label_reason(),
                truncate(reason, area.width.saturating_sub(10) as usize),
                Style::default().fg(theme.muted),
            ));
        }
    } else {
        lines.push(Line::from(vec![
            Span::styled("│ ", Style::default().fg(theme.accent)),
            Span::styled(t().no_query_plan(), Style::default().fg(theme.muted)),
        ]));
    }

    lines.push(Line::from(Span::styled(
        "└──────────────────────────────────────────────┘",
        Style::default().fg(theme.accent),
    )));

    // Toggle hint
    lines.push(Line::from(Span::styled(
        t().query_plan_toggle_hint(),
        Style::default().fg(theme.muted),
    )));

    let explain_widget = Paragraph::new(lines);
    frame.render_widget(explain_widget, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Messages;

    /// The plan box's top border is as wide as its bottom border in either
    /// language, whatever the length of the title
    #[test]
    fn test_plan_box_top_matches_the_bottom_border() {
        let bottom = "└──────────────────────────────────────────────┘";
        for title in [
            crate::i18n::Hu.query_plan_title(),
            crate::i18n::En.query_plan_title(),
        ] {
            let top = box_top(title);
            assert_eq!(top.chars().count(), bottom.chars().count(), "{top}");
            assert!(top.contains(title));
        }
    }

    #[test]
    fn test_truncate_zero_width_does_not_underflow() {
        assert_eq!(truncate("abc", 0), "…");
        assert_eq!(truncate("abc", 2), "a…");
        assert_eq!(truncate("abc", 3), "abc");
    }
}
