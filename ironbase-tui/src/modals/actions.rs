//! Actions modal - flat action menu (Lazygit style)

use super::render_modal_frame;
use crate::app::App;
use crate::i18n::t;
use crate::keymap::{Action, ACTIONS_MENU};
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
    let inner = render_modal_frame(frame, area, t().actions_title(), theme, 45, 55);

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

/// The actions menu, from its key map (`keymap::ACTIONS_MENU`): an action
/// that needs a collection or a document is shown dimmed without one
fn get_context_actions(app: &App) -> Vec<ActionItem> {
    let has_collection = app.has_collection();
    let has_document = app.get_selected_document().is_some();

    ACTIONS_MENU
        .iter()
        .map(|&(key, action)| ActionItem {
            key,
            label: t().action_label(action),
            enabled: match action {
                Action::EditDocument | Action::DeleteDocument | Action::CopyDocument => {
                    has_document
                }
                Action::NewCollection | Action::Scripts | Action::OpenDatabase => true,
                _ => has_collection,
            },
        })
        .collect()
}
