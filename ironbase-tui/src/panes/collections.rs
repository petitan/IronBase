//! Collections pane - left panel showing collection list and indexes

use crate::app::App;
use crate::theme::Theme;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

/// Render the collections pane (left panel)
pub fn render(frame: &mut Frame, area: Rect, app: &App, theme: &Theme, focused: bool) {
    // Split into collections list and indexes
    let chunks = Layout::vertical([
        Constraint::Min(5),    // Collections list
        Constraint::Length(6), // Indexes section
    ])
    .split(area);

    render_collections_list(frame, chunks[0], app, theme, focused);
    render_indexes(frame, chunks[1], app, theme, focused);
}

fn render_collections_list(frame: &mut Frame, area: Rect, app: &App, theme: &Theme, focused: bool) {
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

    let block = Block::default()
        .title(" Kollekcio ")
        .title_style(title_style)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Get collections
    let collections = app.get_collections();

    if collections.is_empty() {
        let empty_msg = Paragraph::new("Nincs kollekció")
            .style(Style::default().fg(theme.muted))
            .alignment(Alignment::Center);
        frame.render_widget(empty_msg, inner);
        return;
    }

    let visible_height = inner.height as usize;
    // Calculate scroll offset to keep selected item visible
    let scroll_offset = if app.selected_collection >= visible_height {
        app.selected_collection.saturating_sub(visible_height - 1)
    } else {
        0
    };

    let items: Vec<ListItem> = collections
        .iter()
        .enumerate()
        .skip(scroll_offset)
        .take(visible_height)
        .map(|(idx, coll)| {
            let is_selected = idx == app.selected_collection && focused;
            let style = if is_selected {
                Style::default().fg(theme.bg).bg(theme.accent)
            } else if idx == app.selected_collection {
                Style::default().fg(theme.accent)
            } else {
                Style::default().fg(theme.fg)
            };

            let marker = if idx == app.selected_collection {
                "▶ "
            } else {
                "  "
            };
            let text = format!("{}{} ({})", marker, coll.name, coll.doc_count_display());
            ListItem::new(text).style(style)
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, inner);
}

fn render_indexes(frame: &mut Frame, area: Rect, app: &App, theme: &Theme, focused: bool) {
    let border_color = if focused {
        theme.accent
    } else {
        theme.secondary
    };

    let block = Block::default()
        .title(" Indexes ")
        .title_style(Style::default().fg(theme.secondary))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Get indexes for selected collection
    let indexes = app.get_current_indexes();

    if indexes.is_empty() {
        let empty_msg = Paragraph::new("Nincs index").style(Style::default().fg(theme.muted));
        frame.render_widget(empty_msg, inner);
        return;
    }

    let items: Vec<ListItem> = indexes
        .iter()
        .map(|idx_name| {
            let style = Style::default().fg(theme.muted);
            ListItem::new(format!("  {}", idx_name)).style(style)
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, inner);
}
