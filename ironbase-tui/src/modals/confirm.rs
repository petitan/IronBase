//! Confirm dialog modal

use super::{centered_rect, render_modal_frame_rows};
use crate::i18n::t;
use crate::theme::Theme;
use crate::widgets::{choice, truncate_width};
use ratatui::buffer::Buffer;
use ratatui::prelude::*;
use ratatui::widgets::{Paragraph, Wrap};

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
            message: t().confirm_delete_document(&collection, &truncate_width(preview, 200)),
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

/// Rows `paragraph` fills when drawn `width` columns wide, counted up to
/// `max_rows` (blank rows at the end do not count: nothing is lost there).
/// Drawn off screen with ratatui's own wrapping, so the dialog's height is
/// what the message really takes (an estimate of the word wrap fell short
/// with runs of spaces)
fn drawn_rows(paragraph: &Paragraph, width: u16, max_rows: u16) -> u16 {
    let area = Rect::new(0, 0, width, max_rows);
    let mut buffer = Buffer::empty(area);
    Widget::render(paragraph, area, &mut buffer);
    (0..max_rows)
        .rev()
        .find(|&y| (0..width).any(|x| buffer[(x, y)].symbol() != " "))
        .map_or(0, |y| y + 1)
}

/// Render the confirm dialog
pub fn render(frame: &mut Frame, area: Rect, state: &ConfirmState, theme: &Theme) {
    // Wrapped: a line wider than the dialog (a long collection name, the
    // document preview) is not cut off
    let msg = Paragraph::new(state.message.clone())
        .style(Style::default().fg(theme.fg))
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: false });

    // As high as the message needs, at least 30% of the screen (a fixed
    // height cut a long message off): borders 2, spacer 1, buttons 1
    const WIDTH_PCT: u16 = 50;
    const CHROME: u16 = 4;
    const MESSAGE_MIN_ROWS: u16 = 3;
    let inner_width = centered_rect(WIDTH_PCT, 100, area).width.saturating_sub(2);
    // The message gets at least its layout's minimum (3 rows): fewer left
    // the buttons out of a short dialog
    let needed = drawn_rows(&msg, inner_width, area.height)
        .max(MESSAGE_MIN_ROWS)
        .saturating_add(CHROME);
    let height = needed.max(centered_rect(100, 30, area).height);
    let inner = render_modal_frame_rows(frame, area, &state.title, theme, WIDTH_PCT, height);

    // Split into message and buttons
    let chunks = Layout::vertical([
        Constraint::Min(MESSAGE_MIN_ROWS), // Message
        Constraint::Length(1),             // Spacer
        Constraint::Length(1),             // Buttons
    ])
    .split(inner);

    // Message
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

#[cfg(test)]
mod wrap_tests {
    use super::*;

    /// The dialog fits its message: runs of spaces, a leading space, wide
    /// characters (cases an estimate of the word wrap got wrong), and a
    /// huge screen (the height arithmetic overflowed `u16`)
    #[test]
    fn test_dialog_fits_its_message() {
        for (message, width, height) in [
            (" abc".to_string(), 60, 20),
            ("a  a x  bba bb ".repeat(20), 60, 30),
            ("日本語日 ".repeat(30), 60, 30),
            ("x".to_string(), 2000, 3000),
        ] {
            let state = ConfirmState {
                message: format!("{message} ZZEND"),
                ..ConfirmState::default()
            };
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|f| render(f, f.area(), &state, &crate::theme::Theme::dark()))
                .unwrap();
            let buffer = terminal.backend().buffer();
            let screen: String = (0..height)
                .flat_map(|y| (0..width).map(move |x| (x, y)))
                .map(|xy| buffer[xy].symbol().to_string())
                .collect();
            assert!(screen.contains("ZZEND"), "{message:?}");
            // The buttons too (a short dialog left them out)
            assert!(screen.contains(&state.cancel_text), "{message:?}");
        }
    }
}
