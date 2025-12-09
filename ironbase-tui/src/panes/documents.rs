//! Documents pane - center panel showing document list

use crate::app::App;
use crate::theme::Theme;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

/// Render the documents pane (center panel)
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

    // Title with pagination info
    let (current_page, total_pages) = app.get_pagination_info();
    let title = format!(" Documents (Page {}/{}) ", current_page, total_pages);

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

    let visible_height = inner.height as usize;
    // A documents tömb már az aktuális oldalt tartalmazza (skip/limit-el betöltve)
    // Ezért NEM kell újra skip-elni!
    // selected_document GLOBÁLIS index, a lokális index = selected_document - doc_scroll_offset
    let local_selected = app.selected_document.saturating_sub(app.doc_scroll_offset);

    let items: Vec<ListItem> = documents
        .iter()
        .enumerate()
        .take(visible_height)
        .map(|(idx, doc)| {
            let is_selected = idx == local_selected && focused;
            let style = if is_selected {
                Style::default().fg(theme.bg).bg(theme.accent)
            } else if idx == local_selected {
                Style::default().fg(theme.accent)
            } else {
                Style::default().fg(theme.fg)
            };

            // Extract _id and create preview
            let id = doc
                .get("_id")
                .map(|v| v.to_string())
                .unwrap_or_else(|| "?".to_string());

            let preview = create_preview(doc, inner.width as usize - 10);
            let text = format!("#{} {}", id, preview);

            ListItem::new(text).style(style)
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, inner);
}

/// Create a truncated preview of a document
fn create_preview(doc: &serde_json::Value, max_len: usize) -> String {
    let json_str = serde_json::to_string(doc).unwrap_or_else(|_| "{}".to_string());

    // Remove _id from preview to save space
    let preview = if let Some(obj) = doc.as_object() {
        let filtered: serde_json::Map<String, serde_json::Value> = obj
            .iter()
            .filter(|(k, _)| *k != "_id")
            .take(3)  // Only show first 3 fields
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        serde_json::to_string(&filtered).unwrap_or(json_str)
    } else {
        json_str
    };

    if preview.chars().count() <= max_len {
        preview
    } else {
        // Safe UTF-8 truncation at character boundary
        let truncated: String = preview.chars().take(max_len.saturating_sub(3)).collect();
        format!("{}...", truncated)
    }
}
