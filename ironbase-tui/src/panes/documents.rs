//! Documents pane - center panel showing document list in table format

use crate::app::App;
use crate::base64_detect::sanitize_base64_values;
use crate::theme::Theme;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};
use std::collections::HashSet;

/// Render the documents pane (center panel) as a table
pub fn render(frame: &mut Frame, area: Rect, app: &App, theme: &Theme, focused: bool) {
    let border_color = if focused {
        theme.accent
    } else {
        theme.secondary
    };
    let title_style = if focused {
        Style::default().fg(theme.accent).bold()
    } else {
        Style::default().fg(theme.secondary)
    };

    // Title with pagination info and filter indicator
    let (current_page, total_pages) = app.get_pagination_info();
    let filter_indicator = if app.active_filter.is_some() {
        " [SZURT - Esc torol]"
    } else {
        ""
    };
    let title = format!(
        " Dokumentumok ({}/{} oldal) {}",
        current_page, total_pages, filter_indicator
    );

    let block = Block::default()
        .title(title)
        .title_style(title_style)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Get documents
    let documents = app.get_documents();

    if documents.is_empty() {
        let msg = if app.has_collection() {
            "Nincs dokumentum"
        } else {
            "Valassz kollekciot"
        };
        let empty_msg = Paragraph::new(msg)
            .style(Style::default().fg(theme.muted))
            .alignment(Alignment::Center);
        frame.render_widget(empty_msg, inner);
        return;
    }

    // Collect all unique field names (excluding _id, limit to first 6 fields)
    let columns = collect_columns(documents, 6);
    let col_count = columns.len();

    if col_count == 0 {
        return;
    }

    // Calculate column widths
    let available_width = inner.width as usize;
    let id_width = 8; // Fixed width for _id
    let remaining = available_width.saturating_sub(id_width + 1);
    let col_width = remaining.checked_div(col_count).unwrap_or(10).clamp(6, 20);

    // Build header row
    let mut header_cells = vec![Cell::from("_id").style(Style::default().fg(theme.accent).bold())];
    for col in &columns {
        header_cells.push(
            Cell::from(truncate_str(col, col_width))
                .style(Style::default().fg(theme.accent).bold()),
        );
    }
    let header = Row::new(header_cells).height(1);

    // Build data rows
    let local_selected = app.selected_document.saturating_sub(app.doc_scroll_offset);
    let visible_height = inner.height.saturating_sub(1) as usize; // -1 for header

    let rows: Vec<Row> = documents
        .iter()
        .enumerate()
        .take(visible_height)
        .map(|(idx, doc)| {
            let is_selected = idx == local_selected && focused;
            let row_style = if is_selected {
                Style::default().fg(theme.bg).bg(theme.accent)
            } else if idx == local_selected {
                Style::default().fg(theme.accent)
            } else {
                Style::default().fg(theme.fg)
            };

            let sanitized = sanitize_base64_values(doc);

            // _id column
            let id_val = sanitized
                .get("_id")
                .map(format_value)
                .unwrap_or_else(|| "?".to_string());

            let mut cells = vec![Cell::from(truncate_str(&id_val, id_width))];

            // Data columns
            for col in &columns {
                let val = sanitized
                    .get(col)
                    .map(format_value)
                    .unwrap_or_else(|| "-".to_string());
                cells.push(Cell::from(truncate_str(&val, col_width)));
            }

            Row::new(cells).style(row_style)
        })
        .collect();

    // Build constraints
    let mut widths = vec![Constraint::Length(id_width as u16)];
    for _ in &columns {
        widths.push(Constraint::Length(col_width as u16));
    }

    let table = Table::new(rows, widths)
        .header(header)
        .highlight_style(Style::default().fg(theme.bg).bg(theme.accent));

    frame.render_widget(table, inner);
}

/// Collect unique column names from documents (excluding _id)
fn collect_columns(documents: &[serde_json::Value], max_cols: usize) -> Vec<String> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut columns: Vec<String> = Vec::new();

    // Priority fields - show these first if they exist
    let priority = ["name", "title", "email", "username", "type", "status"];

    for doc in documents {
        if let Some(obj) = doc.as_object() {
            for key in obj.keys() {
                if key != "_id" && !seen.contains(key) {
                    seen.insert(key.clone());
                }
            }
        }
    }

    // Add priority fields first
    for p in priority {
        if seen.contains(p) {
            columns.push(p.to_string());
            seen.remove(p);
            if columns.len() >= max_cols {
                return columns;
            }
        }
    }

    // Add remaining fields (sorted alphabetically for stable order)
    let mut remaining: Vec<String> = seen.into_iter().collect();
    remaining.sort();

    for key in remaining {
        columns.push(key);
        if columns.len() >= max_cols {
            break;
        }
    }

    columns
}

/// Format a JSON value for table display
fn format_value(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Null => "-".to_string(),
        serde_json::Value::Bool(b) => if *b { "true" } else { "false" }.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(arr) => format!("[{}]", arr.len()),
        serde_json::Value::Object(obj) => format!("{{{}}}", obj.len()),
    }
}

/// Truncate string to max length with ellipsis
fn truncate_str(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_len.saturating_sub(1)).collect();
        format!("{}…", truncated)
    }
}
