//! Documents pane - center panel showing document list in table format

use crate::app::App;
use crate::base64_detect::sanitize_base64_values;
use crate::i18n::t;
use crate::theme::Theme;
use crate::widgets::{selection_mark, truncate_width};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};
use std::collections::HashSet;
use unicode_width::UnicodeWidthStr;

/// Number of document rows the pane draws for a given pane height
/// (borders: 2, table header: 1). Single source of truth for the page size.
pub fn visible_rows(pane_height: u16) -> usize {
    pane_height.saturating_sub(3) as usize
}

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
    let title = t().documents_title(current_page, total_pages, app.active_filter.is_some());

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
            t().no_documents()
        } else {
            t().select_collection()
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

    // Column widths: the values first (each column as wide as its widest
    // value on the page, the width left shared among the wide ones), then
    // what is still left widens the headers that are cut. The table puts
    // one column of space between columns: that is not the cells' width
    let id_width = 8; // Fixed width for _id
    let sanitized: Vec<serde_json::Value> = documents
        .iter()
        .take(visible_rows(area.height))
        .map(sanitize_base64_values)
        .collect();
    let values: Vec<usize> = columns
        .iter()
        .map(|col| {
            sanitized
                .iter()
                .map(|doc| doc.get(col).map(format_value).unwrap_or_else(|| "-".into()))
                .map(|v| UnicodeWidthStr::width(v.as_str()))
                .max()
                .unwrap_or(0)
        })
        .collect();
    let headers: Vec<usize> = columns
        .iter()
        .map(|col| UnicodeWidthStr::width(col.as_str()))
        .collect();
    let available = (inner.width as usize).saturating_sub(id_width + COLUMN_SPACING * col_count);
    let col_widths = widen_headers(fit_widths(&values, available), &headers, available);

    // Build header row
    let mut header_cells = vec![Cell::from("_id").style(Style::default().fg(theme.accent).bold())];
    for (col, &width) in columns.iter().zip(&col_widths) {
        header_cells.push(
            Cell::from(truncate_width(col, width)).style(Style::default().fg(theme.accent).bold()),
        );
    }
    let header = Row::new(header_cells).height(1);

    // Build data rows
    // While the list reloads its rows are the old page: none is the selected one
    let local_selected = if app.documents_loading() {
        usize::MAX
    } else {
        app.selected_document.saturating_sub(app.doc_scroll_offset)
    };
    let visible_height = visible_rows(area.height);

    let rows: Vec<Row> = sanitized
        .iter()
        .enumerate()
        .take(visible_height)
        .map(|(idx, sanitized)| {
            let is_selected = idx == local_selected && focused;
            let row_style = if is_selected {
                Style::default().fg(theme.bg).bg(theme.accent)
            } else if idx == local_selected {
                Style::default().fg(theme.accent)
            } else {
                Style::default().fg(theme.fg)
            };

            // _id column
            let id_val = sanitized
                .get("_id")
                .map(format_value)
                .unwrap_or_else(|| "?".to_string());

            // The selected row's mark in front of its _id: the selection shows
            // without color too
            let mut cells = vec![Cell::from(format!(
                "{}{}",
                selection_mark(idx == local_selected),
                truncate_width(&id_val, id_width.saturating_sub(2))
            ))];

            // Data columns
            for (col, &width) in columns.iter().zip(&col_widths) {
                let val = sanitized
                    .get(col)
                    .map(format_value)
                    .unwrap_or_else(|| "-".to_string());
                cells.push(Cell::from(truncate_width(&val, width)));
            }

            Row::new(cells).style(row_style)
        })
        .collect();

    // Build constraints
    let mut widths = vec![Constraint::Length(id_width as u16)];
    for &width in &col_widths {
        widths.push(Constraint::Length(width as u16));
    }

    let table = Table::new(rows, widths)
        .column_spacing(COLUMN_SPACING as u16)
        .header(header)
        .row_highlight_style(Style::default().fg(theme.bg).bg(theme.accent));

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

/// Space between two table columns
const COLUMN_SPACING: usize = 1;

/// Narrowest a column gets (a cut value still shows a character and `…`)
const MIN_COLUMN_WIDTH: usize = 3;

/// Widest a column gets even with room to spare (a long text does not
/// push the other columns out)
const MAX_COLUMN_WIDTH: usize = 40;

/// Share `available` columns among columns that would need `natural` each:
/// a column narrower than its share gets what it needs, the share it leaves
/// goes to the wider ones (which are cut to an equal width)
fn fit_widths(natural: &[usize], available: usize) -> Vec<usize> {
    let mut widths = vec![0; natural.len()];
    let mut order: Vec<usize> = (0..natural.len()).collect();
    order.sort_by_key(|&i| natural[i]);
    let mut left = available;
    for (n, &i) in order.iter().enumerate() {
        let share = left / (natural.len() - n);
        let want = natural[i].clamp(MIN_COLUMN_WIDTH, MAX_COLUMN_WIDTH);
        widths[i] = want.min(share).max(MIN_COLUMN_WIDTH);
        left = left.saturating_sub(widths[i]);
    }
    widths
}

/// Give the columns' `widths` the room left of `available`, as far as their
/// headers need it (in column order)
fn widen_headers(mut widths: Vec<usize>, headers: &[usize], available: usize) -> Vec<usize> {
    let mut left = available.saturating_sub(widths.iter().sum());
    for (width, &header) in widths.iter_mut().zip(headers) {
        let extra = header
            .min(MAX_COLUMN_WIDTH)
            .saturating_sub(*width)
            .min(left);
        *width += extra;
        left -= extra;
    }
    widths
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Narrow columns keep their width, the wide one gets the rest
    #[test]
    fn test_fit_widths_gives_the_rest_to_the_wide_column() {
        // cím (long), címkék [2], elérh(ető), szerző {2}, év
        assert_eq!(
            fit_widths(&[30, 6, 8, 6, 4], 50),
            [50 - 6 - 8 - 6 - 4, 6, 8, 6, 4]
        );
        // All fit: each its own width
        assert_eq!(fit_widths(&[10, 4], 50), [10, 4]);
        // Two wide columns share equally
        assert_eq!(fit_widths(&[30, 30, 4], 30), [13, 13, 4]);
        // Too narrow for anything: the minimum
        assert_eq!(fit_widths(&[30, 30], 2), [3, 3]);
    }

    /// Room left after the values widens the cut headers
    #[test]
    fn test_widen_headers_uses_only_the_room_left() {
        assert_eq!(widen_headers(vec![10, 3, 4], &[3, 6, 8], 20), [10, 6, 4]);
        assert_eq!(widen_headers(vec![10, 3, 4], &[3, 6, 8], 30), [10, 6, 8]);
    }
}
