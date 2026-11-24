//! Confirm dialog modal

use super::render_modal_frame;
use crate::i18n::t;
use crate::theme::Theme;
use crate::widgets::choice;
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

/// Confirm dialog state
#[derive(Debug, Clone)]
pub struct ConfirmState {
    pub title: String,
    pub message: String,
    pub confirm_text: String,
    pub cancel_text: String,
    pub selected: ConfirmOption,
    pub on_confirm: ConfirmAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConfirmOption {
    #[default]
    Cancel,
    Confirm,
}

impl ConfirmOption {
    pub fn toggle(&self) -> Self {
        match self {
            ConfirmOption::Cancel => ConfirmOption::Confirm,
            ConfirmOption::Confirm => ConfirmOption::Cancel,
        }
    }
}

/// What action to perform on confirmation
#[derive(Debug, Clone)]
pub enum ConfirmAction {
    DeleteDocument {
        collection: String,
        doc_id: serde_json::Value,
    },
    DeleteCollection {
        name: String,
    },
    DropIndex {
        collection: String,
        index_name: String,
    },
    DropVectorIndex {
        collection: String,
        index_name: String,
    },
    RollbackScript {
        name: String,
        version: u32,
    },
    DisableAutoEmbed {
        collection: String,
    },
    ClearEmbedCache,
    CancelEmbedJob {
        job_id: String,
    },
}

impl Default for ConfirmState {
    fn default() -> Self {
        Self {
            title: t().confirm_title().to_string(),
            message: t().confirm_message().to_string(),
            confirm_text: t().yes().to_string(),
            cancel_text: t().key_cancel().to_string(),
            selected: ConfirmOption::Cancel,
            on_confirm: ConfirmAction::DeleteDocument {
                collection: String::new(),
                doc_id: serde_json::Value::Null,
            },
        }
    }
}

impl ConfirmState {
    pub fn delete_document(collection: String, doc_id: serde_json::Value, preview: &str) -> Self {
        Self {
            title: t().confirm_delete_document_title().to_string(),
            message: t().confirm_delete_document(&collection, &truncate(preview, 100)),
            confirm_text: t().cmd_delete().to_string(),
            cancel_text: t().key_cancel().to_string(),
            selected: ConfirmOption::Cancel,
            on_confirm: ConfirmAction::DeleteDocument { collection, doc_id },
        }
    }

    pub fn delete_collection(name: String, doc_count: usize) -> Self {
        Self {
            title: t().confirm_drop_collection_title().to_string(),
            message: t().confirm_drop_collection(&name, doc_count),
            confirm_text: t().cmd_delete().to_string(),
            cancel_text: t().key_cancel().to_string(),
            selected: ConfirmOption::Cancel,
            on_confirm: ConfirmAction::DeleteCollection { name },
        }
    }

    pub fn drop_index(collection: String, index_name: String) -> Self {
        Self {
            title: t().confirm_drop_index_title().to_string(),
            message: t().confirm_drop_index(&index_name, &collection),
            confirm_text: t().cmd_delete().to_string(),
            cancel_text: t().key_cancel().to_string(),
            selected: ConfirmOption::Cancel,
            on_confirm: ConfirmAction::DropIndex {
                collection,
                index_name,
            },
        }
    }

    pub fn drop_vector_index(collection: String, index_name: String) -> Self {
        Self {
            title: t().confirm_drop_vector_index_title().to_string(),
            message: t().confirm_drop_vector_index(&index_name, &collection),
            confirm_text: t().cmd_delete().to_string(),
            cancel_text: t().key_cancel().to_string(),
            selected: ConfirmOption::Cancel,
            on_confirm: ConfirmAction::DropVectorIndex {
                collection,
                index_name,
            },
        }
    }

    pub fn rollback_script(name: String, version: u32) -> Self {
        Self {
            title: t().confirm_rollback_title().to_string(),
            message: t().confirm_rollback(&name, version),
            confirm_text: t().rollback_button().to_string(),
            cancel_text: t().key_cancel().to_string(),
            selected: ConfirmOption::Cancel,
            on_confirm: ConfirmAction::RollbackScript { name, version },
        }
    }

    pub fn disable_auto_embed(collection: String) -> Self {
        Self {
            title: t().confirm_disable_auto_embed_title().to_string(),
            message: t().confirm_disable_auto_embed(&collection),
            confirm_text: t().disable_button().to_string(),
            cancel_text: t().key_cancel().to_string(),
            selected: ConfirmOption::Cancel,
            on_confirm: ConfirmAction::DisableAutoEmbed { collection },
        }
    }

    pub fn clear_embed_cache() -> Self {
        Self {
            title: t().confirm_clear_cache_title().to_string(),
            message: t().confirm_clear_cache().to_string(),
            confirm_text: t().clear_button().to_string(),
            cancel_text: t().key_cancel().to_string(),
            selected: ConfirmOption::Cancel,
            on_confirm: ConfirmAction::ClearEmbedCache,
        }
    }

    pub fn cancel_embed_job(job_id: String) -> Self {
        Self {
            title: t().confirm_cancel_job_title().to_string(),
            message: t().confirm_cancel_job(&job_id),
            confirm_text: t().cancel_job_button().to_string(),
            cancel_text: t().key_cancel().to_string(),
            selected: ConfirmOption::Cancel,
            on_confirm: ConfirmAction::CancelEmbedJob { job_id },
        }
    }

    pub fn toggle_selection(&mut self) {
        self.selected = self.selected.toggle();
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        // Safe UTF-8 truncation at character boundary
        let truncated: String = s.chars().take(max).collect();
        format!("{}...", truncated)
    }
}

/// Render the confirm dialog
pub fn render(frame: &mut Frame, area: Rect, state: &ConfirmState, theme: &Theme) {
    let inner = render_modal_frame(frame, area, &state.title, theme, 50, 30);

    // Split into message and buttons
    let chunks = Layout::vertical([
        Constraint::Min(3),    // Message
        Constraint::Length(1), // Spacer
        Constraint::Length(1), // Buttons
    ])
    .split(inner);

    // Message
    let msg = Paragraph::new(state.message.clone())
        .style(Style::default().fg(theme.fg))
        .alignment(Alignment::Center);
    frame.render_widget(msg, chunks[0]);

    // Buttons
    let (cancel_style, confirm_style) = match state.selected {
        ConfirmOption::Cancel => (
            Style::default().fg(theme.bg).bg(theme.accent),
            Style::default().fg(theme.muted),
        ),
        ConfirmOption::Confirm => (
            Style::default().fg(theme.muted),
            Style::default().fg(theme.bg).bg(theme.error),
        ),
    };

    let buttons = Line::from(vec![
        Span::raw("  "),
        Span::styled(
            choice(&state.cancel_text, state.selected == ConfirmOption::Cancel),
            cancel_style,
        ),
        Span::raw("    "),
        Span::styled(
            choice(
                &state.confirm_text,
                state.selected == ConfirmOption::Confirm,
            ),
            confirm_style,
        ),
        Span::raw("  "),
    ]);

    let buttons_para = Paragraph::new(buttons).alignment(Alignment::Center);
    frame.render_widget(buttons_para, chunks[2]);
}
