//! Modal dialogs - overlays for search, actions, etc.

pub mod actions;
pub mod confirm;
pub mod error;
pub mod export;
pub mod filter;
pub mod help;
pub mod index;
pub mod insert;
pub mod new_collection;
pub mod progress;
pub mod query;
pub mod script;
pub mod search;
pub mod server_info;
pub mod splash;

use crate::theme::Theme;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear};

/// Center a rect within another rect
pub fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::vertical([
        Constraint::Percentage((100 - percent_y) / 2),
        Constraint::Percentage(percent_y),
        Constraint::Percentage((100 - percent_y) / 2),
    ])
    .split(r);

    Layout::horizontal([
        Constraint::Percentage((100 - percent_x) / 2),
        Constraint::Percentage(percent_x),
        Constraint::Percentage((100 - percent_x) / 2),
    ])
    .split(popup_layout[1])[1]
}

/// Common modal rendering helper
pub fn render_modal_frame(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    theme: &Theme,
    width_pct: u16,
    height_pct: u16,
) -> Rect {
    let modal_area = centered_rect(width_pct, height_pct, area);

    // Clear the background
    frame.render_widget(Clear, modal_area);

    // Render the frame
    let block = Block::default()
        .title(format!(" {} ", title))
        .title_style(Style::default().fg(theme.accent).bold())
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.accent))
        .style(Style::default().bg(theme.bg));

    let inner = block.inner(modal_area);
    frame.render_widget(block, modal_area);

    inner
}
