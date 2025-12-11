//! Actions modal - flat action menu (Lazygit style)

use super::render_modal_frame;
use crate::app::App;
use crate::theme::Theme;
use ratatui::prelude::*;
use ratatui::widgets::{List, ListItem};

/// Action item definition
struct ActionItem {
    key: char,
    label: &'static str,
    enabled: bool,
}

/// Render the actions modal
pub fn render(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let inner = render_modal_frame(frame, area, "Akciok", theme, 45, 55);

    let actions = get_context_actions(app);

    let items: Vec<ListItem> = actions
        .iter()
        .map(|action| {
            let style = if action.enabled {
                Style::default().fg(theme.fg)
            } else {
                Style::default().fg(theme.muted)
            };

            let line = Line::from(vec![
                Span::styled(
                    format!("[{}]", action.key),
                    if action.enabled {
                        Style::default().fg(theme.accent).bold()
                    } else {
                        Style::default().fg(theme.muted)
                    },
                ),
                Span::raw(" "),
                Span::styled(action.label, style),
            ]);

            ListItem::new(line)
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, inner);
}

/// Get available actions based on current context
fn get_context_actions(app: &App) -> Vec<ActionItem> {
    let has_collection = app.has_collection();
    let has_document = app.get_selected_document().is_some();

    vec![
        ActionItem {
            key: 'i',
            label: "Dokumentum beszurasa",
            enabled: has_collection,
        },
        ActionItem {
            key: 'e',
            label: "Dokumentum szerkesztese",
            enabled: has_document,
        },
        ActionItem {
            key: 'd',
            label: "Dokumentum torlese",
            enabled: has_document,
        },
        ActionItem {
            key: 'y',
            label: "Dokumentum masolasa",
            enabled: has_document,
        },
        ActionItem {
            key: 'c',
            label: "Kollekció letrehozasa",
            enabled: true,
        },
        ActionItem {
            key: 'D',
            label: "Kollekció torlese",
            enabled: has_collection,
        },
        ActionItem {
            key: 'x',
            label: "Index kezeles",
            enabled: has_collection,
        },
        ActionItem {
            key: 'r',
            label: "Query (JSON)",
            enabled: has_collection,
        },
        ActionItem {
            key: 'f',
            label: "Szuro (vizualis)",
            enabled: has_collection,
        },
        ActionItem {
            key: 'o',
            label: "Kollekció exportalasa",
            enabled: has_collection,
        },
        ActionItem {
            key: 's',
            label: "IronRhai Scriptek",
            enabled: true,
        },
        ActionItem {
            key: 'b',
            label: "Adatbazis megnyitas/letrehozas",
            enabled: true,
        },
    ]
}
