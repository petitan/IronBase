//! Query builder modal - build and execute queries

use super::render_modal_frame;
use crate::app::{QueryState, QUERY_TEMPLATES};
use crate::theme::Theme;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};

/// Render the query builder modal
pub fn render(frame: &mut Frame, area: Rect, state: &QueryState, theme: &Theme) {
    let inner = render_modal_frame(frame, area, "Query Epito", theme, 75, 70);

    // Split into sections
    let chunks = Layout::vertical([
        Constraint::Length(2),  // Help text
        Constraint::Length(8),  // Query editor (fixed)
        Constraint::Min(10),    // Results list (grows)
        Constraint::Length(1),  // Status/error
    ])
    .split(inner);

    // Help text - include Explain button
    let help = Paragraph::new(Line::from(vec![
        Span::styled("^S", Style::default().fg(theme.accent)),
        Span::raw(" Futtat  "),
        Span::styled("^E", Style::default().fg(theme.accent)),
        Span::raw(" Explain  "),
        Span::styled("^j/k", Style::default().fg(theme.accent)),
        Span::raw(" Nav  "),
        Span::styled("^T", Style::default().fg(theme.accent)),
        Span::raw(" Tpl  "),
        Span::styled("Esc", Style::default().fg(theme.accent)),
        Span::raw(" Bezar"),
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
        .map(|(idx, (name, template))| {
            let is_selected = idx == state.template_index;
            let style = if is_selected {
                Style::default().fg(theme.bg).bg(theme.accent)
            } else {
                Style::default().fg(theme.fg)
            };

            let line = Line::from(vec![
                Span::styled(format!("{:<14}", name), style.bold()),
                Span::styled(" ", style),
                Span::styled(truncate(template, 30), style),
            ]);

            ListItem::new(line)
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .title(" Sablonok [j/k] [Enter] ")
            .title_style(Style::default().fg(theme.accent).bold())
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.accent))
            .style(Style::default().bg(theme.bg)),
    );

    frame.render_widget(list, popup_area);
}

/// Truncate string for display
fn truncate(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else {
        let chars: String = s.chars().take(max_len - 1).collect();
        format!("{}…", chars)
    }
}

fn render_editor(frame: &mut Frame, area: Rect, state: &QueryState, theme: &Theme) {
    let visible_height = area.height as usize;

    // Build display lines with cursor
    let mut lines: Vec<Line> = Vec::new();

    for (line_idx, line) in state.lines.iter().enumerate() {
        let is_cursor_line = line_idx == state.cursor_line;

        if is_cursor_line {
            // Line with cursor - UTF-8 safe slicing
            let char_count = line.chars().count();
            let before: String = line.chars().take(state.cursor_col).collect();
            let after: String = if state.cursor_col < char_count {
                line.chars().skip(state.cursor_col).collect()
            } else {
                String::new()
            };

            let line_num = format!("{:2} ", line_idx + 1);
            lines.push(Line::from(vec![
                Span::styled(line_num, Style::default().fg(theme.muted)),
                Span::styled(before, Style::default().fg(theme.fg)),
                Span::styled("|", Style::default().fg(theme.accent)), // Cursor
                Span::styled(after, Style::default().fg(theme.fg)),
            ]));
        } else {
            let line_num = format!("{:2} ", line_idx + 1);
            lines.push(Line::from(vec![
                Span::styled(line_num, Style::default().fg(theme.muted)),
                Span::styled(line.clone(), Style::default().fg(theme.fg)),
            ]));
        }
    }

    // Handle scroll
    let start_line = if state.cursor_line >= visible_height {
        state.cursor_line - visible_height + 1
    } else {
        0
    };

    let visible_lines: Vec<Line> = lines
        .into_iter()
        .skip(start_line)
        .take(visible_height)
        .collect();

    let editor = Paragraph::new(visible_lines).style(Style::default().bg(theme.bg));

    frame.render_widget(editor, area);
}

fn render_results(frame: &mut Frame, area: Rect, state: &QueryState, theme: &Theme) {
    let visible_height = area.height.saturating_sub(1) as usize; // -1 for header

    if let Some(ref results) = state.results {
        let count = results.len();
        let total = state.total_count;

        // Header with count
        let header_text = if total > count {
            format!("Talalatok ({}/{}) [^j/k] [Enter=Szuro]:", count, total)
        } else if count > 0 {
            format!("Talalatok ({}) [^j/k] [Enter=Szuro]:", count)
        } else {
            "Nincs talalat".to_string()
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
            Line::from(Span::styled("Eredmeny:", Style::default().fg(theme.accent))),
            Line::from(Span::styled(
                "Irj query-t es nyomd meg ^S (Ctrl+S)",
                Style::default().fg(theme.muted),
            )),
        ]);
        frame.render_widget(help, area);
    }
}

fn render_status(frame: &mut Frame, area: Rect, state: &QueryState, theme: &Theme) {
    let status_text = if let Some(ref err) = state.error {
        Line::from(vec![
            Span::styled("Hiba: ", Style::default().fg(theme.error)),
            Span::styled(err.clone(), Style::default().fg(theme.error)),
        ])
    } else {
        let (line, col) = (state.cursor_line + 1, state.cursor_col + 1);
        let view_indicator = if state.show_explain { " [Explain]" } else { "" };
        Line::from(vec![Span::styled(
            format!(
                "Sor: {} Oszlop: {} | Kollekció: {}{}",
                line, col, state.collection, view_indicator
            ),
            Style::default().fg(theme.muted),
        )])
    };

    let status = Paragraph::new(status_text);
    frame.render_widget(status, area);
}

/// Render query explain panel
fn render_explain(frame: &mut Frame, area: Rect, state: &QueryState, theme: &Theme) {
    let mut lines: Vec<Line> = vec![
        Line::from(Span::styled(
            "┌─ Query Plan ─────────────────────────────────┐",
            Style::default().fg(theme.accent).bold(),
        )),
    ];

    if let Some(ref explain) = state.explain_result {
        // Extract plan details
        let plan_type = explain
            .get("plan_type")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown");

        let index_name = explain
            .get("index_name")
            .and_then(|v| v.as_str())
            .unwrap_or("-");

        let estimated_rows = explain
            .get("estimated_rows")
            .and_then(|v| v.as_u64())
            .map(|n| n.to_string())
            .unwrap_or_else(|| "?".to_string());

        let selectivity = explain
            .get("selectivity")
            .and_then(|v| v.as_f64())
            .map(|s| format!("{:.4}", s))
            .unwrap_or_else(|| "?".to_string());

        let uses_mcv = explain
            .get("uses_mcv")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let uses_histogram = explain
            .get("uses_histogram")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // Plan type line with icon
        let plan_icon = match plan_type {
            "IndexScan" | "SparseIndexScan" => "🔍",
            "CollectionScan" => "📋",
            "CompoundIndexScan" => "🔎",
            _ => "❓",
        };

        lines.push(Line::from(vec![
            Span::styled("│ ", Style::default().fg(theme.accent)),
            Span::styled("Plan: ", Style::default().fg(theme.muted)),
            Span::styled(
                format!("{} {}", plan_icon, plan_type),
                Style::default().fg(theme.fg).bold(),
            ),
        ]));

        lines.push(Line::from(vec![
            Span::styled("│ ", Style::default().fg(theme.accent)),
            Span::styled("Index: ", Style::default().fg(theme.muted)),
            Span::styled(
                index_name,
                if index_name == "-" {
                    Style::default().fg(theme.warning)
                } else {
                    Style::default().fg(theme.success)
                },
            ),
        ]));

        lines.push(Line::from(vec![
            Span::styled("│ ", Style::default().fg(theme.accent)),
            Span::styled("Est. rows: ", Style::default().fg(theme.muted)),
            Span::styled(estimated_rows, Style::default().fg(theme.fg)),
        ]));

        lines.push(Line::from(vec![
            Span::styled("│ ", Style::default().fg(theme.accent)),
            Span::styled("Selectivity: ", Style::default().fg(theme.muted)),
            Span::styled(selectivity, Style::default().fg(theme.fg)),
        ]));

        // Statistics usage
        let stats_info = match (uses_mcv, uses_histogram) {
            (true, true) => "MCV + Histogram",
            (true, false) => "MCV",
            (false, true) => "Histogram",
            (false, false) => "Basic",
        };

        lines.push(Line::from(vec![
            Span::styled("│ ", Style::default().fg(theme.accent)),
            Span::styled("Statistics: ", Style::default().fg(theme.muted)),
            Span::styled(stats_info, Style::default().fg(theme.fg)),
        ]));

        // Show raw JSON if available (for debugging)
        if let Some(raw) = explain.get("raw") {
            let raw_str = serde_json::to_string_pretty(raw).unwrap_or_default();
            lines.push(Line::from(Span::styled(
                "│",
                Style::default().fg(theme.accent),
            )));
            lines.push(Line::from(vec![
                Span::styled("│ ", Style::default().fg(theme.accent)),
                Span::styled("Raw: ", Style::default().fg(theme.muted)),
            ]));
            for raw_line in raw_str.lines().take(5) {
                lines.push(Line::from(vec![
                    Span::styled("│   ", Style::default().fg(theme.accent)),
                    Span::styled(
                        truncate(raw_line, area.width.saturating_sub(6) as usize),
                        Style::default().fg(theme.muted),
                    ),
                ]));
            }
        }
    } else {
        lines.push(Line::from(vec![
            Span::styled("│ ", Style::default().fg(theme.accent)),
            Span::styled(
                "Nincs explain eredmény. Nyomj ^E-t.",
                Style::default().fg(theme.muted),
            ),
        ]));
    }

    lines.push(Line::from(Span::styled(
        "└──────────────────────────────────────────────┘",
        Style::default().fg(theme.accent),
    )));

    // Toggle hint
    lines.push(Line::from(Span::styled(
        "[^E] Váltás eredmények/explain között",
        Style::default().fg(theme.muted),
    )));

    let explain_widget = Paragraph::new(lines);
    frame.render_widget(explain_widget, area);
}
